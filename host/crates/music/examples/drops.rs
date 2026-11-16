// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where FM: engines drops to the beeper, and where its output jumps, over a game's run: the
//! diagnostic for "it clicks here and there". Each game runs as `music-corpus` runs it (its key
//! script, its learned entries as the personal pack learns them), through the mixer in FM: engines.
//!
//!     cargo run --release -p music --example drops -- <game file>... [--seconds N] [--show A-B]
//!         [--wav FILE] [--cache DIR]
//!
//! Per game:
//! - a summary: frames with sound, how many played as FM and how many the effects follower played
//!   as FM (`fx`), the switches to the beeper, and the drops (back to FM within 2 s);
//! - each audible drop (a beeper stretch with sound in it), with B's decision and the frame's
//!   edges (count, OUT addresses, gaps, first and last T-state) for each of its frames: a rejected
//!   frame of the engine's own (its OUT, decision c0) is the check's doing; another OUT is other
//!   code's sound (an effect);
//! - jumps: frames whose largest second difference of the output is over three times the run's
//!   typical FM frame (90th percentile), with the decision and its note events. Note attacks at
//!   large pitch steps show here too; a jump at an FM/beeper switch is a click;
//! - with `--show A-B`, frames A to B: FM, `fx` (the follower) or beeper (`bp`, with the gate's
//!   reason: two-voice, busy, irregular, B-rejected), the decision, the output's range and level,
//!   the edges;
//! - with `--wav FILE`, the mixer's output (one game per run).
//!
//! Frame numbers are 20 ms frames from the start of the run (the snapshot, or the tape's
//! autostart). Learned entries are cached in `--cache` (default `build/drops-cache`).

use fmzx_pack::personal::{converted_ext, learn_music};
use fmzx_pack::tapeload;
use music::corpus::{inputs, load, parse_keys, wav_stem};
use music::{capture, hz_of_period_q8, repo, sample_rate, wav, RATE};
use std::path::{Path, PathBuf};
use zx_sys::{Beeper, SndEngines, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_FX_B_REJECTED, SND_FX_BUSY,
    SND_FX_IRREGULAR, SND_FX_PLAY, SND_FX_TWO_VOICE, SND_MODE_FM_B};

/// A drop that comes back to FM within this many frames (2 s) is a drop inside the music.
const DROP_MAX: usize = 100;

/// The game's learned entries, as the personal pack learns them (None: none learned).
fn learned(path: &Path, cache: &Path) -> Option<Vec<u8>> {
    let l = tapeload::prepare(path).ok()?;
    let file = format!("{}.{}", wav_stem(path), converted_ext(&l, path));
    let _ = std::fs::create_dir_all(cache);
    learn_music(&file, &l.data, &path.with_extension("keys"), &Default::default(), cache).list
}

/// A decision in one line: confident, engine, style, the voices' pitches, events, drum.
fn decision(d: &SndFrame) -> String {
    let hz: Vec<String> = d
        .tone
        .iter()
        .map(|v| if v.period_q8 == 0 { "-".into() } else { format!("{:.0}", hz_of_period_q8(v.period_q8)) })
        .collect();
    format!("c{} e{} st{} [{}] ev{} dr{}", d.confident, d.engine, d.style, hz.join(" "), d.n_ev, d.drum)
}

/// A frame's edges in one line: count, each OUT address with its count, the gaps' range, the
/// first and last T-state.
fn edges(e: &[zx_sys::zx_edge]) -> String {
    let mut pcs: Vec<(u16, usize)> = Vec::new();
    for x in e {
        match pcs.iter_mut().find(|p| p.0 == x.pc) {
            Some(p) => p.1 += 1,
            None => pcs.push((x.pc, 1)),
        }
    }
    let gaps: Vec<u32> = e.windows(2).map(|w| w[1].tstate - w[0].tstate).collect();
    let pcs: Vec<String> = pcs.iter().map(|(pc, n)| format!("{pc:04X}x{n}")).collect();
    format!(
        "n{} [{}] gaps {}..{} t {}..{}",
        e.len(),
        pcs.join(" "),
        gaps.iter().min().unwrap_or(&0),
        gaps.iter().max().unwrap_or(&0),
        e.first().map_or(0, |x| x.tstate),
        e.last().map_or(0, |x| x.tstate)
    )
}

