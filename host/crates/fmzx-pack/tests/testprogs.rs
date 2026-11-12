// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::manifest::parse_manifest;
use fmzx_pack::pack::build_pack;
use fmzx_pack::testprogs::{self, BORDER_EDGES_PER_FRAME, BORDER_TONE_HZ};
use std::path::{Path, PathBuf};
use zx_sys::{Beeper, Machine};

const PACK_BUDGET: usize = 393_216;
const RATE: (u32, u32) = (750_000, 17);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn machine(sna: &[u8]) -> Machine {
    let mut m = Machine::new();
    m.load_sna(sna).expect("loads");
    m
}

#[test]
fn testprogs_reproducible() {
    let dir = std::env::temp_dir().join(format!("fmzx-testprogs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let written = testprogs::write_all(&dir).unwrap();
    assert_eq!(written.len(), 2);
    for (name, make) in testprogs::PROGRAMS {
        let fresh = make();
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), fresh, "{name} as written");
        let committed = std::fs::read(repo().join("games/test").join(name)).unwrap();
        assert!(committed == fresh, "games/test/{name} differs from a fresh `fmzx-pack testprogs`");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scroll_dirties_every_line() {
    let mut m = machine(&testprogs::scroll_sna());
    let mut moved = 0;
    for f in 1..=60 {
        let before: Vec<[u8; 256]> = (0..192).map(|y| m.screen_line(y)).collect();
        m.run_frame();
        if f >= 10 {
            for (y, old) in before.iter().enumerate() {
                assert!(m.line_dirty(y), "frame {f}: line {y} not dirty");
                if m.screen_line(y) != *old {
                    moved += 1;
                }
            }
            for line in 0..312 {
                assert_eq!(m.border_line(line), 1, "frame {f}: border on line {line}");
            }
        }
    }
    assert_eq!(moved, 51 * 192, "every line's pixels change every frame");
}

/// Steps the program from just after its HALT (where the interrupt handler returns) back to
/// the HALT, starting at frame T-state `t0`; returns the T-state the HALT starts at.
fn run_to_halt(sna: &[u8], resume: u16, halt: u16, t0: u32) -> u32 {
    let mut m = machine(sna);
    for _ in 0..5 {
        m.run_frame();
    }
    let mut r = m.regs();
    r.pc = resume;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.test_set_tstate(t0);
    let mut t = t0;
    while m.regs().pc != halt {
        t += m.test_step();
        assert!(t < 80_000, "never reached the HALT");
    }
    t
}

#[test]
fn scroll_fits_well_inside_a_frame() {
    // 32 T-states: the IM 2 response and the handler's RET after the interrupt at T 0.
    let t = run_to_halt(&testprogs::scroll_sna(), testprogs::RESUME, testprogs::HALT, 32);
    println!("scroll: HALT at T {t}");
    assert!(t < 60_000, "scroll work ends at T {t}");
}

#[test]
fn border_reaches_halt_before_the_next_interrupt() {
    let t = run_to_halt(&testprogs::border_sna(), testprogs::RESUME, testprogs::HALT, 32);
    println!("border: HALT at T {t}");
    assert!(t < 69_888, "border loop ends at T {t}");
}

#[test]
fn border_stripes_and_tone() {
    let mut m = machine(&testprogs::border_sna());
    for f in 1..=60 {
        m.run_frame();
        if f == 20 {
            let changes = (41..=279).filter(|&l| m.border_line(l) != m.border_line(l - 1)).count();
            assert!(changes >= 8, "frame 20: {changes} colour changes on lines 40-279");
            let colours: std::collections::BTreeSet<u8> = (40..=279).map(|l| m.border_line(l)).collect();
            assert_eq!(colours.len(), 8, "all eight colours");
        }
        if f >= 10 {
            let n = m.edges().len();
            assert!(n.abs_diff(BORDER_EDGES_PER_FRAME) <= 1, "frame {f}: {n} edges");
            // one toggle every 8 lines of 224 T-states: no contention slips into the loop
            for w in m.edges().windows(2) {
                assert_eq!(w[1].tstate - w[0].tstate, 8 * 224, "frame {f}: edge spacing");
            }
        }
    }
}

#[test]
fn border_tone_frequency() {
    let mut m = machine(&testprogs::border_sna());
    let mut b = Beeper::new(RATE.0, RATE.1);
    // 10 frames to settle the program and the renderer's DC blocker, then 1 s (50 frames)
    let mut out = Vec::new();
    for f in 1..=60 {
        m.run_frame();
        if f == 1 {
            b.set_level(m.edges_start_level());
        }
        let s = b.render(m.edges());
        if f > 10 {
            out.extend(s);
        }
    }
    let crossings = out.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f64;
    let secs = out.len() as f64 * RATE.1 as f64 / RATE.0 as f64;
    let hz = crossings / 2.0 / secs;
    println!("border tone: {hz:.2} Hz ({crossings} zero crossings in {secs:.4} s)");
    assert!((hz - BORDER_TONE_HZ).abs() <= BORDER_TONE_HZ * 0.01, "tone {hz:.2} Hz, recorded {BORDER_TONE_HZ} Hz");
}

#[test]
fn manifest_lists_the_test_programs_first() {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).unwrap();
    let m = parse_manifest(&text).unwrap();
    assert_eq!(m.game[0].id, "scroll");
    assert_eq!(m.game[0].file, "test/scroll.sna");
    assert_eq!(m.game[1].id, "border");
    assert_eq!(m.game[1].file, "test/border.sna");
    assert!(m.game.len() > 2, "and at least one game");
}

#[test]
fn pack_fits_budget() {
    let pack = build_pack(&repo().join("games/games.toml")).unwrap();
    println!("pack: {} bytes", pack.len());
    assert!(pack.len() <= PACK_BUDGET, "pack is {} bytes, budget {PACK_BUDGET}", pack.len());
}
