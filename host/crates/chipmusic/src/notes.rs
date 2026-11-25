// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A SID capture's notes, on the Spectrum's clock.
//!
//! - The timeline starts at the first row where a voice sounds (gate on, a waveform, volume).
//! - Pitch: f = register * clock / 2^24; a note's `period_t` is 3,500,000 / f T-states.
//! - A tonal note starts when a voice's gate rises (a rise within one call while the gate was
//!   already on is a retrigger: the note ends and a new one starts), or, gated, when the pitch
//!   moves more than 0.6 semitone from the note's (legato); it ends when the gate falls, the
//!   test bit is set, the waveform is none or noise, the volume is 0, or the pitch leaves
//!   16-8000 Hz.
//! - Noise: a voice set to noise with its gate on is a drum hit on voice 3, ending when the
//!   noise does or after 0.06 s, at least 2,048 T before the next hit; hits of several voices
//!   within a quarter frame are one. For 0.16 s after a hit, the voice is tonal again only once
//!   its pitch holds for two rows (a drum's falling tail is part of the drum).
//! - Arpeggios: on a voice, a run of at least 3 back-to-back notes each at most 2.5 PAL frames
//!   long, cycling 2-4 pitches (one recurring), is its root: one note at the run's lowest
//!   pitch, the run's length.
//! - Level (1-255): 255 * sqrt(volume / 15 * (0.35 + 0.65 * sustain / 15)); a drum's with the
//!   sustain at least half.
//! - Voices: the tonal SID voices in order of their duration-weighted mean pitch: 0 the highest
//!   (the lead), 1 the lowest (the bass), 2 the other (the pad); 3 the drum.
//! - Loop: the rows (registers and the calls' gate and noise flags) repeating exactly from a row
//!   to the end, at least twice, a pass at least a second long with a note or hit starting in
//!   it (of those, the one starting first, then the shortest): `intro_t` is that row's time and
//!   `loop_t` the pass's, and the notes are those of the intro and one pass. Else `loop_t` is 0
//!   and `intro_t` the end of the last note (the capture's end, unless the tune falls silent).
//! - Times are rounded to 64 T. A frame (69,888 T on its section's grid: the intro's from 0, the
//!   loop's from `intro_t`) holds at most [`EVENTS_PER_FRAME`] events, counted as the pack
//!   counts a take's (a note on per note; a note off at its end unless the voice's next note
//!   starts there or the note reaches its section's end; a drum's on and off): past that, the
//!   latest drum note with an event in the frame is dropped, then the latest pad, bass and
//!   lead note.

use crate::c64::{Capture, Row, SID_REGS};
use std::collections::{BTreeMap, HashSet};

/// The Spectrum's clock (T-states a second).
pub const SPECTRUM_HZ: u64 = 3_500_000;
/// T-states of a Spectrum frame.
pub const FRAME_T: u64 = 69_888;
/// The events a frame holds on the FM-1.
pub const EVENTS_PER_FRAME: usize = 8;
/// The drum's voice.
pub const DRUM: u8 = 3;
/// The time unit of score events (T).
const UNIT: u64 = 64;
/// A drum's off at least this long before the next hit (T).
const DRUM_GAP: u64 = 2048;
/// The longest period a note may have (24 bits).
const PERIOD_MAX: f64 = 16_777_215.0;

/// A note: its voice (0-2 tonal, 3 the drum), start and end in T since the timeline's start,
/// its period in T (0 for the drum) and level (1-255).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub voice: u8,
    pub start_t: u64,
    pub end_t: u64,
    pub period_t: u32,
    pub level: u8,
}

/// A capture's notes and how they were made.
#[derive(Clone, Debug, Default)]
pub struct Notes {
    pub notes: Vec<Note>,
    pub intro_t: u64,
    /// 0: no loop.
    pub loop_t: u64,
    /// The SID voice (0-2) each tonal voice comes from.
    pub sid_voice: [Option<u8>; 3],
    /// Tonal notes before the arpeggio folding.
    pub raw_notes: usize,
    /// Arpeggio runs folded.
    pub arp_runs: usize,
    /// Notes dropped for frames over [`EVENTS_PER_FRAME`] events: drum, tonal.
    pub dropped: (usize, usize),
}

/// A note in C64 cycles, before the timeline is settled.
#[derive(Clone, Copy, Debug)]
struct Raw {
    voice: u8,
    start: u64,
    end: u64,
    hz: f64,
    level: u8,
}

