// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The decoder's work per frame is capped (`SND_WORK_MAX`), whatever the edges: it runs on
//! every frame in every sound mode, so dense or adversarial edge streams (speaker flips, time
//! slots and short pulses) must not cost more than a small multiple of real music.

use music::synthetic::{
    frame_loop, frame_tune, pin_pulse, pin_tune, slots_loop, slots_tune, xor_loop, xor_tune, FRAME_T, PIN_ROW_T, SLOTS_AT, SLOTS_LOOP,
};
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

/// A frame of a 3-slot loop of `len` T (`OUT`s at `SLOTS_AT` scaled to it) whose channel c
/// outputs `bit(pass, c)` in pass `pass` of the frame.
fn tdm(len: u64, mut bit: impl FnMut(u64, usize) -> bool) -> Vec<u32> {
    let mut v = Vec::new();
    let mut level = false;
    let mut i = 0u64;
    loop {
        for (c, &at) in SLOTS_AT.iter().enumerate() {
            let t = i * len + at * len / SLOTS_LOOP;
            if t >= FRAME_T {
                return v;
            }
            let b = bit(i, c);
            if b != level {
                level = b;
                v.push(t as u32);
            }
        }
        i += 1;
    }
}

/// Edges of pulses `width` T wide starting at `starts` (OR-ed where they overlap), within a frame.
fn pulse_edges(mut starts: Vec<u32>, width: u32) -> Vec<u32> {
    starts.sort();
    let mut v = Vec::new();
    let mut cur: Option<(u32, u32)> = None;
    for a in starts {
        let b = a + width;
        match cur {
            Some((c0, c1)) if a <= c1 => cur = Some((c0, c1.max(b))),
            Some((c0, c1)) => {
                v.extend([c0, c1]);
                cur = Some((a, b));
            }
            None => cur = Some((a, b)),
        }
    }
    if let Some((c0, c1)) = cur {
        v.extend([c0, c1]);
    }
    v.retain(|&t| t < FRAME_T as u32);
    v.truncate(254);
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
    // time-slot streams (a loop carrier: channel 0 always on, channel 1 always off, so every pass
    // has a rising edge and the time-slot decoder runs on them), whose third channel
    // takes random values every pass
    gen("TDM random channel", &mut |_, r| tdm(SLOTS_LOOP, |_, c| c == 0 || (c == 2 && r.next() % 2 == 0)));
    // toggles every pass (11 kHz: more rises than any voice, the rise rings overrun)
    gen("TDM fast channel", &mut |_, _| tdm(SLOTS_LOOP, |i, c| c == 0 || (c == 2 && i % 2 == 0)));
    // changes its period every pass (no run of periods ever agrees)
    gen("TDM chirp", &mut |f, _| tdm(SLOTS_LOOP, |i, c| c == 0 || (c == 2 && ((i * i + f * 7) / 97) % 2 == 0)));
    // the shortest loop (40 T: the most grid points a frame can have) with a random channel
    gen("40 T loop, random channel", &mut |_, r| tdm(40, |_, c| c == 0 || (c == 2 && r.next() % 2 == 0)));
    // the most edges a frame can hold, 17 T apart with jitter
    gen("4096 dense edges", &mut |_, r| (0..4096u32).map(|i| i * 17 + r.next() % 9).filter(|&t| t < FRAME_T as u32).collect());
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
    // short pulses (one width, the speaker high a few percent of the time: the short-pulse
    // decoder runs on them): 127 at random places, just under the busy limit
    gen("pulses random 127", &mut |_, r| pulse_edges((0..127).map(|_| r.next() % 69_600).collect(), 80));
    // three pulse voices near 2.5 kHz, with the rises lost where they merge given back
    gen("pulses 3 voices near 2.5 kHz", &mut |f, _| pulse_edges([1_401, 1_467, 1_539].iter().flat_map(|&p| lattice(f, p)).collect(), 60));
    // pulses half of which merge with another 40 T later: every one gives a rise back
    gen("pulses merging", &mut |_, r| {
        let mut v = Vec::new();
        let mut t = r.next() % 500;
        while t < 69_000 && v.len() < 200 {
            v.push(t);
            if r.next() % 2 == 0 {
                v.push(t + 40);
            }
            t += 500 + r.next() % 600;
        }
        pulse_edges(v, 100)
    });
    // a 60 Hz pulse bass under random pulses (long chains through dense rises)
    gen("pulses bass + random", &mut |f, r| {
        let mut v = lattice(f, 58_000);
        v.extend((0..120).map(|_| r.next() % 69_600));
        pulse_edges(v, 100)
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
    let game = |mut m: zx_sys::Machine, n: usize| -> Vec<Vec<zx_edge>> {
        capture(&mut m, &mut Beeper::new(RATE.0, RATE.1), n).into_iter().map(|f| f.edges).collect()
    };
    let music = [
        ("xor tune", xor_loop(&xor_tune(0)).frames),
        ("frame tune", frame_loop(&frame_tune(40)).frames),
        ("slots tune", slots_loop(&slots_tune()).frames),
        ("pin-pulse tune", pin_pulse(&pin_tune(), PIN_ROW_T, true).frames),
        ("castaway", game(music::castaway(), 2150)),
        ("colony8", game(music::colony8(), 1550)),
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

/// `noise.sna` (fmzx-pack's testprogs; `fw_budget` measures the firmware's frame on it as the
/// decoder's worst case) must stay that worst case: its 250 edges a frame at pseudo-random
/// spacings drive the decoder to at least 90% of its work cap, and are never music.
#[test]
fn noise_sna_nears_the_work_cap() {
    let mut m = zx_sys::Machine::new();
    m.load_sna(&fmzx_pack::testprogs::noise_sna()).expect("loads");
    let mut d = SndDecoder::new();
    let (mut peak, mut sum) = (0u32, 0u64);
    let frames = 300;
    for f in 0..frames {
        m.run_frame();
        let fr = d.frame(m.edges(), m.edges_start_level());
        assert_eq!(fr.confident, 0, "frame {f}: noise decoded as music");
        peak = peak.max(d.work());
        sum += u64::from(d.work());
    }
    println!(
        "noise.sna: decoder work peak {peak} ({:.0}% of {SND_WORK_MAX}), average {}",
        f64::from(peak) * 100.0 / f64::from(SND_WORK_MAX),
        sum / frames
    );
    assert!(peak * 10 >= SND_WORK_MAX * 9, "noise.sna peaks at {peak} work units, under 90% of {SND_WORK_MAX}");
}
