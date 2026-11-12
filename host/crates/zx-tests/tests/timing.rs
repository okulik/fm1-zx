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

    // SET 0,(IX+0) at 0x8000 (IX = 0x4000), Fuse's tables: read(EA) starts at
    // 14335 (+6), then 1 internal T at EA (14344: phase 1, +5), then write(EA)
    // at 14350 (phase 7, +0): 23 + 6 + 5 = 34.
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xDD, 0xCB, 0x00, 0xC6]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.ix = 0x4000;
    r.sp = 0xFF00;
    m.set_regs(&r);
    m.test_set_tstate(14335 - 16);
    assert_eq!(m.test_step(), 34);
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
    // Independent count: DI; loop: INC HL; JP loop (6 + 10 = 16 T per pass) in
    // uncontended RAM. Passes are summed from HL's wrapping delta per frame.
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xF3, 0x23, 0xC3, 0x01, 0x80]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.hl = 0;
    r.sp = 0xFF00;
    m.set_regs(&r);
    let mut passes: u64 = 0;
    let mut last = 0u16;
    for _ in 0..1000 {
        m.run_frame();
        let hl = m.regs().hl;
        passes += hl.wrapping_sub(last) as u64;
        last = hl;
    }
    let expected = 1000 * 69888 + m.frame_tstate() as u64;
    let measured = passes * 16;
    assert!(measured.abs_diff(expected) <= 16, "{measured} vs {expected}");
}

fn interrupt_count_with_ei_after_nops(nops: usize) -> u8 {
    // IM 2, handler increments 0xA000. Main: `nops` NOPs, EI, JR $; the machine
    // starts with interrupts off, so INT is only seen if still asserted after EI.
    let mut m = Machine::new();
    load(&mut m, 0x80FF, &[0x00, 0x90]);
    load(&mut m, 0x9000, &[0xF5, 0x3A, 0x00, 0xA0, 0x3C, 0x32, 0x00, 0xA0, 0xF1, 0xED, 0x4D]);
    let mut prog = vec![0x00; nops];
    prog.extend([0xFB, 0x18, 0xFE]);
    load(&mut m, 0x8000, &prog);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFFF0;
    r.i = 0x80;
    r.im = 2;
    m.set_regs(&r);
    m.run_frame();
    m.peek(0xA000)
}

#[test]
fn interrupt_line_drops_after_32_tstates() {
    // 2 NOPs then EI (T=8..12), JR (..24): INT is still asserted, so it is taken.
    assert_eq!(interrupt_count_with_ei_after_nops(2), 1);
    // 10 NOPs then EI at T=40: INT was released at T=32, so no interrupt.
    assert_eq!(interrupt_count_with_ei_after_nops(10), 0);
}

#[test]
fn io_contention_cases() {
    // IN A,(n) at 0x8000 (uncontended fetches, 7 T) started at T=14328 so its I/O cycle starts at 14335.
    assert_eq!(cycles_of_in_at(0x40, 0xFE), 17); // contended high byte, ULA port: C:1, C:3
    assert_eq!(cycles_of_in_at(0x40, 0xFF), 23); // contended high byte, odd port: C:1 x4
    assert_eq!(cycles_of_in_at(0x80, 0xFE), 16); // uncontended high byte, ULA port: N:1, C:3
    assert_eq!(cycles_of_in_at(0x80, 0xFF), 11); // uncontended, odd port: N:4
}

#[test]
fn jr_in_contended_ram_contends_internal_cycles() {
    // Fuse JR e: fetch pc (4), read pc+1 (3), pc+1:1 x5, all at contended 0x4000.
    // From T=14335: fetch +6 -> 14345; operand at 14345 (phase 2: +4) -> 14352;
    // internal T at 14352 (+5), 14358 (+0), 14359 (+6), 14366 (+0), 14367 (+6)
    // -> 14374. Total 39 = 12 + 6 + 4 + 5 + 6 + 6.
    let mut m = Machine::new();
    load(&mut m, 0x4000, &[0x18, 0x00]);
    let mut r = m.regs();
    r.pc = 0x4000;
    m.set_regs(&r);
    m.test_set_tstate(14335);
    assert_eq!(m.test_step(), 39);
}

#[test]
fn ldir_write_to_contended_ram_contends_de_cycles() {
    // Fuse LDIR repeat: 4,4 fetches, read hl (3), write de (3), de:1 x2, de:1 x5,
    // here with uncontended code/source and DE = 0x4000, write starting at 14335.
    // Write +6 -> 14344; seven internal T at 14344 (+5), 14350 (+0), 14351 (+6),
    // 14358 (+0), 14359 (+6), 14366 (+0), 14367 (+6) -> 14374. Start 14324: 50 T.
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xED, 0xB0]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.hl = 0x8100;
    r.de = 0x4000;
    r.bc = 2;
    m.set_regs(&r);
    m.test_set_tstate(14335 - 11);
    assert_eq!(m.test_step(), 50);
}
