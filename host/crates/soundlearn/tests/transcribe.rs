// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The transcriber on synthesized 1-bit signals made from known note lists: a square-wave
//! melody, two and three voices of narrow pulses OR-ed into the bit, three phase accumulators
//! mixed by pulse-width shares, chords with rests between them, staccato bursts of a square
//! wave shorter than the analysis window. A transcribed note matches a
//! true one (each at most once) when its pitch is within 30 cents and their overlap is at least
//! half of the longer one; precision is the share of transcribed notes matched, recall the share
//! of true notes.

use soundlearn::spectrum::CLOCK_HZ;
use soundlearn::transcribe::{transcribe, Note, Transcriber};
use zx_sys::{zx_edge, ZX_FRAME_TSTATES};

const FRAME: u64 = ZX_FRAME_TSTATES as u64;

/// A true note: absolute T-states from the start of frame 0, and its period.
#[derive(Clone, Copy, Debug)]
struct True {
    start: u64,
    end: u64,
    period: f64,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * f64::from(self.next() % 10_000) / 10_000.0
    }
    fn int(&mut self, lo: u64, hi: u64) -> u64 {
        lo + u64::from(self.next()) % (hi - lo + 1)
    }
}

/// A melody from `at`: `n` notes of `lo`-`hi` Hz lasting `len` frames (a range), each followed
/// by a rest of 0-`rest` frames.
fn melody(rng: &mut Rng, at: u64, n: usize, lo: f64, hi: f64, len: (u64, u64), rest: u64) -> Vec<True> {
    let mut t = at;
    let mut out = Vec::new();
    let mut prev = 0.0;
    for _ in 0..n {
        let mut hz = lo * (hi / lo).powf(rng.range(0.0, 1.0));
        // a new pitch at least a semitone from the last: a repeat would be a matter of onsets
        while prev > 0.0 && (hz / prev).log2().abs() < 1.0 / 12.0 {
            hz = lo * (hi / lo).powf(rng.range(0.0, 1.0));
        }
        prev = hz;
        let d = rng.int(len.0, len.1) * FRAME + rng.int(0, FRAME / 2);
        out.push(True { start: t, end: t + d, period: CLOCK_HZ / hz });
        t += d + rng.int(0, rest) * FRAME;
    }
    out
}

/// High intervals of a square wave per note.
fn square(notes: &[True]) -> Vec<(u64, u64)> {
    let mut iv = Vec::new();
    for n in notes {
        let mut k = 0.0;
        loop {
            let a = n.start + (k * n.period).round() as u64;
            if a >= n.end {
                break;
            }
            iv.push((a, (n.start + ((k + 0.5) * n.period).round() as u64).min(n.end)));
            k += 1.0;
        }
    }
    iv
}

/// Staccato notes: each a burst of a square wave from just after a frame's interrupt, `len`
/// T-states long, the next 4-9 frames on; high for `duty` of each period. The loop is
/// `drift` slower while the screen is drawn (contended memory), so each true period is the
/// burst's mean.
fn staccato(rng: &mut Rng, n: usize, lo: f64, hi: f64, len: u64, duty: f64, drift: f64) -> (Vec<True>, Vec<(u64, u64)>) {
    let mut notes = Vec::new();
    let mut iv = Vec::new();
    let mut frame = 2;
    let screen = |t: u64| (14_336..57_344).contains(&(t % FRAME));
    for _ in 0..n {
        let hz = lo * (hi / lo).powf(rng.range(0.0, 1.0));
        let period = CLOCK_HZ / hz;
        let start = frame * FRAME + 600 + rng.int(0, 60);
        let (mut t, mut cycles) = (start as f64, 0u32);
        while t + period < (start + len) as f64 {
            let p = if screen(t as u64) { period * (1.0 + drift) } else { period };
            iv.push((t.round() as u64, (t + duty * p).round() as u64));
            t += p;
            cycles += 1;
        }
        notes.push(True { start, end: t.round() as u64, period: (t - start as f64) / f64::from(cycles) });
        frame += rng.int(4, 9);
    }
    (notes, iv)
}

