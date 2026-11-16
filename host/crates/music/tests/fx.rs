// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The effects follower (`core/sound/fx_read.c`, `fx.c`) in the mixer: in FM: engines it re-voices
//! the frames the mixer gives to the delayed beeper as FM (a tone following the pitch, ticks, brief
//! noise), leaves two-voice, busy and sustained irregular frames on the beeper, crossfades every
//! switch, and with effects off (or in another mode) the mixer is what it was without it.
//!
//! Synthetic frames from transition lists, no machine (method B never claims anything, so the
//! mixer plays the delayed path's beeper side throughout).

use fmzx_pack::testprogs::beep_tap;
use music::{capture, castaway, sample_rate, Frame, CPU_HZ, RATE};
use zx_sys::{
    zx_edge, Beeper, Machine, SndMixer, MISREAD_BEEP, SND_DELAY_FRAMES, SND_FX_B_REJECTED, SND_FX_BUSY,
    SND_FX_IRREGULAR, SND_FX_OFF, SND_FX_PLAY, SND_FX_TWO_VOICE, SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B,
    ZX_FRAME_TSTATES,
};

const FT: u64 = ZX_FRAME_TSTATES as u64;
const DELAY: usize = SND_DELAY_FRAMES as usize;

/// A frame's speaker transitions (in-frame T-states, ascending) as edges, the speaker bit
/// alternating from `start_level`.
fn edges(ts: &[u32], start_level: u8) -> Vec<zx_edge> {
    let mut level = start_level & 2;
    ts.iter()
        .map(|&t| {
            level ^= 2;
            zx_edge { tstate: t, level, pc: 0 }
        })
        .collect()
}

/// `n` frames (plus SND_DELAY_FRAMES more, for the delay to play them out) from transitions at
/// absolute T-states `ts` (ascending, from the first frame's start), with their beeper samples.
fn frames(ts: &[u64], n: usize) -> Vec<Frame> {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let (mut level, mut i) = (0u8, 0usize);
    (0..n + DELAY)
        .map(|f| {
            let base = f as u64 * FT;
            let mut tr = Vec::new();
            while i < ts.len() && ts[i] < base + FT {
                tr.push((ts[i] - base) as u32);
                i += 1;
            }
            let e = edges(&tr, level);
            let start_level = level;
            if tr.len() % 2 == 1 {
                level ^= 2;
            }
            let beeper = beeper.render(&e);
            Frame { edges: e, start_level, beeper }
        })
        .collect()
}

/// Per input frame k: the mixer's output for it (the call SND_DELAY_FRAMES later), the delayed
/// beeper it would play without the follower, and the follower's gate.
struct Run {
    out: Vec<Vec<i16>>,
    delayed: Vec<Vec<i16>>,
    gate: Vec<u8>,
}

/// Feeds every frame; `before(mixer, call)` sets the call up and returns its mode.
fn run_calls(mix: &mut SndMixer, frames: &[Frame], mut before: impl FnMut(&mut SndMixer, usize) -> u8) -> Run {
    let beeper: Vec<i16> = frames.iter().flat_map(|f| f.beeper.iter().copied()).collect();
    let mut start = vec![0usize];
    for f in frames {
        start.push(start.last().unwrap() + f.beeper.len());
    }
    let delay = start[DELAY];
    let mut r = Run { out: Vec::new(), delayed: Vec::new(), gate: Vec::new() };
    for (t, f) in frames.iter().enumerate() {
        let mode = before(mix, t);
        let o = mix.frame(mode, None, &f.edges, f.start_level, &f.beeper);
        if t >= DELAY {
            let from = start[t] - delay;
            r.delayed.push(beeper[from..from + o.len()].to_vec());
            r.out.push(o);
            r.gate.push(mix.fx_gate());
        }
    }
    r
}

fn run(mix: &mut SndMixer, frames: &[Frame]) -> Run {
    run_calls(mix, frames, |_, _| SND_MODE_FM_B)
}

fn mixer() -> SndMixer {
    SndMixer::new(RATE.0, RATE.1)
}

