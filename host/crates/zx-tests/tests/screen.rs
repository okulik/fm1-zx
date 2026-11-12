// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::{load, screen_text, Machine};

/// A machine idling in `DI; JR $` at 0x8000 with one frame run, so the dirty
/// bits are clear and the ROM is out of the way.
fn idle() -> Machine {
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xF3, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    m.set_regs(&r);
    m.run_frame(); // the first frame reports every line dirty (one-shot redraw)
    m.run_frame();
    m
}

fn dirty_lines(m: &Machine) -> Vec<usize> {
    (0..192).filter(|&y| m.line_dirty(y)).collect()
}

#[test]
fn boots_to_the_copyright_line() {
    let mut m = Machine::new();
    for _ in 0..200 {
        m.run_frame();
    }
    assert_eq!(screen_text(&m, 23), "\u{a9} 1982 Sinclair Research Ltd");
}

#[test]
fn line_decode() {
    let mut m = idle();
    // y = 9: third-of 0, line-in-cell 1, cell row 1 -> 0x4000 + 0x100 + 0x20 + 8
    m.poke(0x4128, 0b1010_0000);
    // attribute: bright, paper 5, ink 2; cell row 1, column 8
    m.poke(0x5800 + 32 + 8, 0x40 | (5 << 3) | 2);
    assert_eq!(&m.screen_line(9)[64..68], &[10, 13, 10, 13]);
}

#[test]
fn flash_swaps_after_16_frames() {
    let mut m = idle();
    m.poke(0x4000, 0b1000_0000);
    m.poke(0x5800, 0x80 | (5 << 3) | 2);
    // the idle() frame counted as frame 1; phase is (count / 16) & 1
    while m.frame_count() < 16 {
        assert_eq!(m.screen_line(0)[..2], [2, 5], "frame {}", m.frame_count());
        m.run_frame();
    }
    while m.frame_count() < 32 {
        assert_eq!(m.screen_line(0)[..2], [5, 2], "frame {}", m.frame_count());
        m.run_frame();
    }
    assert_eq!(m.screen_line(0)[..2], [2, 5]);
}

#[test]
fn flash_flip_marks_flash_cells() {
    let mut m = idle();
    m.poke(0x5800 + 3 * 32, 0x80); // cell row 3 only
    while m.frame_count() < 15 {
        m.run_frame();
    }
    m.run_frame(); // ends with count 16: the phase flipped
    assert_eq!(dirty_lines(&m), (24..32).collect::<Vec<_>>());
    m.run_frame();
    assert!(dirty_lines(&m).is_empty());
}

#[test]
fn dirty_marks() {
    let m = Machine::new();
    assert_eq!(dirty_lines(&m).len(), 192, "all dirty after init");
    let mut m = idle();
    assert!(dirty_lines(&m).is_empty());

    m.poke(0x4128, 0xFF); // line 9
    assert_eq!(dirty_lines(&m), vec![9]);
    m.run_frame();
    assert!(dirty_lines(&m).is_empty());

    m.poke(0x4128, 0xFF); // same value
    m.poke(0x0000, 0x12); // ROM
    assert!(dirty_lines(&m).is_empty());

    m.poke(0x5800 + 5 * 32 + 1, 0x38); // cell row 5
    assert_eq!(dirty_lines(&m), (40..48).collect::<Vec<_>>());
    m.run_frame();
    assert!(dirty_lines(&m).is_empty());
}

#[test]
fn cpu_writes_mark_lines() {
    let mut m = Machine::new();
    m.run_frame(); // the one-shot full redraw
    // LD A,1; LD (4800h),A; LD (4800h),A; JR $ : line 64 once
    load(&mut m, 0x8000, &[0xF3, 0x3E, 0x01, 0x32, 0x00, 0x48, 0x32, 0x00, 0x48, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(dirty_lines(&m), vec![64]);
    m.run_frame();
    assert!(dirty_lines(&m).is_empty());
}

#[test]
fn first_frame_after_init_redraws_everything() {
    let mut m = Machine::new();
    assert_eq!(dirty_lines(&m).len(), 192, "dirty right after init");
    m.run_frame();
    assert_eq!(dirty_lines(&m).len(), 192, "the first frame reports all lines");
    m.run_frame();
    assert!(dirty_lines(&m).is_empty(), "then only real changes");
    m.poke(0x4000, 0xFF);
    assert_eq!(dirty_lines(&m), vec![0]);
}
