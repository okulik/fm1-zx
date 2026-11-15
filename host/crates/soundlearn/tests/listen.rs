// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Listening to `games/test/tune.sna`: the speaker routine's period samples and snapshots, and
//! the generic input schedule.

use soundlearn::listen::{listen, listen_with, schedule, Inputs, LISTEN_FRAMES};
use zx_sys::Machine;

/// Pitches of the tune's eight notes (`fmzx_pack::testprogs::TUNE_HZ`).
const TUNE_HZ: [f64; 8] = [524.67, 586.65, 657.01, 701.11, 781.02, 876.39, 989.70, 1049.28];

fn tune() -> Machine {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna");
    let mut m = Machine::new();
    m.load_sna(&std::fs::read(path).expect("tune.sna")).expect("loads");
    m
}

fn none() -> Inputs {
    Box::new(|_| ([0; 8], 0))
}

#[test]
fn listen_tune() {
    let heard = listen(&mut tune(), &none(), 600);
    assert_eq!(heard.frames, 600);
    assert!(heard.sound_frames > 100, "sound frames {}", heard.sound_frames);
    assert_eq!(heard.sites.len(), 1, "sites {:?}", heard.sites.iter().map(|s| s.pc).collect::<Vec<_>>());
    let site = &heard.sites[0];
    assert_eq!(site.pc, 0x8014);
    assert!(site.edges > 1000 && site.frames_with_edges > 100);
    // every sample sits on one of the 8 pitches; each pitch is heard
    let mut seen = [0usize; 8];
    let mut off = 0;
    for p in &site.periods {
        let near = TUNE_HZ.iter().position(|hz| (p.period_t - 3.5e6 / hz).abs() / (3.5e6 / hz) <= 0.005);
        match near {
            Some(i) => seen[i] += 1,
            None => off += 1,
        }
        assert!((p.snap as usize) < site.snapshots.len());
    }
    assert!(seen.iter().filter(|&&n| n > 0).count() >= 8, "clusters {seen:?}, off {off}");
    assert!(off * 20 < site.periods.len(), "{off} of {} samples off the pitches", site.periods.len());
    // the B and C reloads (the DJNZ and DEC C loops' counters) are in the snapshots' reads
    for reader in [0x801Bu16, 0x801E] {
        assert!(site.snapshots.iter().any(|s| s.reads.iter().any(|r| r.0 == reader)), "reader {reader:04X}");
    }
}

#[test]
fn schedule_is_deterministic() {
    let (a, b, c) = (schedule(7), schedule(7), schedule(8));
    assert!((0..LISTEN_FRAMES).all(|f| a(f) == b(f)));
    assert!((2775..LISTEN_FRAMES).any(|f| a(f) != c(f)));
    // quiet start, then the start keys, one at a time
    assert!((0..2000).all(|f| a(f) == ([0; 8], 0)));
    assert!((2000..2005).all(|f| a(f).0[6] & 1 != 0), "ENTER held");
    assert!((2005..2155).all(|f| a(f) == ([0; 8], 0)));
}

#[test]
fn listen_is_fast() {
    let t = std::time::Instant::now();
    let heard = listen(&mut tune(), &schedule(1), LISTEN_FRAMES);
    let secs = t.elapsed().as_secs_f64();
    eprintln!("listen_is_fast: {secs:.2} s, {} periods", heard.sites.iter().map(|s| s.periods.len()).sum::<usize>());
    assert!(secs < 5.0, "{secs} s");
}