/// High intervals of a pulse `width` T-states wide every period per note.
fn pulses(notes: &[True], width: u64) -> Vec<(u64, u64)> {
    let mut iv = Vec::new();
    for n in notes {
        let mut k = 0.0;
        loop {
            let a = n.start + (k * n.period).round() as u64;
            if a >= n.end {
                break;
            }
            iv.push((a, a + width));
            k += 1.0;
        }
    }
    iv
}

/// Phase accumulators, one per voice, stepped every `pass` T-states: a voice whose 16-bit
/// accumulator carries adds its share to the pass's pulse, high from the pass's start.
fn pwm(voices: &[Vec<True>], pass: u64, shares: &[u64]) -> Vec<(u64, u64)> {
    let end = voices.iter().flatten().map(|n| n.end).max().unwrap_or(0);
    let mut acc = vec![0u32; voices.len()];
    let mut iv = Vec::new();
    let mut t = 0;
    while t < end {
        let mut width = 0;
        for (v, notes) in voices.iter().enumerate() {
            if let Some(n) = notes.iter().find(|n| n.start <= t && t < n.end) {
                let step = (65536.0 * pass as f64 / n.period).round() as u32;
                acc[v] += step;
                if acc[v] >= 65536 {
                    acc[v] -= 65536;
                    width += shares[v];
                }
            }
        }
        if width > 0 {
            iv.push((t, t + width.min(pass - 1)));
        }
        t += pass;
    }
    iv
}

