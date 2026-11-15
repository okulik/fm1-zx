// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A learned entry's programs: bytecode for a stack machine of `u32` values, at most 8 deep and
//! [`SND_PROG_MAX`](zx_sys::SND_PROG_MAX) bytes long (the opcode table is in
//! `core/sound/learned.c`). [`eval_prog`] is the reference evaluator the C interpreter is tested
//! against.

use zx_sys::zx_regs;

/// Most values on the stack.
const DEPTH: usize = 8;

/// One instruction. Registers are numbered 0 A, 1 F, 2 B, 3 C, 4 D, 5 E, 6 H, 7 L, 8 BC, 9 DE,
/// 10 HL, 11 IX, 12 IY, 13 SP, 14 A', 15 BC', 16 DE', 17 HL'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Push a constant.
    K(u32),
    /// Push the byte at anchor + o.
    PeekRel(u16),
    /// Push the little-endian word at anchor + o.
    Peek16Rel(u16),
    /// Push the byte at an address.
    Peek(u16),
    /// Push the word at an address.
    Peek16(u16),
    /// Push the byte at register r (8-12: BC, DE, HL, IX, IY) + d.
    PeekReg(u8, i8),
    /// Push register r (0-17).
    Reg(u8),
    Add,
    /// Wrapping.
    Sub,
    /// Wrapping.
    Mul,
    /// Unsigned; a zero divisor ends the program as silent.
    Div,
    And,
    /// Shift right by 0-31.
    Shr(u8),
    /// Shift left by 0-31.
    Shl(u8),
    /// Pop x; x = 0 ends the program as silent.
    ZeroSilent,
}

/// The bytecode of `ops`; a constant takes the shortest of its three forms.
pub fn encode_prog(ops: &[Op]) -> Vec<u8> {
    let mut b = Vec::new();
    for &op in ops {
        match op {
            Op::K(k) if k <= 0xFF => b.extend([0x01, k as u8]),
            Op::K(k) if k <= 0xFFFF => {
                b.push(0x02);
                b.extend((k as u16).to_le_bytes());
            }
            Op::K(k) => {
                b.push(0x03);
                b.extend(k.to_le_bytes());
            }
            Op::PeekRel(o) => b.extend([0x10, o as u8, (o >> 8) as u8]),
            Op::Peek16Rel(o) => b.extend([0x11, o as u8, (o >> 8) as u8]),
            Op::Peek(a) => b.extend([0x12, a as u8, (a >> 8) as u8]),
            Op::Peek16(a) => b.extend([0x13, a as u8, (a >> 8) as u8]),
            Op::PeekReg(r, d) => b.extend([0x14, r, d as u8]),
            Op::Reg(r) => b.extend([0x20, r]),
            Op::Add => b.push(0x30),
            Op::Sub => b.push(0x31),
            Op::Mul => b.push(0x32),
            Op::Div => b.push(0x33),
            Op::And => b.push(0x34),
            Op::Shr(s) => b.extend([0x35, s]),
            Op::Shl(s) => b.extend([0x36, s]),
            Op::ZeroSilent => b.push(0x40),
        }
    }
    b
}

/// Register r of `ctx`, None past 17.
fn reg(ctx: &zx_regs, r: u8) -> Option<u32> {
    let hi = |w: u16| u32::from(w >> 8);
    let lo = |w: u16| u32::from(w & 0xFF);
    Some(match r {
        0 => hi(ctx.af),
        1 => lo(ctx.af),
        2 => hi(ctx.bc),
        3 => lo(ctx.bc),
        4 => hi(ctx.de),
        5 => lo(ctx.de),
        6 => hi(ctx.hl),
        7 => lo(ctx.hl),
        8 => ctx.bc.into(),
        9 => ctx.de.into(),
        10 => ctx.hl.into(),
        11 => ctx.ix.into(),
        12 => ctx.iy.into(),
        13 => ctx.sp.into(),
        14 => hi(ctx.af_),
        15 => ctx.bc_.into(),
        16 => ctx.de_.into(),
        17 => ctx.hl_.into(),
        _ => return None,
    })
}

/// Runs `ops` with memory `peek`, the anchor and the registers `ctx`: None if the program is not
/// valid (as far as it ran: a stack underflow or overflow, an operand out of range, other than one
/// value at the end, or longer than `SND_PROG_MAX` bytes), Some(None) silent, else Some(result).
pub fn eval_prog(ops: &[Op], peek: impl Fn(u16) -> u8, anchor: u16, ctx: &zx_regs) -> Option<Option<u32>> {
    if encode_prog(ops).len() > zx_sys::SND_PROG_MAX {
        return None;
    }
    let peek16 = |a: u16| u32::from(peek(a)) | u32::from(peek(a.wrapping_add(1))) << 8;
    let mut s: Vec<u32> = Vec::with_capacity(DEPTH);
    for &op in ops {
        let x = match op {
            Op::K(k) => k,
            Op::PeekRel(o) => peek(anchor.wrapping_add(o)).into(),
            Op::Peek16Rel(o) => peek16(anchor.wrapping_add(o)),
            Op::Peek(a) => peek(a).into(),
            Op::Peek16(a) => peek16(a),
            Op::PeekReg(r, d) => {
                if !(8..=12).contains(&r) {
                    return None;
                }
                peek((reg(ctx, r)? as u16).wrapping_add(d as u16)).into()
            }
            Op::Reg(r) => reg(ctx, r)?,
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::And => {
                let y = s.pop()?;
                let x = s.pop()?;
                s.push(match op {
                    Op::Add => x.wrapping_add(y),
                    Op::Sub => x.wrapping_sub(y),
                    Op::Mul => x.wrapping_mul(y),
                    Op::And => x & y,
                    _ if y == 0 => return Some(None),
                    _ => x / y,
                });
                continue;
            }
            Op::Shr(n) | Op::Shl(n) => {
                let x = s.pop().filter(|_| n < 32)?;
                s.push(if matches!(op, Op::Shr(_)) { x >> n } else { x << n });
                continue;
            }
            Op::ZeroSilent => {
                if s.pop()? == 0 {
                    return Some(None);
                }
                continue;
            }
        };
        if s.len() == DEPTH {
            return None;
        }
        s.push(x);
    }
    (s.len() == 1).then(|| Some(s[0]))
}