/// Transitions of one voice sweeping exponentially from f0 to f1 Hz over `n` frames from `t0`.
fn sweep(t0: u64, n: usize, f0: f64, f1: f64) -> Vec<u64> {
    let len = n as f64 * FT as f64;
    let (lr, secs) = ((f1 / f0).ln(), len / CPU_HZ);
    // the phase (cycles) at T-state x is f0 secs / lr (r^(x / len) - 1); transition k at k / 2
    (0..)
        .map(|k| len * (1.0 + k as f64 / 2.0 * lr / (f0 * secs)).ln() / lr)
        .take_while(|&x| x < len)
        .map(|x| t0 + x.round() as u64)
        .collect()
}

/// The sweep's pitch in the middle of its frame j.
fn sweep_hz(j: usize, n: usize, f0: f64, f1: f64) -> f64 {
    f0 * (f1 / f0).powf((j as f64 + 0.5) / n as f64)
}

/// A steady tone of `hz` for `n` frames from `t0`.
fn tone(t0: u64, n: usize, hz: f64) -> Vec<u64> {
    let half = CPU_HZ / hz / 2.0;
    (0..).map(|k| (k as f64 * half).round() as u64).take_while(|&x| x < n as u64 * FT).map(|x| t0 + x).collect()
}

/// Two interleaved trains of 48 T pulses (periods 2,740 T and 3,690 T) for `n` frames from `t0`:
/// two-voice beeper music. The speaker is high while either pulse is.
fn two_voice(t0: u64, n: usize) -> Vec<u64> {
    let end = n as u64 * FT;
    let mut pulses: Vec<(u64, u64)> = (0..)
        .map(|k| k * 2740)
        .take_while(|&x| x + 48 < end)
        .chain((0..).map(|k| 1000 + k * 3690).take_while(|&x| x + 48 < end))
        .map(|x| (x, x + 48))
        .collect();
    pulses.sort();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (a, b) in pulses {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged.into_iter().flat_map(|(a, b)| [t0 + a, t0 + b]).collect()
}

/// 300 evenly spaced transitions per frame (233 T apart) for `n` frames from `t0`.
fn busy(t0: u64, n: usize) -> Vec<u64> {
    (0..).map(|k| k * 233).take_while(|&x| x < n as u64 * FT).map(|x| t0 + x).collect()
}

/// One irregular pattern of 19 gaps (924-5,296 T, summing to a frame), repeated for `n` frames.
const PATTERN: [u64; 19] =
    [924, 1000, 5296, 5200, 5100, 1100, 5296, 5150, 4900, 950, 5296, 5050, 4800, 5200, 1200, 5100, 4950, 1050, 2326];

fn irregular(t0: u64, n: usize) -> Vec<u64> {
    let mut v = Vec::new();
    let mut t = t0 + 500;
    for _ in 0..n {
        for g in PATTERN {
            v.push(t);
            t += g;
        }
    }
    v
}

/// Largest |second difference| of `s`, with the two samples before it (`prev`'s last two) in
/// front, so a jump at the frame's start counts.
fn d2max(prev: &[i16], s: &[i16]) -> i32 {
    let x: Vec<i32> = prev[prev.len().saturating_sub(2)..].iter().chain(s).map(|&v| v as i32).collect();
    x.windows(3).map(|w| (w[0] - 2 * w[1] + w[2]).abs()).max().unwrap_or(0)
}

/// Pitch (Hz) by autocorrelation: the lag of the highest peak between 1/2000 s and 1/300 s,
/// refined by a parabola through it and its neighbours.
fn pitch(s: &[i16]) -> f64 {
    let sr = sample_rate();
    let x: Vec<f64> = s.iter().map(|&v| v as f64).collect();
    let r = |l: usize| (0..x.len() - l).map(|i| x[i] * x[i + l]).sum::<f64>();
    let (lo, hi) = ((sr / 2000.0).floor() as usize, (sr / 300.0).ceil() as usize);
    let rs: Vec<f64> = (lo - 1..=hi + 1).map(r).collect();
    let b = (1..rs.len() - 1).max_by(|&a, &b| rs[a].total_cmp(&rs[b])).unwrap();
    let den = rs[b - 1] - 2.0 * rs[b] + rs[b + 1];
    let delta = if den != 0.0 { 0.5 * (rs[b - 1] - rs[b + 1]) / den } else { 0.0 };
    sr / ((lo - 1 + b) as f64 + delta)
}

/// How periodic `s` is: its autocorrelation at the pitch's lag over its energy (1 = a pure
/// periodic signal, noise far less).
fn periodicity(s: &[i16]) -> f64 {
    let x: Vec<f64> = s.iter().map(|&v| v as f64).collect();
    let lag = (sample_rate() / pitch(s)).round() as usize;
    let r1: f64 = (0..x.len() - lag).map(|i| x[i] * x[i + lag]).sum();
    r1 / x.iter().map(|v| v * v).sum::<f64>()
}

fn median(mut v: Vec<i32>) -> i32 {
    v.sort();
    v[v.len() / 2]
}

/// Asserts that no output frame's largest second difference exceeds 3x the larger of the
/// follower frames' median (frames `follower`) and that frame's own delayed beeper's: every switch
/// crossfades, with no jump beyond what the signals on either side have themselves.
fn no_jumps(r: &Run, follower: &[usize]) {
    let typ = median(follower.iter().map(|&k| d2max(&r.out[k - 1], &r.out[k])).collect());
    assert!(typ > 100, "the follower frames sound (median second difference {typ})");
    let mut worst = (0.0, 0);
    for k in 1..r.out.len() {
        // `own` makes the bound loose at beeper frames on purpose (a beeper's own steps may be
        // large); the fades_to / fades_from checks are what pin the crossfades, so they must stay.
        let own = d2max(&r.delayed[k - 1], &r.delayed[k]);
        let got = d2max(&r.out[k - 1], &r.out[k]);
        assert!(got <= 3 * typ.max(own), "frame {k}: second difference {got}, follower's typical {typ}, beeper's own {own}");
        let ratio = got as f64 / typ.max(own) as f64;
        if ratio > worst.0 {
            worst = (ratio, k);
        }
    }
    println!("follower's typical second difference {typ}; the worst frame {} at {:.2}x its bound's base", worst.1, worst.0);
}

fn ms(x: f64) -> usize {
    (x / 1000.0 * sample_rate()).round() as usize
}

/// Asserts that output frame `out` leaves the follower for the delayed beeper `bd` in one
/// crossfade, at least 2 ms long and over by 4.5 ms (a cut would equal bd at once).
fn fades_to(out: &[i16], bd: &[i16], what: &str) {
    let settle = (0..ms(5.0)).rev().find(|&i| out[i] != bd[i]).map_or(0, |i| i + 1);
    assert!(settle >= ms(2.0) && settle <= ms(4.5), "{what}: the crossfade lasts {settle} samples");
    assert_eq!(out[ms(4.5)..], bd[ms(4.5)..], "{what}: the delayed beeper after the crossfade");
}

/// Asserts that output frame `out` leaves the delayed beeper `bd` for the follower no faster than
/// the 4 ms crossfade: sample i holds at most (i + 1) / 176 of the follower's (at most 16,640)
/// difference from bd.
fn fades_from(out: &[i16], bd: &[i16], what: &str) {
    let xf = sample_rate() / 250.0;
    for i in 0..ms(4.0) {
        let (d, room) = ((out[i] as i32 - bd[i] as i32).abs() as f64, (16640.0 + (bd[i] as f64).abs()) * (i + 1) as f64 / xf);
        assert!(d <= room + 2.0, "{what}: sample {i} is {d} from the delayed beeper, the crossfade allows {room:.0}");
    }
}

fn differs(a: &[i16], b: &[i16]) -> bool {
    a.iter().zip(b).filter(|(x, y)| (**x as i32 - **y as i32).abs() > 200).count() > a.len() / 4
}

/// A sweep, noise, two-voice music, a lone edge and a tone, each after silence.
fn sound_mix() -> (Vec<u64>, usize) {
    let mut ts = sweep(5 * FT, 20, 400.0, 1600.0);
    ts.extend(irregular(30 * FT, 2));
    ts.extend(two_voice(40 * FT, 10));
    ts.push(80 * FT + 30000);
    ts.extend(tone(90 * FT, 10, 600.0));
    (ts, 110)
}

#[test]
fn fx_off_is_delayed_beeper() {
    let (ts, n) = sound_mix();
    let fr = frames(&ts, n);
    let mut mix = mixer();
    mix.set_effects(false);
    let r = run(&mut mix, &fr);
    for k in 0..n {
        assert_eq!(r.out[k], r.delayed[k], "frame {k}");
        assert_eq!(r.gate[k], SND_FX_OFF, "frame {k}");
    }
    // the same mix with effects on does re-voice it
    let on = run(&mut mixer(), &fr);
    assert!((0..n).any(|k| on.out[k] != on.delayed[k]), "effects on change the sound");
}

#[test]
fn fx_sweep_is_followed() {
    let (s0, len) = (10usize, 50usize);
    let fr = frames(&sweep(s0 as u64 * FT, len, 400.0, 1600.0), s0 + len + 20);
    let r = run(&mut mixer(), &fr);
    let mut worst = 0.0f64;
    for j in 0..len {
        let k = s0 + j;
        assert_eq!(r.gate[k], SND_FX_PLAY, "sweep frame {j}");
        assert!(differs(&r.out[k], &r.delayed[k]), "sweep frame {j} plays the follower");
        let (got, want) = (pitch(&r.out[k]), sweep_hz(j, len, 400.0, 1600.0));
        assert!((got / want - 1.0).abs() < 0.03, "sweep frame {j}: pitch {got:.1} Hz, the sweep's {want:.1} Hz");
        worst = worst.max((got / want - 1.0).abs());
    }
    println!("the follower's pitch within {:.2}% of the sweep's", worst * 100.0);
    // then silence: the follower's release, then nothing
    let after = &r.out[s0 + len + 2];
    assert!(after.iter().all(|&x| x.abs() < 100), "silence after the sweep");
}

#[test]
fn fx_two_voice_stays_beeper() {
    let (s0, len) = (5usize, 20usize);
    let fr = frames(&two_voice(s0 as u64 * FT, len), s0 + len + 5);
    let r = run(&mut mixer(), &fr);
    for j in 0..len {
        let k = s0 + j;
        assert_eq!(r.gate[k], SND_FX_TWO_VOICE, "frame {j}");
        if j > 0 {
            assert_eq!(r.out[k], r.delayed[k], "frame {j} is the delayed beeper");
        }
    }
}

#[test]
fn fx_busy_frame_stays_beeper() {
    let (s0, len) = (5usize, 10usize);
    let fr = frames(&busy(s0 as u64 * FT, len), s0 + len + 5);
    assert!(fr[s0 + 1].edges.len() > 256, "frames busier than the decoder keeps");
    let r = run(&mut mixer(), &fr);
    for j in 0..len {
        let k = s0 + j;
        assert_eq!(r.gate[k], SND_FX_BUSY, "frame {j}");
        if j > 0 {
            assert_eq!(r.out[k], r.delayed[k], "frame {j} is the delayed beeper");
        }
    }
}

#[test]
fn fx_irregular_runs() {
    // the pattern reads as noise: its full periods (adjacent gap pairs) spread well over 0.3
    let pairs: Vec<u64> = PATTERN.iter().zip(PATTERN.iter().cycle().skip(1)).map(|(a, b)| a + b).collect();
    for w in 0..pairs.len() {
        let mut q: Vec<u64> = (0..6).map(|i| pairs[(w + i) % pairs.len()]).collect();
        q.sort();
        assert!((q[5] - q[0]) as f64 / q[3] as f64 > 0.4, "window {w} is irregular");
    }
    assert_eq!(PATTERN.iter().sum::<u64>(), FT, "the pattern fills a frame");

    let (s0, len) = (5usize, 10usize);
    let t0 = s0 + len;   // the tone's first frame
    let lone = t0 + 5;   // one irregular frame inside the tone
    let mut ts = irregular(s0 as u64 * FT, len);
    ts.extend(tone(t0 as u64 * FT, 5, 600.0));
    ts.extend(irregular(lone as u64 * FT, 1));
    ts.extend(tone((lone + 1) as u64 * FT, 4, 600.0));
    let fr = frames(&ts, t0 + 10 + 5);
    let r = run(&mut mixer(), &fr);
    for j in 0..len {
        let k = s0 + j;
        assert_eq!(r.gate[k], SND_FX_IRREGULAR, "irregular frame {j}");
        if j > 0 {
            assert_eq!(r.out[k], r.delayed[k], "irregular frame {j} is the delayed beeper");
        }
    }
    for k in t0..t0 + 10 {
        assert_eq!(r.gate[k], SND_FX_PLAY, "tone frame {}", k - t0);
    }
    assert!(differs(&r.out[lone], &r.delayed[lone]), "the lone irregular frame plays the follower");
}

#[test]
fn fx_lone_edge_ticks() {
    let at = 100 * FT + 20000;   // 2 s of silence before it
    let fr = frames(&[at], 110);
    let r = run(&mut mixer(), &fr);
    let k = (at / FT) as usize;
    assert_eq!(r.gate[k], SND_FX_PLAY);
    let out: Vec<i16> = r.out.concat();
    let start: usize = r.out[..k].iter().map(|o| o.len()).sum();
    let s = start + ((at % FT) as usize * r.out[k].len()) / ZX_FRAME_TSTATES as usize;
    let peak = out[s..s + ms(5.0)].iter().map(|&x| (x as i32).abs()).max().unwrap();
    assert!((1600..=4800).contains(&peak), "the tick peaks at {peak}");
    let late = out[s + ms(30.0)..s + ms(35.0)].iter().map(|&x| (x as i32).abs()).max().unwrap();
    assert!(late < 100, "30 ms later the output is {late}");
    println!("the tick peaks at {peak}, 30 ms later {late}");
    // the beeper would have held a step there, not a tick
    assert!(out[..s].iter().all(|&x| x == 0), "silence before the edge");
}

#[test]
fn fx_switches_crossfade() {
    let mut ts = Vec::new();
    let mut follower = Vec::new();
    for rep in 0..3 {
        let b = (5 + rep * 18) as u64;
        ts.extend(sweep(b * FT, 6, 400.0, 1600.0));
        ts.extend(two_voice((b + 6) * FT, 6));
        follower.extend(b as usize..b as usize + 6);
    }
    let n = 5 + 3 * 18 + 5;
    let fr = frames(&ts, n);
    let r = run(&mut mixer(), &fr);
    for rep in 0..3 {
        let b = 5 + rep * 18;
        assert!((b..b + 6).all(|k| r.gate[k] == SND_FX_PLAY), "repeat {rep}: the sweep plays the follower");
        assert!((b + 6..b + 12).all(|k| r.gate[k] == SND_FX_TWO_VOICE), "repeat {rep}: two voices stay beeper");
        fades_to(&r.out[b + 6], &r.delayed[b + 6], &format!("repeat {rep}: sweep -> two voices"));
        fades_from(&r.out[b + 12], &r.delayed[b + 12], &format!("repeat {rep}: two voices -> silence"));
    }
    no_jumps(&r, &follower);
}

#[test]
fn fx_toggle_crossfades() {
    let (s0, len) = (5usize, 50usize);
    let fr = frames(&sweep(s0 as u64 * FT, len, 400.0, 1600.0), s0 + len + 5);
    // effects off before call 20, on again before call 35: output frames 17 and 32
    let (off, on) = (20usize, 35usize);
    let r = run_calls(&mut mixer(), &fr, |m, t| {
        if t == off {
            m.set_effects(false);
        }
        if t == on {
            m.set_effects(true);
        }
        SND_MODE_FM_B
    });
    let follower: Vec<usize> = (s0..s0 + len).filter(|&k| k < off - DELAY || k > on - DELAY).collect();
    no_jumps(&r, &follower);
    let (k_off, k_on) = (off - DELAY, on - DELAY);
    // one 4 ms crossfade each way; in between the delayed beeper exactly
    fades_to(&r.out[k_off], &r.delayed[k_off], "effects off");
    for k in k_off + 1..k_on {
        assert_eq!(r.out[k], r.delayed[k], "frame {k}: effects off");
    }
    fades_from(&r.out[k_on], &r.delayed[k_on], "effects on");
    assert!(differs(&r.out[k_on + 1], &r.delayed[k_on + 1]), "effects on again: the follower");
}

#[test]
fn fx_only_in_fm_engines() {
    let (ts, n) = sound_mix();
    let fr = frames(&ts, n);
    // FM: sound: the follower never sounds
    let a_on = run_calls(&mut mixer(), &fr, |_, _| SND_MODE_FM_A);
    let mut m = mixer();
    m.set_effects(false);
    let a_off = run_calls(&mut m, &fr, |_, _| SND_MODE_FM_A);
    assert_eq!(a_on.out, a_off.out, "FM: sound with effects on and off");
    assert!(a_on.gate.iter().all(|&g| g == SND_FX_OFF), "FM: sound reports the follower off");
    // Beeper: the live beeper
    let mut m = mixer();
    for (t, f) in fr.iter().enumerate() {
        assert_eq!(m.frame(SND_MODE_BEEPER, None, &f.edges, f.start_level, &f.beeper), f.beeper, "Beeper mode, frame {t}");
        assert_eq!(m.fx_gate(), SND_FX_OFF);
    }
    // switching FM: engines -> FM: sound mid-sweep (frame 12) and back (frame 20): once the
    // crossfade is over, FM: sound plays as with effects off
    let mode = |t: usize| if (12 + DELAY..20 + DELAY).contains(&t) { SND_MODE_FM_A } else { SND_MODE_FM_B };
    let on = run_calls(&mut mixer(), &fr, |_, t| mode(t));
    let mut m = mixer();
    m.set_effects(false);
    let off = run_calls(&mut m, &fr, |_, t| mode(t));
    assert!(differs(&on.out[11], &off.out[11]), "FM: engines re-voices the sweep");
    for k in 13..20 {
        assert_eq!(on.out[k], off.out[k], "frame {k}: FM: sound after the switch");
    }
    assert!(differs(&on.out[21], &off.out[21]), "back in FM: engines, the follower again");
}

/// Per call t of a machine run: the mixer's output (frame t - SND_DELAY_FRAMES), the delayed
/// beeper it would play without the follower, the gate, whether the mixer plays FM, and method
/// B's record of that frame.
struct MRun {
    out: Vec<Vec<i16>>,
    delayed: Vec<Vec<i16>>,
    gate: Vec<u8>,
    fm_on: Vec<bool>,
    rec: Vec<(u8, u8)>,
}

/// Runs `m` for `frames` frames through `mix` in FM: engines.
fn run_machine(mix: &mut SndMixer, mut m: Machine, frames: usize) -> MRun {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut all: Vec<i16> = Vec::new();
    let mut start = vec![0usize];
    let mut r = MRun { out: vec![], delayed: vec![], gate: vec![], fm_on: vec![], rec: vec![] };
    for t in 0..frames {
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        all.extend_from_slice(&f.beeper);
        start.push(all.len());
        let o = mix.frame(SND_MODE_FM_B, Some(&m), &f.edges, f.start_level, &f.beeper);
        if t >= DELAY {
            let from = start[t] - start[DELAY];   // the mixer delays by a fixed number of samples
            r.delayed.push(all[from..from + o.len()].to_vec());
            r.out.push(o);
            r.gate.push(mix.fx_gate());
            r.fm_on.push(mix.fm_on());
            r.rec.push(mix.b_record(t - DELAY));
        }
    }
    r
}

/// The ROM BEEP entry misread (its period a semitone too long): B's check refuses the melody's
/// notes. The run, with the length of the rejected run each frame is in (0: not rejected).
fn misread_beep_run() -> (MRun, Vec<usize>) {
    let tap = beep_tap();
    let mut m = Machine::new();
    m.tape_insert(&tap).expect("inserts");
    m.tape_autostart();
    let mut mix = mixer();
    mix.set_misread_engines(MISREAD_BEEP);
    let r = run_machine(&mut mix, m, 300);
    let rej: Vec<bool> = r.rec.iter().map(|&x| x == (1, 0)).collect();
    let mut len = vec![0usize; rej.len()];
    let mut k = 0;
    while k < rej.len() {
        let mut e = k;
        while e < rej.len() && rej[e] && rej[k] {
            e += 1;
        }
        if rej[k] {
            len[k..e].fill(e - k);
            k = e;
        } else {
            k += 1;
        }
    }
    (r, len)
}

#[test]
fn fx_b_rejected_short_run_stays_beeper() {
    // a brief rejection (1-2 frames) inside a known tune: the follower leaves it on the beeper
    let (r, len) = misread_beep_run();
    let (mut short, mut exact) = (0, 0);
    for k in 0..r.out.len() {
        if len[k] == 0 || len[k] >= 3 || r.fm_on[k] {
            continue;   // not a short rejected run (or FM is still held through it: the gate reads OFF)
        }
        short += 1;
        assert_eq!(r.gate[k], SND_FX_B_REJECTED, "frame {k}");
        // the first frame of a run may still fade from the follower; from the second the beeper
        // alone (the frame before it was not the follower's either)
        if k > 0 && !r.fm_on[k - 1] && r.gate[k - 1] != SND_FX_PLAY {
            exact += 1;
            assert_eq!(r.out[k], r.delayed[k], "frame {k} is the delayed beeper");
        }
    }
    println!("{short} frames of 1-2 frame rejected runs stay beeper, {exact} of them exactly");
    assert!(short >= 3, "the fixture has short rejected runs ({short} frames)");
}

#[test]
fn fx_b_rejected_long_run_is_followed() {
    // the melody's notes are refused for many frames in a row: a run of 3 or more is followed
    let (r, len) = misread_beep_run();
    let (mut played, mut differ) = (0, 0);
    for k in 0..r.out.len() {
        if len[k] < 3 || r.fm_on[k] {
            continue;
        }
        assert_ne!(r.gate[k], SND_FX_B_REJECTED, "frame {k} is in a run of {}", len[k]);
        if r.gate[k] == SND_FX_PLAY {
            played += 1;
            if r.out[k] != r.delayed[k] {
                differ += 1;
            }
        }
    }
    println!("{played} frames of long rejected runs followed, {differ} differ from the beeper");
    assert!(played > 50 && differ > 50, "the long run is followed ({played} played, {differ} differ)");
}

#[test]
fn fx_busy_then_tone_starts_clean() {
    // a busy frame keeps only its first 256 edges, so the follower's last transition is mid-frame;
    // the tone after it starts as a new sound (its first gap is not a 10,000 T half period of
    // noise): the first frame after the busy ones already sounds the tone's pitch
    let (s0, nb) = (5usize, 6usize);
    let mut ts = busy(s0 as u64 * FT, nb);
    ts.extend(tone((s0 + nb) as u64 * FT + 1000, 12, 600.0));
    let fr = frames(&ts, s0 + nb + 12 + 3);
    let r = run(&mut mixer(), &fr);
    let k = s0 + nb;
    assert_eq!(r.gate[k - 1], SND_FX_BUSY);
    assert_eq!(r.gate[k], SND_FX_PLAY, "the tone's first frame");
    let (first, steady) = (periodicity(&r.out[k]), periodicity(&r.out[k + 6]));
    println!("periodicity of the first tone frame {first:.2}, of a steady one {steady:.2}");
    assert!(first > 0.8, "the first tone frame is a clean tone (periodicity {first:.2}; steady {steady:.2})");
    let got = pitch(&r.out[k]);
    assert!((got / 600.0 - 1.0).abs() < 0.03, "the first frame after the busy ones sounds {got:.1} Hz, the tone's 600 Hz");
}

#[test]
fn fx_fm_music_unchanged_with_effects_on() {
    // a known engine's tune in FM: engines: where the mixer plays FM (and did the frame before,
    // past the crossfade) the output is the same with the follower on and off
    let mut off = mixer();
    off.set_effects(false);
    let on = run_machine(&mut mixer(), castaway(), 700);
    let off = run_machine(&mut off, castaway(), 700);
    let mut same = 0;
    for k in 1..on.out.len() {
        if on.fm_on[k] && off.fm_on[k] && on.fm_on[k - 1] && off.fm_on[k - 1] {
            assert_eq!(on.out[k], off.out[k], "frame {k}");
            same += 1;
        }
    }
    println!("{same} FM frames identical with effects on and off");
    assert!(same > 100, "the tune plays in FM ({same} frames)");
}

