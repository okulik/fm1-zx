// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A SID or AY capture's notes, on the Spectrum's clock.
//!
//! # SID
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
//! - Level (1-255): 255 * sqrt(volume / 15 * (0.35 + 0.65 * sustain / 15)); a drum's with the
//!   sustain at least half.
//! - Loop: the rows (registers and the calls' gate and noise flags) repeating exactly.
//!
//! # AY
//!
//! - The AY's clock is 1,773,400 Hz; rows are a frame apart. A channel's level is up when its
//!   volume is above 0 or it uses the envelope (level bit 4), whose level counts as full, except
//!   that a shape that runs once and ends at 0 (0-7, 9, 15) is up only for one cycle (256 *
//!   the envelope period AY cycles) from its last restart (a write to register 13).
//! - Pitch: the tone's, f = clock / (16 * period) (a 12-bit period, 0 as 1), when the tone is
//!   enabled in the mixer (register 7, active low); else, with the envelope, the noise off and a
//!   repeating shape, a buzzer's: f = clock / (256 * envelope period) for the sawtooths 8 and
//!   12, clock / (512 * envelope period) for the triangles 10 and 14.
//! - The timeline starts at the first row where a channel's level is up with a pitch or noise.
//! - A tonal note starts when the channel starts to sound (its level up from 0, or its tone or
//!   buzzer starting), is retriggered (the level to 0 and up again within a frame, the volume up
//!   by 3 or more steps, or, with the envelope, the envelope restarted), its tone is enabled in
//!   the mixer, or, sounding, its pitch moves more than 0.6 semitone (legato); it ends when it
//!   stops sounding (a once-only envelope's end between rows ends it there) or the pitch leaves
//!   16-8000 Hz.
//! - Noise: a channel with noise enabled and its level up is a drum hit on voice 3 when the
//!   noise starts fresh (the level up from 0 or retriggered) or is enabled after the last hit's
//!   tail (noise alternating with tone within it is that hit's sound); the hit ends when the
//!   channel falls silent, after 0.06 s, at a new hit, or when a tonal note starts on it; the
//!   gap and merging as for the SID. For 0.16 s after a hit (until a fresh tone) the channel is
//!   tonal only with the noise off and its pitch held for two rows; tone and noise together
//!   longer than that are tonal.
//! - Level (1-255): 255 for the envelope; else 255 * sqrt(10^((volume - 15) * 3 / 20)) (the
//!   AY's volume steps are about 3 dB, the square root as the SID's levels take of a linear
//!   volume), at least 32 (see `ay_level`).
//! - Loop: the rows repeating as heard: registers a channel does not use are taken as 0 (a
//!   silent channel's, a disabled tone's period, the noise period without noise, the envelope
//!   without a channel on it, the mixer's I/O bits), with each row's envelope restart and
//!   retriggers.
//!
//! # Both
//!
//! - Arpeggios: on a voice, a run of at least 3 back-to-back notes each at most 2.5 PAL frames
//!   long, cycling 2-4 pitches (one recurring), is its root: one note at the run's lowest
//!   pitch, the run's length.
//! - Voices: the tonal SID voices or AY channels in order of their duration-weighted mean
//!   pitch: 0 the highest (the lead), 1 the lowest (the bass), 2 the other (the pad); 3 the
//!   drum.
//! - Loop: the rows repeating from a row to the end, at least twice, a pass at least a second
//!   long with a note or hit starting in it (of those, the one starting first, then the
//!   shortest): `intro_t` is that row's time and `loop_t` the pass's, and the notes are those of
//!   the intro and one pass. Else `loop_t` is 0 and `intro_t` the end of the last note (the
//!   capture's end, unless the tune falls silent).
//! - Times are rounded to 64 T. A frame (69,888 T on its section's grid: the intro's from 0, the
//!   loop's from `intro_t`) holds at most [`EVENTS_PER_FRAME`] events, counted as the pack
//!   counts a take's (a note on per note; a note off at its end unless the voice's next note
//!   starts there or the note reaches its section's end; a drum's on and off): past that, the
//!   latest drum note with an event in the frame is dropped, then the latest pad, bass and
//!   lead note.

use crate::ay::{AyCapture, AyRow, AY_HZ, AY_REGS};
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
    /// The chip's voice each tonal voice comes from: the SID voice or AY channel (0-2).
    pub chip_voice: [Option<u8>; 3],
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

