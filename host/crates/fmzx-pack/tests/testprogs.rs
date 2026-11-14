// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::manifest::parse_manifest;
use fmzx_pack::pack::build_pack;
use fmzx_pack::testprogs::{
    self, tape_tap, tune_sna, BORDER_EDGES_PER_FRAME, BORDER_TONE_HZ, NOISE_EDGES_PER_FRAME, OUT_SITES, TAPE_DONE_BORDER, TAPE_WAIT_FRAMES, TUNE_HZ, TUNE_NOTES_HZ,
    TUNE_NOTE_FRAMES, TUNE_START_FRAME,
};
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
    assert_eq!(written.len(), testprogs::PROGRAMS.len());
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
fn noise_edges_every_frame() {
    let mut m = machine(&testprogs::noise_sna());
    let mut gaps = std::collections::BTreeSet::new();
    for f in 1..=100 {
        m.run_frame();
        if f >= 2 {
            // every edge a speaker flip, all of them before the next interrupt
            let e = m.edges();
            assert_eq!(e.len(), NOISE_EDGES_PER_FRAME, "frame {f}");
            assert!(e.windows(2).all(|w| (w[0].level ^ w[1].level) & 2 != 0), "frame {f}: not all flips");
            gaps.extend(e.windows(2).map(|w| w[1].tstate - w[0].tstate));
        }
    }
    let (lo, hi) = (*gaps.first().unwrap(), *gaps.last().unwrap());
    println!("noise: edges {lo}-{hi} T apart, {} spacings", gaps.len());
    assert!(lo >= 121 && hi <= 240 && gaps.len() >= 8, "spacings {lo}-{hi} ({} of them)", gaps.len());
}