/// A run far longer than the 4,096 kept samples: they span the whole run, and each sample's
/// snapshot holds the reload values of its own note.
#[test]
fn long_run_keeps_the_whole_run() {
    let heard = listen(&mut tune(), &none(), LISTEN_FRAMES);
    let site = &heard.sites[0];
    assert!(site.periods.len() > 1000 && site.periods.len() <= 4096, "{}", site.periods.len());
    assert_eq!(site.snapshots.len(), site.periods.len());
    let (first, last) = (site.periods[0].at_frame, site.periods[site.periods.len() - 1].at_frame);
    assert!(first < 600 && last > LISTEN_FRAMES - 600, "samples span frames {first}..{last}");
    // the periods are in time order and spread: no gap over a tenth of the run
    assert!(site.periods.windows(2).all(|w| w[0].at_frame <= w[1].at_frame && w[1].at_frame - w[0].at_frame < 900));
    // per note: the (B, C) reload values the table read gave (read 1 of the two reloads)
    let mut per_note: Vec<std::collections::HashMap<(u8, u8), usize>> = vec![Default::default(); 8];
    let mut total = 0;
    for p in &site.periods {
        let snap = &site.snapshots[p.snap as usize];
        let value = |pc: u16| snap.reads.iter().find(|r| r.0 == pc && r.1 == 1).map(|r| r.3);
        let Some(i) = TUNE_HZ.iter().position(|hz| (p.period_t - 3.5e6 / hz).abs() / (3.5e6 / hz) <= 0.005) else { continue };
        let (Some(b), Some(c)) = (value(0x801B), value(0x801E)) else { continue };
        *per_note[i].entry((b, c)).or_default() += 1;
        total += 1;
    }
    assert!(total > 1000, "{total} samples with their reloads");
    let mut modes = Vec::new();
    for (i, n) in per_note.iter().enumerate() {
        let sum: usize = n.values().sum();
        let (mode, top) = n.iter().max_by_key(|e| *e.1).map(|e| (*e.0, *e.1)).expect("note heard");
        assert!(top * 10 >= sum * 9, "note {i}: reloads {n:?}");
        modes.push(mode);
    }
    modes.sort_unstable();
    modes.dedup();
    assert_eq!(modes.len(), 8, "each note has its own reloads");
}

/// The kept samples after thinning, with a cap small enough that it happens in several rounds:
/// they are every 2^r-th sample of the unthinned run, so evenly spread over the whole run, and
/// each one's snapshot still holds the reload values of its own note.
#[test]
fn thinning_keeps_even_samples_with_their_snapshots() {
    let none = none();
    let full = listen(&mut tune(), &none, LISTEN_FRAMES);
    let all = &full.sites[0].periods;
    let heard = listen_with(&mut tune(), &none, LISTEN_FRAMES, 200);
    let site = &heard.sites[0];
    let kept = &site.periods;
    assert!(all.len() > 2000 && kept.len() < 200 && kept.len() > 50, "{} of {}", kept.len(), all.len());
    // the stride, a power of two of at least 8 (three rounds of thinning)
    let stride = (3..=10)
        .map(|r| 1usize << r)
        .find(|&st| kept.iter().enumerate().all(|(i, k)| all.get(i * st).is_some_and(|a| a.period_t == k.period_t && a.at_frame == k.at_frame)))
        .expect("the kept samples are every 2^r-th of the full run");
    assert!(kept.len() * stride + stride > all.len(), "the kept samples reach the end ({} of {})", kept.len() * stride, all.len());
    assert!(kept[0].at_frame < 100 && kept[kept.len() - 1].at_frame > LISTEN_FRAMES - 1500);
    // paired with their own snapshots
    assert_eq!(site.snapshots.len(), kept.len());
    for (i, p) in kept.iter().enumerate() {
        assert_eq!(p.snap as usize, i);
        let reads = &site.snapshots[i].reads;
        let value = |pc: u16| reads.iter().find(|r| r.0 == pc && r.1 == 1).map(|r| r.3);
        // the unthinned run's snapshot of the same sample has the same reloads
        let full_snap = &full.sites[0].snapshots[all[i * stride].snap as usize].reads;
        let want = |pc: u16| full_snap.iter().find(|r| r.0 == pc && r.1 == 1).map(|r| r.3);
        assert!(value(0x801B).is_some() && value(0x801B) == want(0x801B) && value(0x801E) == want(0x801E), "sample {i}");
    }
}
