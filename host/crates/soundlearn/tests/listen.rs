// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Listening to `games/test/tune.sna`: the speaker routine's period samples and snapshots, and
//! the generic input schedule.

use soundlearn::listen::{listen, schedule, Inputs, LISTEN_FRAMES};
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