/// The largest second difference of a frame's output, and the sample it is at.
fn jump(o: &[i16]) -> (i32, usize) {
    o.windows(3)
        .enumerate()
        .map(|(i, w)| ((w[2] as i32 - 2 * w[1] as i32 + w[0] as i32).abs(), i))
        .max()
        .unwrap_or((0, 0))
}

struct Run {
    on: Vec<bool>,           // the mixer plays FM (frame k)
    gate: Vec<u8>,           // the effects follower's gate (SND_FX_*) for frame k
    sound: Vec<bool>,        // the speaker level changes in frame k
    dec: Vec<SndFrame>,      // B's decision about frame k
    edges: Vec<String>,      // frame k's edges, described
    out: Vec<Vec<i16>>,      // the mixer's output for frame k
}

fn run(path: &Path, frames: usize, list: Option<&[u8]>) -> Result<Run, String> {
    let mut m = load(path)?;
    let steps = std::fs::read_to_string(path.with_extension("keys")).ok().and_then(|t| parse_keys(&t).ok()).unwrap_or_default();
    let mut mix = SndMixer::new(RATE.0, RATE.1);
    let mut b = SndEngines::new();
    if let Some(l) = list {
        mix.set_learned(l).map_err(|e| format!("learned list: {e:?}"))?;
        b = SndEngines::with_learned(l).map_err(|e| format!("learned list: {e:?}"))?;
    }
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let delay = SND_DELAY_FRAMES as usize;
    let mut r = Run { on: Vec::new(), sound: Vec::new(), gate: Vec::new(), dec: Vec::new(), edges: Vec::new(), out: Vec::new() };
    // the decisions and the mixer's output come SND_DELAY_FRAMES after their frame: run that
    // much further
    for k in 0..frames + delay {
        if k < frames {
            let (rows, kemp) = inputs(&steps, k as u64 * 20);
            m.set_keys(rows);
            m.set_kempston(kemp);
        }
        let f = capture(&mut m, &mut beeper, 1).pop().expect("one frame");
        let mut level = f.start_level & 2;
        r.sound.push(f.edges.iter().any(|e| std::mem::replace(&mut level, e.level & 2) != e.level & 2));
        r.edges.push(edges(&f.edges));
        let o = mix.frame(SND_MODE_FM_B, Some(&m), &f.edges, f.start_level, &f.beeper);
        if k >= delay {
            r.out.push(o);
            r.on.push(mix.fm_on());
            r.gate.push(mix.fx_gate());
        }
        r.dec.push(b.frame(&m, &f.edges, f.start_level));
    }
    r.dec.drain(..delay);
    r.sound.truncate(frames);
    r.edges.truncate(frames);
    Ok(r)
}

impl Run {
    /// Frame k's playback: `FM` (the mixer's music), `fx` (the follower's effects as FM), or `bp`
    /// (the beeper), with the gate's reason when a gate kept the beeper.
    fn label(&self, k: usize) -> String {
        if self.on[k] {
            return "FM".into();
        }
        match self.gate[k] {
            SND_FX_PLAY if self.sound[k] => "fx".into(),   // a silent frame has nothing to follow
            SND_FX_TWO_VOICE => "bp two-voice".into(),
            SND_FX_BUSY => "bp busy".into(),
            SND_FX_IRREGULAR => "bp irregular".into(),
            SND_FX_B_REJECTED => "bp B-rejected".into(),
            _ => "bp".into(),
        }
    }
}

