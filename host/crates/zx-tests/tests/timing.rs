// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

#[test]
fn contention_pattern() {
    let expect = [6, 5, 4, 3, 2, 1, 0, 0];
    for i in 0..8 {
        assert_eq!(contention(14335 + i), expect[i as usize]);
    }
    assert_eq!(contention(14335 + 128), 0); // past the line's 128 contended T
    assert_eq!(contention(14335 + 224), 6); // next line
    assert_eq!(contention(14334), 0);
    assert_eq!(contention(14335 + 192 * 224), 0); // after the paper
}

#[test]
fn contention_matches_reference_for_every_t_state() {
    for t in 0..69888u32 {
        let r = t.wrapping_sub(14335);
        let want = if t >= 14335 && r < 192 * 224 && r % 224 < 128 {
            [6, 5, 4, 3, 2, 1, 0, 0][(r % 8) as usize]
        } else {
            0
        };
        assert_eq!(contention(t), want, "t = {t}");
    }
}

#[test]
fn fetch_from_contended_ram_is_delayed() {
    // NOP at 0x4000 fetched at T=14335 takes 6 + 4; the same NOP at 0x8000 takes 4.
    assert_eq!(cycles_of_nop_at(0x4000, 14335), 10);
    assert_eq!(cycles_of_nop_at(0x8000, 14335), 4);
}

#[test]
fn contention_applies_at_each_m_cycle_start() {
    // LD (HL),A at 0x8000 (HL = 0x4000): the write M-cycle starts 4 T in.
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0x77]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.hl = 0x4000;
    r.sp = 0xFF00;
    m.set_regs(&r);
    m.test_set_tstate(14335 - 4);
    assert_eq!(m.test_step(), 7 + 6);

    // SET 0,(IX+0) at 0x8000 (IX = 0x4000): read(EA) starts 16 T in and is
    // delayed 6; the write(EA) then starts at 14335 + 6 + 4 (phase 2, +4).
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xDD, 0xCB, 0x00, 0xC6]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.ix = 0x4000;
    r.sp = 0xFF00;
    m.set_regs(&r);
    m.test_set_tstate(14335 - 16);
    assert_eq!(m.test_step(), 23 + 6 + 4);
}

#[test]
fn instructions_chain_with_contention() {
    // Two NOPs at 0x4000: 14335 -> +10, the second starts at 14345 (phase 10 -> 4).
    let mut m = Machine::new();
    let mut r = m.regs();
    r.pc = 0x4000;
    m.set_regs(&r);
    m.test_set_tstate(14335);
    assert_eq!(m.test_step(), 10);
    assert_eq!(m.test_step(), 4 + 4); // phase 10: 14345 % 8 pattern index 2 -> 4
}

#[test]
fn one_interrupt_per_frame() {
    assert_eq!(count_interrupts(10, false), 10);
    assert_eq!(count_interrupts(10, true), 0);
}

#[test]
fn no_drift_over_1000_frames() {
    let mut m = Machine::new();
    let start = total_tstates(&m);
    for _ in 0..1000 {
        m.run_frame();
    }
    assert_eq!(total_tstates(&m) - start, 1000 * 69888 + m.frame_tstate() as u64);
}

#[test]
fn io_contention_cases() {
    // IN A,(n) at 0x8000 (uncontended fetches, 7 T) started at T=14328 so its I/O cycle starts at 14335.
    assert_eq!(cycles_of_in_at(0x40, 0xFE), 17); // contended high byte, ULA port: C:1, C:3
    assert_eq!(cycles_of_in_at(0x40, 0xFF), 23); // contended high byte, odd port: C:1 x4
    assert_eq!(cycles_of_in_at(0x80, 0xFE), 16); // uncontended high byte, ULA port: N:1, C:3
    assert_eq!(cycles_of_in_at(0x80, 0xFF), 11); // uncontended, odd port: N:4
}