/// The frames' edges and start levels of the OR of high intervals, and T0 (the first edge's
/// T-state in frame 0).
fn frames(mut iv: Vec<(u64, u64)>, n_frames: u64) -> (Vec<(Vec<zx_edge>, u8)>, u32) {
    iv.sort();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (a, b) in iv {
        match merged.last_mut() {
            Some(m) if a <= m.1 => m.1 = m.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut out: Vec<(Vec<zx_edge>, u8)> = (0..n_frames).map(|_| (Vec::new(), 0)).collect();
    for &(a, b) in &merged {
        for (t, level) in [(a, 2u8), (b, 0u8)] {
            let f = t / FRAME;
            if f < n_frames {
                out[f as usize].0.push(zx_edge { tstate: (t % FRAME) as u32, level, pc: 0x8000 });
            }
        }
        // frames wholly inside start high
        for f in (a / FRAME + 1)..=(b.saturating_sub(1) / FRAME).min(n_frames.saturating_sub(1)) {
            if f * FRAME > a && f * FRAME < b {
                out[f as usize].1 = 2;
            }
        }
    }
    let t0 = out[0].0.first().map_or(0, |e| e.tstate);
    (out, t0)
}

/// (precision, recall) of `got` against `want` (see the file's docs); `t0` converts the true
/// notes' times.
fn score(got: &[Note], want: &[True], t0: u32) -> (f64, f64) {
    let mut used = vec![false; want.len()];
    let mut hits = 0;
    for g in got {
        let mut best: Option<(usize, u64)> = None;
        for (i, w) in want.iter().enumerate() {
            if used[i] {
                continue;
            }
            let (ws, we) = (w.start.saturating_sub(u64::from(t0)), w.end.saturating_sub(u64::from(t0)));
            let cents = 1200.0 * (f64::from(g.period_t) / w.period).log2();
            let ov = g.end_t.min(we).saturating_sub(g.start_t.max(ws));
            let longer = (g.end_t - g.start_t).max(we - ws);
            if cents.abs() <= 30.0 && 2 * ov >= longer && best.is_none_or(|b| ov > b.1) {
                best = Some((i, ov));
            }
        }
        if let Some((i, _)) = best {
            used[i] = true;
            hits += 1;
        }
    }
    (hits as f64 / got.len().max(1) as f64, hits as f64 / want.len().max(1) as f64)
}

/// Cents between each matched note's period and the true one: the largest.
fn worst_cents(got: &[Note], want: &[True], t0: u32) -> f64 {
    let mut worst: f64 = 0.0;
    for g in got {
        let mid = (g.start_t + g.end_t) / 2 + u64::from(t0);
        let near = want
            .iter()
            .filter(|w| w.start <= mid && mid < w.end)
            .map(|w| 1200.0 * (f64::from(g.period_t) / w.period).log2())
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        if let Some(c) = near.filter(|c| c.abs() <= 30.0) {
            worst = worst.max(c.abs());
        }
    }
    worst
}

fn check(name: &str, iv: Vec<(u64, u64)>, want: &[True], min_p: f64, min_r: f64) -> Vec<Note> {
    let n_frames = want.iter().map(|n| n.end).max().unwrap() / FRAME + 10;
    let (fr, t0) = frames(iv, n_frames);
    let got = transcribe(&fr, t0);
    let (p, r) = score(&got, want, t0);
    let worst = worst_cents(&got, want, t0);
    eprintln!("{name}: {} notes, {} true: precision {p:.3} recall {r:.3}, worst pitch {worst:.1} cents", got.len(), want.len());
    for g in &got {
        assert!(g.voice < 3 && g.end_t > g.start_t && g.level >= 1, "{g:?}");
        for h in &got {
            assert!(g == h || g.voice != h.voice || g.end_t <= h.start_t || h.end_t <= g.start_t, "{g:?} overlaps {h:?}");
        }
    }
    assert!(p >= min_p && r >= min_r, "{name}: precision {p:.3} (at least {min_p}), recall {r:.3} (at least {min_r})");
    got
}

#[test]
fn a_square_melody() {
    let mut rng = Rng(0x1234_5678);
    let want = melody(&mut rng, 1000, 80, 60.0, 2500.0, (4, 14), 3);
    let got = check("square melody", square(&want), &want, 0.95, 0.95);
    assert!(worst_cents(&got, &want, 1000) < 5.0);
    // a melody sounds one note at a time: a stray note overlapping one (a harmonic where a
    // short note ends) at most
    let overlapping = got.iter().filter(|a| got.iter().any(|b| a != &b && a.start_t < b.end_t && b.start_t < a.end_t)).count();
    assert!(overlapping <= 4, "{overlapping} notes overlap another");
}

#[test]
fn two_interleaved_pulse_voices() {
    let mut rng = Rng(0x9E37_79B9);
    let mut want = melody(&mut rng, 500, 70, 300.0, 2000.0, (4, 10), 2);
    want.extend(melody(&mut rng, 500, 40, 45.0, 280.0, (8, 20), 2));
    check("two pulse voices", pulses(&want, 60), &want, 0.88, 0.95);
}

#[test]
fn three_interleaved_pulse_voices() {
    let mut rng = Rng(0x0BAD_F00D);
    let mut want = melody(&mut rng, 700, 70, 600.0, 2500.0, (4, 10), 2);
    want.extend(melody(&mut rng, 700, 50, 180.0, 580.0, (6, 14), 2));
    want.extend(melody(&mut rng, 700, 35, 40.0, 170.0, (10, 24), 2));
    check("three pulse voices", pulses(&want, 40), &want, 0.75, 0.9);
}

#[test]
fn three_accumulators_mixed_by_pulse_width() {
    let mut rng = Rng(0x5EED_1234);
    let a = melody(&mut rng, 0, 60, 500.0, 2000.0, (4, 10), 2);
    let b = melody(&mut rng, 0, 45, 150.0, 480.0, (6, 14), 2);
    let c = melody(&mut rng, 0, 30, 50.0, 145.0, (10, 24), 2);
    let iv = pwm(&[a.clone(), b.clone(), c.clone()], 240, &[30, 40, 50]);
    let want: Vec<True> = a.into_iter().chain(b).chain(c).collect();
    check("three accumulators", iv, &want, 0.83, 0.93);
}

#[test]
fn chords_and_rests() {
    let mut rng = Rng(0xC0DE_CAFE);
    let mut want = Vec::new();
    let mut t = 300;
    for _ in 0..30 {
        let root = 80.0 * 2f64.powf(rng.range(0.0, 3.0));
        let len = rng.int(8, 20) * FRAME;
        // a triad: root, third (major or minor) and fifth, the third an octave up
        let third = if rng.next().is_multiple_of(2) { 4.0 } else { 3.0 };
        for semis in [0.0, 12.0 + third, 7.0] {
            let hz = root * 2f64.powf(semis / 12.0);
            want.push(True { start: t, end: t + len, period: CLOCK_HZ / hz });
        }
        t += len + rng.int(2, 6) * FRAME;
    }
    check("chords and rests", pulses(&want, 50), &want, 0.95, 0.95);
}

/// Notes of 1.3 frames, a sixth of the window, 4-9 frames apart: each its own stretch of sound,
/// heard through a window clipped to it; the duty a little off a half (weak even harmonics) and
/// the period 1.5% longer while the screen is drawn.
#[test]
fn staccato_bursts_shorter_than_the_window() {
    let mut rng = Rng(0x0005_7ACC);
    let (want, iv) = staccato(&mut rng, 80, 150.0, 2000.0, 90_000, 0.493, 0.015);
    let got = check("staccato bursts", iv, &want, 0.95, 0.95);
    // one line, each note as long as its burst
    assert!(got.iter().all(|n| n.voice == 0), "{got:?}");
    // frame 0 is silent: T0 is 0
    for g in &got {
        let mid = g.start_t + (g.end_t - g.start_t) / 2;
        let w = want.iter().find(|w| w.start <= mid && mid < w.end).expect("a burst");
        assert!(g.start_t.abs_diff(w.start) < FRAME / 4 && g.end_t.abs_diff(w.end) < FRAME / 4, "{g:?} against {w:?}");
    }
}

#[test]
fn times_count_from_the_first_edge() {
    // one note from T-state 5000 of frame 3 for 10 frames
    let start = 3 * FRAME + 5000;
    let want = [True { start, end: start + 10 * FRAME, period: 7954.0 }];
    let (fr, t0) = frames(square(&want), 20);
    // the stretch starts at frame 3: its first edge is at T-state 5000
    assert_eq!(fr[3].0[0].tstate, 5000);
    let got = transcribe(&fr[3..], 5000);
    assert_eq!(got.len(), 1, "{got:?}");
    let n = got[0];
    let frame = FRAME as i64;
    assert!((n.start_t as i64).abs() <= frame / 2, "{n:?}");
    assert!((n.end_t as i64 - 10 * frame).abs() <= frame / 2, "{n:?}");
    assert!((f64::from(n.period_t) / 7954.0 - 1.0).abs() < 0.001, "{n:?}");
    assert_eq!(n.level, 255);
    let _ = t0;
}

#[test]
fn silence_and_noise_have_no_notes() {
    let silent: Vec<(Vec<zx_edge>, u8)> = (0..200).map(|_| (Vec::new(), 2)).collect();
    assert!(transcribe(&silent, 0).is_empty());
    // random levels changing every 64-640 T-states
    let mut rng = Rng(0xFACE_B00C);
    let mut iv = Vec::new();
    let mut t = 0;
    while t < 300 * FRAME {
        let d = rng.int(64, 640);
        if rng.next().is_multiple_of(2) {
            iv.push((t, t + d));
        }
        t += d;
    }
    let (fr, t0) = frames(iv, 300);
    let got = transcribe(&fr, t0);
    // 6 s of loud broadband noise: two stray notes a second at most
    assert!(got.len() <= 12, "{} notes", got.len());
}

#[test]
fn a_long_stretch_is_quick() {
    let mut rng = Rng(0x7777_1111);
    let mut want = melody(&mut rng, 0, 600, 300.0, 2000.0, (4, 10), 2);
    want.extend(melody(&mut rng, 0, 400, 45.0, 280.0, (8, 20), 2));
    let (fr, t0) = frames(pulses(&want, 60), 9000);
    let t = std::time::Instant::now();
    let mut tr = Transcriber::new(t0);
    for (e, l) in &fr {
        tr.push_frame(e, *l);
    }
    let got = tr.notes();
    let secs = t.elapsed().as_secs_f64();
    eprintln!("9000 frames: {} notes in {secs:.2} s", got.len());
    assert!(secs < 20.0, "{secs}");
}
