// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::{load, Machine};

#[test]
fn runs_a_program_from_ram() {
    let mut m = Machine::new();
    // LD A,5; ADD A,3; LD (9000h),A; DI; HALT
    load(&mut m, 0x8000, &[0x3E, 0x05, 0xC6, 0x03, 0x32, 0x00, 0x90, 0xF3, 0x76]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.peek(0x9000), 8);
    assert_eq!(m.regs().af >> 8, 8);
}

#[test]
fn rom_is_read_only() {
    let mut m = Machine::new();
    let b = m.peek(0x0000);
    m.poke(0x0000, b ^ 0xFF);
    assert_eq!(m.peek(0x0000), b);
}

#[test]
fn power_on_starts_the_rom_at_zero() {
    assert_eq!(Machine::new().regs().pc, 0);
    assert_eq!(Machine::new().peek(0), 0xF3);
}

#[test]
fn frame_overshoot_is_carried() {
    let mut m = Machine::new();
    for _ in 0..3 {
        m.run_frame();
        assert!(m.frame_tstate() < 23); // longest Z80 instruction is 23 T
    }
    assert_eq!(m.frame_count(), 3);
}