fn midi(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

fn level(vol: u8, sus: f64) -> u8 {
    (255.0 * (f64::from(vol) / 15.0 * sus).sqrt()).round().clamp(1.0, 255.0) as u8
}

/// A voice sounds: the volume up and a voice gated with a waveform, not in test.
fn audible(r: &Row) -> bool {
    r.regs[24] & 15 != 0
        && (0..3).any(|v| {
            let ctrl = r.regs[7 * v + 4];
            ctrl & 1 != 0 && ctrl & 8 == 0 && ctrl >> 4 != 0
        })
}

type Key = ([u8; SID_REGS], u8, u8);

fn key(r: &Row) -> Key {
    (r.regs, r.rise, r.noise_on)
}

/// The loop in `rows`: the row `i` and length `l` (rows) of a period the rows keep from `i` to
/// the end, at least twice, at least a second long and with a gate rising or a noise starting
/// in it: of those, the one starting first, and the shortest of those.
fn find_loop(rows: &[Row], clock: f64) -> Option<(usize, usize)> {
    let k: Vec<Key> = rows.iter().map(key).collect();
    let n = k.len();
    let mut best: Option<(usize, usize)> = None;
    for l in 1..=n / 2 {
        let mut i = n - l;
        while i > 0 && k[i - 1] == k[i - 1 + l] {
            i -= 1;
        }
        if n - i < 2 * l || best.is_some_and(|b| i >= b.0) || ((rows[i + l].t - rows[i].t) as f64) < clock {
            continue;
        }
        if rows[i..i + l].iter().any(|r| r.rise != 0 || r.noise_on != 0) {
            best = Some((i, l));
        }
    }
    best
}

/// The tonal notes and drum hits of `rows` (times in cycles); `end` the capture's end.
fn extract(rows: &[Row], end: u64, clock: f64) -> (Vec<Raw>, Vec<Raw>) {
    let frame = clock / 50.125;
    let (tail, hit_len) = ((0.16 * clock) as u64, (0.06 * clock) as u64);
    let mut notes = Vec::new();
    let mut hits: Vec<Raw> = Vec::new();
    for v in 0..3 {
        let b = 7 * v;
        let bit = 1u8 << v;
        let mut cur: Option<(u64, f64, f64, u8)> = None; // start, hz, midi, level
        let mut prev_mf = 0.0;
        let mut last_hit: Option<u64> = None;
        let mut open_hit: Option<usize> = None;
        let mut stable_rows = 0;
        let (mut prev_gate, mut prev_noise) = (false, false);
        for r in rows {
            let t = r.t;
            let vol = r.regs[24] & 15;
            let ctrl = r.regs[b + 4];
            let (gate, test, wave) = (ctrl & 1 != 0, ctrl & 8 != 0, ctrl >> 4);
            let noise = wave & 8 != 0;
            let freq = u32::from(r.regs[b]) | u32::from(r.regs[b + 1]) << 8;
            let hz = f64::from(freq) * clock / 16_777_216.0;
            let mf = if hz > 0.0 { midi(hz) } else { 0.0 };
            // a rise or a noise start between the rows counts as one within the call
            let rise = r.rise & bit != 0 || (gate && !prev_gate);
            let noise_on = r.noise_on & bit != 0 || (gate && noise && !prev_noise);
            let sus = f64::from(r.regs[b + 6] >> 4) / 15.0;
            // the drum
            if let Some(k) = open_hit {
                if !(gate && noise) || t - hits[k].start >= hit_len || rise {
                    hits[k].end = t;
                    open_hit = None;
                }
            }
            if noise_on || (noise && gate && rise) {
                last_hit = Some(t);
                if vol > 0 {
                    if let Some(k) = open_hit.take() {
                        hits[k].end = t;
                    }
                    hits.push(Raw { voice: DRUM, start: t, end: t + hit_len, hz: 0.0, level: level(vol, sus.max(0.5)) });
                    open_hit = Some(hits.len() - 1);
                }
            }
            stable_rows = if (mf - prev_mf).abs() <= 0.6 { stable_rows + 1 } else { 0 };
            if rise {
                stable_rows = 0;
            }
            let in_tail = last_hit.is_some_and(|h| t - h < tail) && !(rise && !noise);
            if rise && !noise {
                last_hit = None;
            }
            let tonal = gate
                && !test
                && !noise
                && wave != 0
                && vol > 0
                && (16.0..=8000.0).contains(&hz)
                && (!in_tail || stable_rows >= 1);
            let lvl = level(vol, 0.35 + 0.65 * sus);
            match cur {
                Some((start, nhz, nmf, nlvl)) if !tonal || rise || (mf - nmf).abs() > 0.6 => {
                    notes.push(Raw { voice: v as u8, start, end: t, hz: nhz, level: nlvl });
                    cur = tonal.then_some((t, hz, mf, lvl));
                }
                None if tonal => cur = Some((t, hz, mf, lvl)),
                _ => {}
            }
            prev_mf = mf;
            prev_gate = gate;
            prev_noise = gate && noise;
        }
        if let Some((start, hz, _, level)) = cur {
            notes.push(Raw { voice: v as u8, start, end, hz, level });
        }
        if let Some(k) = open_hit {
            hits[k].end = hits[k].end.min(end);
        }
    }
    // hits of several voices at once are one
    hits.sort_by_key(|h| h.start);
    let mut merged: Vec<Raw> = Vec::new();
    for h in hits {
        match merged.last_mut() {
            Some(m) if (h.start - m.start) as f64 * 4.0 < frame => {
                m.level = m.level.max(h.level);
                m.end = m.end.max(h.end);
            }
            _ => merged.push(h),
        }
    }
    (notes, merged)
}

/// Folds each voice's arpeggio runs into their roots (see the module docs). Returns the runs.
fn fold(notes: &mut Vec<Raw>, clock: f64) -> usize {
    let step_max = (2.5 * clock / 50.125) as u64;
    notes.sort_by_key(|n| (n.voice, n.start));
    let mut out = Vec::with_capacity(notes.len());
    let mut runs = 0;
    let mut i = 0;
    let short = |n: &Raw| n.end - n.start <= step_max;
    while i < notes.len() {
        let mut j = i;
        let mut set = vec![midi(notes[i].hz).round() as i32];
        while j + 1 < notes.len()
            && notes[j + 1].voice == notes[i].voice
            && notes[j + 1].start == notes[j].end
            && short(&notes[j])
            && short(&notes[j + 1])
        {
            let m = midi(notes[j + 1].hz).round() as i32;
            if !set.contains(&m) {
                if set.len() == 4 {
                    break;
                }
                set.push(m);
            }
            j += 1;
        }
        let len = j - i + 1;
        if len >= 3 && set.len() >= 2 && len > set.len() {
            let root = notes[i..=j].iter().map(|n| n.hz).fold(f64::INFINITY, f64::min);
            out.push(Raw { end: notes[j].end, hz: root, ..notes[i] });
            runs += 1;
            i = j + 1;
        } else {
            out.push(notes[i]);
            i += 1;
        }
    }
    *notes = out;
    runs
}

/// A piece of a note in a section (see [`fit_frames`]): section, start and end in units of the
/// section, voice, whether it reaches the section's end.
#[derive(Clone, Copy)]
struct Piece {
    note: usize,
    sec: usize,
    start: u64,
    end: u64,
    voice: u8,
    cut: bool,
}

fn pieces(notes: &[Note], intro_t: u64, loop_t: u64) -> Vec<Piece> {
    let unit = |t: u64| t.saturating_add(UNIT / 2) / UNIT;
    let mut out = Vec::new();
    for (sec, (from, len)) in [(0, intro_t), (intro_t, loop_t)].into_iter().enumerate() {
        if len == 0 {
            continue;
        }
        let (to, lim) = (from + len, len.div_ceil(UNIT));
        for (k, n) in notes.iter().enumerate() {
            if n.end_t <= from || n.start_t >= to || (sec == 1 && n.voice == DRUM && n.start_t < from) {
                continue;
            }
            let start = unit(n.start_t.max(from) - from);
            let end = unit(n.end_t.min(to) - from);
            let cut = n.end_t >= to || end >= lim;
            if start >= lim || (!cut && end <= start) {
                continue;
            }
            out.push(Piece { note: k, sec, start, end: end.min(lim), voice: n.voice, cut });
        }
    }
    out
}

/// The frames (section, frame) each piece has events in.
fn piece_events(ps: &[Piece]) -> Vec<(usize, Vec<u64>)> {
    let frame_units = FRAME_T / UNIT;
    let starts: HashSet<(usize, u8, u64)> = ps.iter().map(|p| (p.sec, p.voice, p.start)).collect();
    ps.iter()
        .map(|p| {
            let mut f = vec![p.start / frame_units];
            let off = !p.cut && (p.voice == DRUM || !starts.contains(&(p.sec, p.voice, p.end)));
            if off {
                f.push(p.end / frame_units);
            }
            (p.sec, f)
        })
        .collect()
}

/// Drops notes until no frame holds more than [`EVENTS_PER_FRAME`] events (see the module
/// docs). Returns the drum and tonal notes dropped.
fn fit_frames(notes: &mut Vec<Note>, intro_t: u64, loop_t: u64) -> (usize, usize) {
    let mut dropped = (0, 0);
    loop {
        let ps = pieces(notes, intro_t, loop_t);
        let ev = piece_events(&ps);
        let mut per: BTreeMap<(usize, u64), usize> = BTreeMap::new();
        for (sec, fs) in &ev {
            for f in fs {
                *per.entry((*sec, *f)).or_default() += 1;
            }
        }
        let Some((&(sec, f), _)) = per.iter().find(|(_, &n)| n > EVENTS_PER_FRAME) else { return dropped };
        let there: Vec<usize> = (0..ps.len()).filter(|&k| ev[k].0 == sec && ev[k].1.contains(&f)).collect();
        let pick = [DRUM, 2, 1, 0]
            .iter()
            .find_map(|&v| there.iter().filter(|&&k| ps[k].voice == v).max_by_key(|&&k| notes[ps[k].note].start_t))
            .map(|&k| ps[k].note);
        let Some(k) = pick else { return dropped };
        if notes[k].voice == DRUM {
            dropped.0 += 1;
        } else {
            dropped.1 += 1;
        }
        notes.remove(k);
    }
}

/// The notes of a capture (see the module docs).
pub fn convert(cap: &Capture) -> Notes {
    let clock = f64::from(cap.clock_hz);
    let Some(first) = cap.rows.iter().position(audible) else { return Notes::default() };
    let t0 = cap.rows[first].t;
    let rows: Vec<Row> = cap.rows[first..].iter().map(|r| Row { t: r.t - t0, ..r.clone() }).collect();
    let end = cap.end.saturating_sub(t0).max(rows.last().map_or(0, |r| r.t + 1));
    let (mut raw, hits) = extract(&rows, end, clock);
    let raw_notes = raw.len();
    let arp_runs = fold(&mut raw, clock);
    // the timeline: a loop, or the capture, or up to a silent end
    let (mut cut, mut looped) = (end, None);
    if let Some((i, l)) = find_loop(&rows, clock) {
        let (a, b) = (rows[i].t, rows[i + l].t);
        if raw.iter().chain(&hits).any(|n| n.start >= a && n.start < b) {
            looped = Some((a, b));
            cut = b;
        }
    }
    if looped.is_none() {
        cut = raw.iter().chain(&hits).map(|n| n.end).max().unwrap_or(0).min(end);
    }
    let to_t = |c: u64| ((c as f64 * SPECTRUM_HZ as f64 / clock / UNIT as f64).round() as u64) * UNIT;
    // the voices by mean pitch: lead, bass, pad
    let mut stats: Vec<(u8, f64)> = (0..3u8)
        .filter_map(|v| {
            let ns: Vec<&Raw> = raw.iter().filter(|n| n.voice == v && n.start < cut).collect();
            let dur: f64 = ns.iter().map(|n| (n.end - n.start) as f64).sum();
            (dur > 0.0).then(|| (v, ns.iter().map(|n| midi(n.hz) * (n.end - n.start) as f64).sum::<f64>() / dur))
        })
        .collect();
    stats.sort_by(|a, b| b.1.total_cmp(&a.1));
    let order: Vec<u8> = match stats.len() {
        0 => vec![],
        1 => vec![stats[0].0],
        2 => vec![stats[0].0, stats[1].0],
        _ => vec![stats[0].0, stats[2].0, stats[1].0],
    };
    let mut sid_voice = [None; 3];
    for (o, &v) in order.iter().enumerate() {
        sid_voice[o] = Some(v);
    }
    let cut_t = to_t(cut);
    let mut notes: Vec<Note> = raw
        .iter()
        .filter(|n| n.start < cut)
        .filter_map(|n| {
            let voice = order.iter().position(|&v| v == n.voice)? as u8;
            let period = (SPECTRUM_HZ as f64 / n.hz).round().clamp(64.0, PERIOD_MAX) as u32;
            Some(Note { voice, start_t: to_t(n.start), end_t: to_t(n.end.min(cut)), period_t: period, level: n.level })
        })
        .filter(|n| n.end_t > n.start_t)
        .collect();
    // drums: each off at least DRUM_GAP before the next hit
    let hit_t: Vec<(u64, u64, u8)> =
        hits.iter().filter(|h| h.start < cut).map(|h| (to_t(h.start), to_t(h.end.min(cut)), h.level)).collect();
    for (k, &(start, mut end_t, level)) in hit_t.iter().enumerate() {
        if let Some(&(next, _, _)) = hit_t.get(k + 1) {
            if end_t + DRUM_GAP > next {
                end_t = next.saturating_sub(DRUM_GAP).max(start + (next - start) / 2 / UNIT * UNIT);
            }
            if end_t >= next {
                continue;
            }
        }
        if end_t > start {
            notes.push(Note { voice: DRUM, start_t: start, end_t, period_t: 0, level });
        }
    }
    notes.sort_by_key(|n| (n.start_t, n.voice));
    let (intro_t, loop_t) = match looped {
        Some((a, b)) => (to_t(a), to_t(b) - to_t(a)),
        None => (cut_t, 0),
    };
    let dropped = fit_frames(&mut notes, intro_t, loop_t);
    Notes { notes, intro_t, loop_t, sid_voice, raw_notes, arp_runs, dropped }
}