fn report(path: &Path, r: &Run, show: Option<(usize, usize)>, learned: bool) {
    let frames = r.sound.len();
    let fm = (0..frames).filter(|&k| r.sound[k] && r.on[k]).count();
    // a beeper frame the follower played as FM effects (not B's FM)
    let fx = (0..frames).filter(|&k| r.sound[k] && r.gate[k] == SND_FX_PLAY && !r.on[k]).count();
    let sounding = r.sound.iter().filter(|&&s| s).count();
    // the beeper stretches after FM, and those back to FM within DROP_MAX frames
    let (mut switches, mut drops) = (0, Vec::new());
    let mut k = 1;
    while k < frames {
        if r.on[k - 1] && !r.on[k] {
            switches += 1;
            let s = k;
            while k < frames && !r.on[k] {
                k += 1;
            }
            if k < frames && k - s <= DROP_MAX {
                drops.push((s, k - s));
            }
        } else {
            k += 1;
        }
    }
    let audible: Vec<(usize, usize)> = drops.iter().copied().filter(|&(s, n)| (s..s + n).any(|k| r.sound[k])).collect();
    println!(
        "== {}: learned entries {}; FM for {fm} of {sounding} frames with sound; follower for {fx} of {sounding} frames with sound; {switches} switches to the beeper, \
         {} drops ({} audible)",
        path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        if learned { "yes" } else { "no" },
        drops.len(),
        audible.len()
    );
    let line = |k: usize| format!("f{k:5} {:6.2}s {:13} {:34} | {}", k as f64 / 50.0, r.label(k), decision(&r.dec[k]), r.edges[k]);
    for &(s, n) in &audible {
        println!("  drop at {:.2}s: {n} frames on the beeper", s as f64 / 50.0);
        for k in s.saturating_sub(2)..(s + n + 1).min(frames) {
            println!("    {}", line(k));
        }
    }
    let mut fm_jumps: Vec<i32> = (0..frames).filter(|&k| r.on[k]).map(|k| jump(&r.out[k]).0).collect();
    fm_jumps.sort_unstable();
    let typical = fm_jumps.get(fm_jumps.len() * 9 / 10).copied().unwrap_or(1).max(1);
    let jumps: Vec<usize> = (1..frames).filter(|&k| (r.on[k] || r.on[k - 1]) && jump(&r.out[k]).0 > 3 * typical).collect();
    println!("  jumps (over 3x the typical FM frame's {typical}): {}", jumps.len());
    for &k in &jumps {
        let (v, at) = jump(&r.out[k]);
        let ev: Vec<String> = r.dec[k]
            .events()
            .iter()
            .map(|e| format!("v{}@{} {:.0} Hz", e.voice, e.t, if e.period_q8 == 0 { 0.0 } else { hz_of_period_q8(e.period_q8) }))
            .collect();
        let switch = match (r.on[k - 1], r.on[k]) {
            (true, false) => " FM->beeper",
            (false, true) => " beeper->FM",
            _ => "",
        };
        println!("    {} | {v} at sample {at}{switch} | events {ev:?}", line(k));
    }
    if let Some((a, b)) = show {
        println!("  frames {a} to {b}:");
        for k in a..=b.min(frames - 1) {
            let o = &r.out[k];
            let mean = o.iter().map(|&x| x as i64).sum::<i64>() / o.len().max(1) as i64;
            println!(
                "    {} | out {}..{} mean {mean}",
                line(k),
                o.iter().min().unwrap_or(&0),
                o.iter().max().unwrap_or(&0)
            );
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        let v = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        v
    };
    let seconds: usize = take("--seconds").map_or(40, |s| s.parse().expect("--seconds N"));
    let show = take("--show").map(|s| {
        let (a, b) = s.split_once('-').expect("--show A-B");
        (a.parse().expect("--show A-B"), b.parse().expect("--show A-B"))
    });
    let wav_out = take("--wav").map(PathBuf::from);
    let cache = take("--cache").map_or_else(|| repo().join("build/drops-cache"), PathBuf::from);
    if args.is_empty() {
        eprintln!("usage: drops <game file>... [--seconds N] [--show A-B] [--wav FILE] [--cache DIR]");
        std::process::exit(2);
    }
    for a in &args {
        let path = Path::new(a);
        let list = learned(path, &cache);
        match run(path, seconds * 50, list.as_deref()) {
            Ok(r) => {
                report(path, &r, show, list.is_some());
                if let Some(w) = &wav_out {
                    let all: Vec<i16> = r.out.iter().flatten().copied().collect();
                    if let Err(e) = wav::write(w, &all, sample_rate().round() as u32) {
                        eprintln!("{}: {e}", w.display());
                    }
                }
            }
            Err(e) => println!("== {a}: {e}"),
        }
    }
}