#[test]
fn out_sites_every_frame() {
    for changing in [false, true] {
        let mut m = machine(&testprogs::out_sites_sna(changing));
        let mut bytes = Vec::new();
        for f in 1..=20 {
            m.run_frame();
            if f >= 2 {
                // the four OUTs, each one a speaker flip, in order, every frame
                let pcs: Vec<u16> = m.out_sites().iter().map(|s| s.pc).collect();
                assert_eq!(pcs, OUT_SITES, "frame {f} (changing {changing})");
                let e = m.edges();
                assert_eq!(e.iter().map(|e| e.pc).collect::<Vec<_>>(), OUT_SITES, "frame {f} (changing {changing})");
                assert!(e.windows(2).all(|w| (w[0].level ^ w[1].level) & 2 != 0), "frame {f}: not all flips");
            }
            bytes.push(OUT_SITES.map(|a| m.peek(a + 3)));
        }
        // the byte beside each OUT counts the frames, or stays
        let moved = bytes.windows(2).all(|w| w[0].iter().zip(&w[1]).all(|(a, b)| b.wrapping_sub(*a) == 1));
        let still = bytes.iter().all(|b| *b == [0; 4]);
        assert!(if changing { moved } else { still }, "changing {changing}: the bytes beside the OUTs {bytes:?}");
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

/// The payload of the tape's last block: the screen image the code loads.
fn tape_image(tap: &[u8]) -> Vec<u8> {
    let mut at = 0;
    let mut last = &tap[..0];
    while at < tap.len() {
        let len = u16::from_le_bytes([tap[at], tap[at + 1]]) as usize;
        last = &tap[at + 2..at + 2 + len];
        at += 2 + len;
    }
    assert_eq!(last[0], 0xFF, "flag of the data block");
    last[1..last.len() - 1].to_vec()
}

fn screen_ram(m: &Machine) -> Vec<u8> {
    (0x4000..0x5B00u16).map(|a| m.peek(a)).collect()
}

#[test]
fn tape_loads_its_data_block_after_starting() {
    let tape = tape_tap();
    let image = tape_image(&tape);
    assert_eq!(image.len(), 6912);
    let mut m = Machine::new();
    m.tape_insert(&tape).unwrap();
    m.tape_autostart();
    let mut frames = 0;
    // the code starts when it sets the border to 1
    while m.border_line(100) != 1 {
        assert!(frames < 600, "the code did not start within 600 frames");
        m.run_frame();
        frames += 1;
    }
    let mut waited = 0;
    while m.border_line(100) == 1 {
        assert!(waited < 600, "the data block was not loaded within 600 frames");
        assert_ne!(screen_ram(&m), image, "image shown after {waited} frames");
        m.run_frame();
        waited += 1;
    }
    assert!(waited >= TAPE_WAIT_FRAMES - 5, "the code waited only {waited} frames");
    assert_eq!(m.border_line(100), TAPE_DONE_BORDER);
    assert_eq!(screen_ram(&m), image);
    assert_eq!(m.tape_pos() as usize, tape.len(), "the tape is at its end");
    for _ in 0..10 {
        m.run_frame();
    }
    assert_eq!(m.border_line(100), TAPE_DONE_BORDER, "stays done");
}

/// Renders `frames` frames of `tune.sna`: the samples of each frame, and the edges as absolute
/// T-states (69,888 per frame) with the frame they fall in and the level.
fn render_tune(frames: u32) -> (Vec<Vec<i16>>, Vec<(u64, u32, u8)>) {
    let mut m = machine(&tune_sna());
    let mut b = Beeper::new(RATE.0, RATE.1);
    let (mut audio, mut edges) = (Vec::new(), Vec::new());
    for f in 1..=frames {
        m.run_frame();
        if f == 1 {
            b.set_level(m.edges_start_level());
        }
        audio.push(b.render(m.edges()));
        edges.extend(m.edges().iter().map(|e| ((f as u64 - 1) * 69_888 + e.tstate as u64, f, e.level)));
    }
    (audio, edges)
}

#[test]
fn tune_pitches() {
    let pass = TUNE_NOTES_HZ.len() as u32 * TUNE_NOTE_FRAMES;
    let (audio, _) = render_tune(10 + 2 * pass + TUNE_START_FRAME);
    // frame f (1-based) is audio[f - 1]; note k of the run starts at TUNE_START_FRAME + 25k
    for k in 0..2 * TUNE_NOTES_HZ.len() as u32 {
        let i = k as usize % TUNE_NOTES_HZ.len();
        let first = TUNE_START_FRAME + k * TUNE_NOTE_FRAMES + 2;
        let last = TUNE_START_FRAME + (k + 1) * TUNE_NOTE_FRAMES - 2; // exclusive
        if first < 10 + TUNE_START_FRAME {
            continue; // settling: the first note of the first pass
        }
        let win: Vec<i16> = (first..last).flat_map(|f| audio[f as usize - 1].iter().copied()).collect();
        let crossings = win.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f64;
        let secs = win.len() as f64 * RATE.1 as f64 / RATE.0 as f64;
        let hz = crossings / 2.0 / secs;
        println!("note {i} (pass {}): {hz:.2} Hz, nominal {:.2}, recorded {:.2}", k / 8, TUNE_NOTES_HZ[i], TUNE_HZ[i]);
        assert!((hz - TUNE_NOTES_HZ[i]).abs() <= TUNE_NOTES_HZ[i] * 0.01, "note {i}: {hz:.2} Hz, nominal {}", TUNE_NOTES_HZ[i]);
        assert!((hz - TUNE_HZ[i]).abs() <= TUNE_HZ[i] * 0.002, "note {i}: {hz:.2} Hz, recorded {}", TUNE_HZ[i]);
    }
}

#[test]
fn tune_has_no_gaps() {
    let (_, edges) = render_tune(410);
    let half = |note: usize| 3_500_000.0 / (2.0 * TUNE_NOTES_HZ[note]);
    // note n of the run starts at frame TUNE_START_FRAME + 25 n (T 69,888 per frame)
    let note_of = |t: u64| ((t / 69_888 - (TUNE_START_FRAME as u64 - 1)) / TUNE_NOTE_FRAMES as u64) as usize;
    let from: Vec<&(u64, u32, u8)> = edges.iter().filter(|e| e.1 >= 10).collect();
    let (mut seams, mut worst_excess) = (0, f64::MIN);
    for w in from.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (gap, i) = ((b.0 - a.0) as f64, note_of(a.0) % 8);
        assert_ne!(a.2 & 2, b.2 & 2, "EAR does not toggle at frame {}", b.1);
        assert_eq!((a.2 | b.2) & 1, 0, "MIC set at frame {}", b.1);
        if note_of(a.0) == note_of(b.0) {
            // inside a note: the note's half-period, within the 0.5% of the table and contention
            assert!((gap - half(i)).abs() <= 0.01 * half(i), "note {i}: edges {gap} T apart at frame {}, half-period {:.0}", b.1, half(i));
        } else {
            // the seam: 0.9 to 1.2 half-periods of the note that ends, and at most 315 T more
            seams += 1;
            worst_excess = worst_excess.max(gap - half(i));
            assert!(gap >= 0.9 * half(i) && gap <= 1.2 * half(i), "seam after note {i}: {gap} T, half-period {:.0}", half(i));
            assert!(gap - half(i) <= 315.0, "seam after note {i}: {gap} T is {:.0} T over its half-period", gap - half(i));
        }
    }
    assert!(seams >= 14, "{seams} seams");
    println!("{seams} seams, the longest {worst_excess:.0} T over its half-period");
}
