// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared fixtures for the machine tests.

pub use zx_sys::Machine;

/// Pokes `bytes` into the machine starting at `addr`.
pub fn load(m: &mut Machine, addr: u16, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        m.poke(addr.wrapping_add(i as u16), *b);
    }
}

pub use zx_sys::contention;

fn run_one(m: &mut Machine, pc: u16, a: u8, t: u32) -> u32 {
    let mut r = m.regs();
    r.pc = pc;
    r.af = (r.af & 0x00FF) | ((a as u16) << 8);
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.test_set_tstate(t);
    m.test_step()
}

/// T-states of a NOP at `addr` fetched at frame T-state `t`.
pub fn cycles_of_nop_at(addr: u16, t: u32) -> u32 {
    let mut m = Machine::new();
    load(&mut m, addr, &[0x00]);
    run_one(&mut m, addr, 0, t)
}

/// T-states of `IN A,(n)` at 0x8000 (A = `a_high`) whose I/O cycle starts at
/// T-state 14335.
pub fn cycles_of_in_at(a_high: u8, n: u8) -> u32 {
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xDB, n]);
    run_one(&mut m, 0x8000, a_high, 14335 - 7)
}

/// Runs `frames` frames of a program that idles with interrupts enabled
/// (or disabled, when `di`) under IM 2; the handler increments a RAM byte.
/// Returns the byte.
pub fn count_interrupts(frames: u32, di: bool) -> u8 {
    let mut m = Machine::new();
    // vector table: I = 0x80, bus value 0xFF -> word at 0x80FF
    load(&mut m, 0x80FF, &[0x00, 0x90]);
    // handler: PUSH AF; LD A,(A000h); INC A; LD (A000h),A; POP AF; EI; RETI
    load(
        &mut m,
        0x9000,
        &[0xF5, 0x3A, 0x00, 0xA0, 0x3C, 0x32, 0x00, 0xA0, 0xF1, 0xFB, 0xED, 0x4D],
    );
    // main: EI (or DI); JR $
    load(&mut m, 0x8000, &[if di { 0xF3 } else { 0xFB }, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFFF0;
    r.i = 0x80;
    r.im = 2;
    m.set_regs(&r);
    for _ in 0..frames {
        m.run_frame();
    }
    m.peek(0xA000)
}

/// Frame count times the frame length, plus the T-states into the current frame.
pub fn total_tstates(m: &Machine) -> u64 {
    m.frame_count() as u64 * zx_sys::ZX_FRAME_TSTATES as u64 + m.frame_tstate() as u64
}