/// The loop in rows with keys `k`, times `t` and note starts `starts`: the row `i` and length `l`
/// (rows) of a period the keys keep from `i` to the end, at least twice, at least a second
/// (`clock` time units) long and with a start in it: of those, the one starting first, and the
/// shortest of those. Returns its start and end times.
fn find_loop<K: PartialEq>(k: &[K], t: &[u64], starts: &[bool], clock: f64) -> Option<(u64, u64)> {
    let n = k.len();
    let mut best: Option<(usize, usize)> = None;
    for l in 1..=n / 2 {
        let mut i = n - l;
        while i > 0 && k[i - 1] == k[i - 1 + l] {
            i -= 1;
        }
        if n - i < 2 * l || best.is_some_and(|b| i >= b.0) || ((t[i + l] - t[i]) as f64) < clock {
            continue;
        }
        if starts[i..i + l].iter().any(|&s| s) {
            best = Some((i, l));
        }
    }
    best.map(|(i, l)| (t[i], t[i + l]))
}

/// The tonal notes and drum hits of `rows` (times in cycles); `end` the capture's end.
fn extract(rows: &[Row], end: u64, clock: f64) -> (Vec<Raw>, Vec<Raw>) {
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
    (notes, merge_hits(hits, clock))
}

/// Hits of several voices within a quarter frame are one.
fn merge_hits(mut hits: Vec<Raw>, clock: f64) -> Vec<Raw> {
    let frame = clock / 50.125;
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
    merged
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

/// The notes of a SID capture (see the module docs).
pub fn convert(cap: &Capture) -> Notes {
    let clock = f64::from(cap.clock_hz);
    let Some(first) = cap.rows.iter().position(audible) else { return Notes::default() };
    let t0 = cap.rows[first].t;
    let rows: Vec<Row> = cap.rows[first..].iter().map(|r| Row { t: r.t - t0, ..r.clone() }).collect();
    let end = cap.end.saturating_sub(t0).max(rows.last().map_or(0, |r| r.t + 1));
    let (raw, hits) = extract(&rows, end, clock);
    let keys: Vec<Key> = rows.iter().map(key).collect();
    let t: Vec<u64> = rows.iter().map(|r| r.t).collect();
    let starts: Vec<bool> = rows.iter().map(|r| r.rise != 0 || r.noise_on != 0).collect();
    let looped = find_loop(&keys, &t, &starts, clock);
    settle(raw, hits, looped, end, clock)
}

/// The notes from a capture's tonal notes and drum hits (times in `clock` units a second, from
/// the timeline's start), a loop the rows keep (`a`..`b`) and the capture's end: arpeggios
/// folded, the timeline, the voices ordered, the times on the Spectrum's clock, the drums'
/// gaps and the frames' events (see the module docs).
fn settle(mut raw: Vec<Raw>, hits: Vec<Raw>, rows_loop: Option<(u64, u64)>, end: u64, clock: f64) -> Notes {
    let raw_notes = raw.len();
    let arp_runs = fold(&mut raw, clock);
    // the timeline: a loop, or the capture, or up to a silent end
    let (mut cut, mut looped) = (end, None);
    if let Some((a, b)) = rows_loop {
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
    let mut chip_voice = [None; 3];
    for (o, &v) in order.iter().enumerate() {
        chip_voice[o] = Some(v);
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
    Notes { notes, intro_t, loop_t, chip_voice, raw_notes, arp_runs, dropped }
}

/// The lowest level of an AY note (see [`ay_level`]).
const AY_LEVEL_FLOOR: f64 = 32.0;

/// An AY channel's level (1-255) from its 4-bit volume (1-15). The AY's volume steps are about
/// 3 dB apart (15 full, each step down to 0.708 of the amplitude), so the amplitude is
/// 10^((v - 15) * 3 / 20); the level is 255 * its square root, the compression the SID levels
/// apply to the SID's linear volume, so an AY tune and a SID tune of the same loudness get the
/// same levels. The floor of 32 keeps the quietest volumes (1-3: echoes and fade tails at 23-32)
/// as audible as the quietest SID notes rather than lost under the louder voices.
fn ay_level(vol: u8) -> u8 {
    let amp = 10f64.powf(f64::from(i32::from(vol.min(15)) - 15) * 3.0 / 20.0);
    (255.0 * amp.sqrt()).round().clamp(AY_LEVEL_FLOOR, 255.0) as u8
}

/// An envelope shape that runs once and ends at 0 (0-7, 9 and 15).
fn env_once(shape: u8) -> bool {
    shape < 8 || shape == 9 || shape == 15
}

/// An envelope shape that repeats (8, 10, 12 and 14).
fn env_repeats(shape: u8) -> bool {
    shape >= 8 && shape & 1 == 0
}

/// What an AY channel does in a row.
struct AyChan {
    /// Its level is up: a volume above 0, or the envelope, while it lasts.
    audible: bool,
    level: u8,
    /// Its pitch: the tone's if enabled in the mixer, else a buzzer's, else 0.
    hz: f64,
    tone: bool,
    noise: bool,
    env: bool,
    vol: u8,
    /// When a once-only envelope it uses ends (time units since the timeline's start).
    env_end: Option<i64>,
}

/// Channel `c` in row `r`; `since` the time of the envelope's last restart (time units since
/// the timeline's start), `clock` time units a second.
fn ay_chan(r: &AyRow, c: usize, since: Option<i64>, clock: f64) -> AyChan {
    let g = &r.regs;
    let (tone, noise) = (g[7] >> c & 1 == 0, g[7] >> (c + 3) & 1 == 0);
    let (env, vol) = (g[8 + c] & 0x10 != 0, g[8 + c] & 0x0F);
    let shape = g[13];
    let ep = f64::from((u16::from(g[11]) | u16::from(g[12]) << 8).max(1));
    let env_end = (env && env_once(shape)).then(|| since.map_or(i64::MIN, |s| s + (256.0 * ep * clock / f64::from(AY_HZ)) as i64));
    let audible = if env { env_end.is_none_or(|e| (r.t as i64) < e) } else { vol > 0 };
    let period = f64::from((u16::from(g[2 * c]) | u16::from(g[2 * c + 1]) << 8).max(1));
    let hz = if tone {
        f64::from(AY_HZ) / (16.0 * period)
    } else if env && !noise && env_repeats(shape) {
        f64::from(AY_HZ) / (if shape & 2 != 0 { 512.0 } else { 256.0 } * ep)
    } else {
        0.0
    };
    AyChan { audible, level: if env { 255 } else { ay_level(vol) }, hz, tone, noise, env, vol, env_end }
}

/// The tonal notes and drum hits of AY `rows` (times in `clock` units a second); `since` each
/// row's envelope restart (see [`ay_chan`]), `end` the capture's end.
fn extract_ay(rows: &[AyRow], since: &[Option<i64>], end: u64, clock: f64) -> (Vec<Raw>, Vec<Raw>) {
    let (tail, hit_len) = ((0.16 * clock) as u64, (0.06 * clock) as u64);
    let mut notes = Vec::new();
    let mut hits: Vec<Raw> = Vec::new();
    for c in 0..3 {
        let mut cur: Option<(u64, f64, f64, u8)> = None; // start, hz, midi, level
        let mut prev_mf = 0.0;
        let mut last_hit: Option<u64> = None;
        let mut open_hit: Option<usize> = None;
        let mut stable_rows = 0;
        let (mut prev_on, mut prev_noisy, mut prev_tone, mut prev_env, mut prev_vol) = (false, false, false, false, 0u8);
        let mut prev_env_end: Option<i64> = None;
        for (r, &s) in rows.iter().zip(since) {
            let t = r.t;
            let ch = ay_chan(r, c, s, clock);
            // where a sound that stopped between the rows ended: a once-only envelope's end
            let stop = |start: u64| match prev_env_end {
                Some(e) if !ch.audible => (e.max(start as i64 + 1) as u64).min(t),
                _ => t,
            };
            let on = ch.audible && (ch.hz > 0.0 || ch.noise);
            let noisy = ch.audible && ch.noise;
            let retrig = r.rise >> c & 1 != 0
                || (ch.env && r.env_restart)
                || (!ch.env && prev_on && !prev_env && ch.vol >= prev_vol + 3);
            // a fresh sound (its level up from 0, or retriggered), or the tone switched on
            let fresh = on && (!prev_on || retrig);
            let onset = fresh || (on && ch.tone && !prev_tone);
            let mf = if ch.hz > 0.0 { midi(ch.hz) } else { 0.0 };
            // the drum: a fresh noise, or noise switched on after the last hit's tail (noise
            // alternating with tone within it is that hit's)
            let after_tail = last_hit.is_none_or(|h| t - h >= tail);
            let new_hit = noisy && (fresh || (!prev_noisy && after_tail));
            if let Some(k) = open_hit {
                if !on || t - hits[k].start >= hit_len || new_hit {
                    hits[k].end = stop(hits[k].start).min(hits[k].end);
                    open_hit = None;
                }
            }
            if new_hit {
                last_hit = Some(t);
                hits.push(Raw { voice: DRUM, start: t, end: t + hit_len, hz: 0.0, level: ch.level });
                open_hit = Some(hits.len() - 1);
            }
            // the tones
            stable_rows = if (mf - prev_mf).abs() <= 0.6 { stable_rows + 1 } else { 0 };
            if onset {
                stable_rows = 0;
            }
            let in_tail = last_hit.is_some_and(|h| t - h < tail) && !(fresh && !noisy);
            if fresh && !noisy {
                last_hit = None;
            }
            let tonal = ch.audible && (16.0..=8000.0).contains(&ch.hz) && (!in_tail || (stable_rows >= 1 && !noisy));
            if let Some(k) = open_hit.filter(|_| tonal && !noisy) {
                hits[k].end = t;
                open_hit = None;
            }
            match cur {
                Some((start, nhz, nmf, nlvl)) if !tonal || onset || (mf - nmf).abs() > 0.6 => {
                    notes.push(Raw { voice: c as u8, start, end: stop(start), hz: nhz, level: nlvl });
                    cur = tonal.then_some((t, ch.hz, mf, ch.level));
                }
                None if tonal => cur = Some((t, ch.hz, mf, ch.level)),
                _ => {}
            }
            prev_mf = mf;
            (prev_on, prev_noisy, prev_tone, prev_env, prev_vol) = (on, noisy, ch.tone, ch.env, ch.vol);
            prev_env_end = ch.env_end;
        }
        if let Some((start, hz, _, level)) = cur {
            notes.push(Raw { voice: c as u8, start, end, hz, level });
        }
        if let Some(k) = open_hit {
            hits[k].end = hits[k].end.min(end);
        }
    }
    (notes, merge_hits(hits, clock))
}

type AyKey = ([u8; AY_REGS], bool, u8);

/// A row as heard, for the loop: registers a channel does not use are 0 (a silent channel's
/// level, mixer bits and tone period, a period whose tone is off, the noise period when no
/// channel has noise, the envelope's registers and restart when no channel uses it, the
/// mixer's I/O bits), as players leave them as they were, which differs between passes.
fn ay_key(r: &AyRow) -> AyKey {
    let mut g = r.regs;
    let (mut mixer, mut noise, mut env, mut rise) = (0x3Fu8, false, false, 0u8);
    for c in 0..3 {
        let (tone_on, noise_on) = (g[7] >> c & 1 == 0, g[7] >> (c + 3) & 1 == 0);
        let (uses_env, vol) = (g[8 + c] & 0x10 != 0, g[8 + c] & 0x0F);
        let buzzer = uses_env && env_repeats(g[13]);
        if (vol == 0 && !uses_env) || !(tone_on || noise_on || buzzer) {
            g[8 + c] = 0;
            g[2 * c] = 0;
            g[2 * c + 1] = 0;
            continue;
        }
        if tone_on {
            mixer &= !(1 << c);
        } else {
            g[2 * c] = 0;
            g[2 * c + 1] = 0;
        }
        if noise_on {
            mixer &= !(8 << c);
            noise = true;
        }
        if uses_env {
            g[8 + c] = 0x10;
            env = true;
        }
        rise |= r.rise & 1 << c;
    }
    g[7] = mixer;
    if !noise {
        g[6] = 0;
    }
    if !env {
        g[11..14].fill(0);
    }
    (g, env && r.env_restart, rise)
}

/// The notes of an AY capture (see the module docs).
pub fn convert_ay(cap: &AyCapture) -> Notes {
    let clock = SPECTRUM_HZ as f64;
    // each row's envelope restart, then the timeline from the first row where a channel sounds
    let mut last = None;
    let since_abs: Vec<Option<i64>> = cap
        .rows
        .iter()
        .map(|r| {
            if r.env_restart {
                last = Some(r.t as i64);
            }
            last
        })
        .collect();
    let sounds = |(r, s): (&AyRow, &Option<i64>)| {
        (0..3).any(|c| {
            let ch = ay_chan(r, c, *s, clock);
            ch.audible && (ch.hz > 0.0 || ch.noise)
        })
    };
    let Some(first) = cap.rows.iter().zip(&since_abs).position(sounds) else { return Notes::default() };
    let t0 = cap.rows[first].t;
    let rows: Vec<AyRow> = cap.rows[first..].iter().map(|r| AyRow { t: r.t - t0, ..r.clone() }).collect();
    let since: Vec<Option<i64>> = since_abs[first..].iter().map(|s| s.map(|s| s - t0 as i64)).collect();
    let end = cap.end.saturating_sub(t0).max(rows.last().map_or(0, |r| r.t + 1));
    let (raw, hits) = extract_ay(&rows, &since, end, clock);
    let keys: Vec<AyKey> = rows.iter().map(ay_key).collect();
    let t: Vec<u64> = rows.iter().map(|r| r.t).collect();
    let onsets: HashSet<u64> = raw.iter().chain(&hits).map(|n| n.start).collect();
    let starts: Vec<bool> = t.iter().map(|x| onsets.contains(x)).collect();
    let looped = find_loop(&keys, &t, &starts, clock);
    settle(raw, hits, looped, end, clock)
}
