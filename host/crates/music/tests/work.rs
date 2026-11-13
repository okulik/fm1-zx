// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The decoder's work per frame is capped (`SND_WORK_MAX`), whatever the edges: it runs on
//! every frame in every sound mode, so dense or adversarial edge streams must not cost more
//! than a small multiple of real music.

use music::synthetic::{frame_loop, frame_tune, xor_loop, xor_tune, FRAME_T};
use music::{capture, test_sna, RATE};
use zx_sys::{zx_edge, Beeper, SndDecoder, SND_WORK_MAX};

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

/// Edges at the T-states `t` (sorted, deduplicated), alternating the speaker level.
fn edges(mut t: Vec<u32>, level: &mut u8) -> Vec<zx_edge> {
    t.sort();
    t.dedup();
    t.into_iter()
        .map(|x| {
            *level ^= 2;
            zx_edge { tstate: x, level: *level }
        })
        .collect()
}

/// Edges at multiples of `period` (absolute T) that fall in frame `f`.
fn lattice(f: u64, period: u64) -> Vec<u32> {
    let f0 = f * FRAME_T;
    let mut v = Vec::new();
    let mut t = (f0 / period + 1) * period;
    while t < f0 + FRAME_T {
        v.push((t - f0) as u32);
        t += period;
    }
    v
}

/// The adversarial streams: 200 frames each.
fn adversarial() -> Vec<(&'static str, Vec<Vec<zx_edge>>)> {
    let mut rng = Rng(12345);
    let mut level = 0u8;
    let mut out = Vec::new();
    let mut gen = |name, f: &mut dyn FnMut(u64, &mut Rng) -> Vec<u32>| {
        let frames: Vec<Vec<zx_edge>> = (0..200).map(|i| edges(f(i, &mut rng), &mut level)).collect();
        out.push((name, frames));
    };
    // 255 uniformly random edges per frame (just under the busy limit)
    gen("random 255", &mut |_, r| (0..255).map(|_| r.next() % FRAME_T as u32).collect());
    // a 30 Hz bass and a dense cluster of jittered edges 137 T apart
    gen("30 Hz bass + cluster", &mut |f, r| {
        let mut v = lattice(f, 58_000);
        let n = v.len();
        v.extend((0..250 - n as u32).map(|i| 20_000 + i * 137 + r.next() % 40));
        v
    });
    // a 100 Hz bass under random edges
    gen("100 Hz bass + random", &mut |f, r| {
        let mut v = lattice(f, 17_500);
        while v.len() < 255 {
            v.push(r.next() % FRAME_T as u32);
        }
        v
    });
    // three voices near 2.5 kHz
    gen("3 voices near 2.5 kHz", &mut |f, _| {
        let mut v: Vec<u32> = [701, 733, 769].iter().flat_map(|&p| lattice(f, p)).collect();
        v.sort();
        v.truncate(255);
        v
    });
    // a tone over 65% of the frame, then dense edges
    gen("tone 65% + dense", &mut |_, r| {
        let mut v: Vec<u32> = (1_000..46_000).step_by(900).collect();
        let mut t = 46_500;
        while t < 69_800 && v.len() < 255 {
            v.push(t);
            t += 93 + r.next() % 30;
        }
        v
    });
    // white-noise-like bursts: random lengths, places and spacing
    gen("noise bursts", &mut |_, r| {
        let mut v = Vec::new();
        let mut t = r.next() % 30_000;
        let len = r.next() % 250;
        for _ in 0..len {
            t += 20 + r.next() % 400;
            if t >= FRAME_T as u32 {
                break;
            }
            v.push(t);
        }
        v
    });
    out
}

/// The work of each frame: (average, maximum, frames at the cap).
fn work(frames: &[Vec<zx_edge>]) -> (f64, u32, usize) {
    let mut d = SndDecoder::new();
    let mut level = 0;
    let w: Vec<u32> = frames
        .iter()
        .map(|e| {
            d.frame(e, level);
            level = e.last().map_or(level, |x| x.level);
            d.work()
        })
        .collect();
    let max = *w.iter().max().unwrap();
    let capped = w.iter().filter(|&&x| x >= SND_WORK_MAX).count();
    (w.iter().map(|&x| x as f64).sum::<f64>() / w.len() as f64, max, capped)
}

#[test]
fn decoder_work_is_bounded() {
    let sna = |name: &str, n: usize| -> Vec<Vec<zx_edge>> {
        capture(&mut test_sna(name), &mut Beeper::new(RATE.0, RATE.1), n).into_iter().map(|f| f.edges).collect()
    };
    let music = [
        ("xor tune", xor_loop(&xor_tune(0)).frames),
        ("frame tune", frame_loop(&frame_tune(40)).frames),
        ("tune.sna", sna("tune.sna", 450)),
        ("border.sna", sna("border.sna", 200)),
    ];
    let mut music_max = 0;
    for (name, frames) in &music {
        let (avg, max, capped) = work(frames);
        println!("{name}: work avg {avg:.0}, max {max}, frames at the frame cap {capped}");
        assert_eq!(capped, 0, "{name}: music never reaches the cap");
        music_max = music_max.max(max);
    }
    println!("cap {SND_WORK_MAX} = {:.1} x the music's most", SND_WORK_MAX as f64 / music_max as f64);
    for (name, frames) in adversarial() {
        let (avg, max, capped) = work(&frames);
        println!("{name}: work avg {avg:.0}, max {max}, frames at the frame cap {capped}");
        // the searches stop at the cap; the pass over the pool that crosses it finishes
        assert!(max <= SND_WORK_MAX + 2 * 512 + 64, "{name}: {max} units in a frame");
    }
}
