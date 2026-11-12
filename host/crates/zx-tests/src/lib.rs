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

/// Executes the instruction bytes at 0x8000 with `b`, `c`, `a` loaded and
/// interrupts off, starting at frame T-state `t`. Returns the machine.
pub fn run_at(m: &mut Machine, code: &[u8], a: u8, b: u8, c: u8, t: u32) {
    load(m, 0x8000, code);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.af = (r.af & 0x00FF) | ((a as u16) << 8);
    r.bc = ((b as u16) << 8) | c as u16;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.test_set_tstate(t);
    m.test_step();
}

/// A as left by the last instruction.
pub fn reg_a(m: &Machine) -> u8 {
    (m.regs().af >> 8) as u8
}

/// Text of character row `row` (0-23): each 8x8 cell is matched against the ROM
/// font (0x3D00, chars 0x20-0x7F, 0x7F is the copyright sign) by bitmap alone;
/// `?` when nothing matches. Trailing spaces are trimmed.
pub fn screen_text(m: &Machine, row: usize) -> String {
    let lines: Vec<[u8; 256]> = (0..8).map(|l| m.screen_line(row * 8 + l)).collect();
    let mut text = String::new();
    for col in 0..32 {
        // The cell's two colours are unknown (the ROM prints black on white),
        // so accept the bitmap with either colour as the set bits.
        let min = lines.iter().flat_map(|l| l[col * 8..col * 8 + 8].iter().copied()).min().unwrap_or(0);
        let pattern = |set_is_min: bool| -> Vec<u8> {
            lines
                .iter()
                .map(|l| {
                    let px = &l[col * 8..col * 8 + 8];
                    px.iter().fold(0u8, |b, &p| (b << 1) | u8::from((p == min) == set_is_min))
                })
                .collect()
        };
        let (a, b) = (pattern(false), pattern(true));
        let found = (0x20u16..=0x7F).find(|&c| {
            let base = 0x3D00 + (c - 0x20) * 8;
            (0..8).all(|i| m.peek(base + i) == a[i as usize])
                || (0..8).all(|i| m.peek(base + i) == b[i as usize])
        });
        text.push(match found {
            Some(0x7F) => '\u{a9}',
            Some(c) => c as u8 as char,
            None => '?',
        });
    }
    text.trim_end().to_string()
}
