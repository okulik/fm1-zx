// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The code reader: from a speaker `OUT`, the music routine around it as a model: the loop's
//! family, its channels and which `OUT` sites belong to each, each channel's half period as a
//! formula of the routine's inputs, the note's timing, and the routine's signature.
//!
//! # How it reads
//!
//! The instructions reachable from the `OUT` within 512 bytes either side form a graph
//! (`cfg`). Counted delay loops (`DJNZ`, `DEC r; JR NZ`, `DEC rr; LD A,h; OR l; JR NZ`, `JP NZ`
//! too) become single steps of `per x n + konst` T-states. The pass is then walked from the
//! `OUT` until it comes back: a zero test of a counter (the idioms above, `JR`/`JP` `Z`/`NZ`
//! or `DJNZ`) either leaves the pass for the next note's setup (it goes on by the other side)
//! or, when its two sides meet again within a few straight instructions and one holds an `OUT`,
//! is a diamond: a channel's counter reload. Anything else in the pass is refused. The setup
//! paths from each zero test's exit back into the pass are followed too (calls stepped over).
//!
//! One symbolic run of the pass (`sym`) gives, for every register and fixed-address byte,
//! where its value came from and each of its bits; from that:
//!
//! - **Square**: no diamonds, every speaker `OUT` of the pass an edge (bit 4 changes at each over
//!   three passes, from either start level): one channel, its half period the pass over the
//!   number of `OUT`s. The delay counters' reloads must all count the same thing: one counter
//!   register fed from one source (else a duty cycle, refused); two registers fed from the same
//!   source is [`Refusal::Ambiguous`] (one channel toggled from two places, or two in unison?).
//! - **Slots**: no diamonds, each `OUT`'s bit 4 the top bit of its own 16-bit phase accumulator
//!   (`ADD HL,rr`, kept in memory or in HL): half period `32768 x pass / step`.
//! - **Flips**: every speaker `OUT` in a diamond's reload arm, each flipping one level all
//!   channels share (Unison style, but only balanced: both arms of every diamond the same time):
//!   half period `n x pass`, `n` the counter's reload.
//!
//! The pass time must be constant for Slots and Flips. The note's timing comes from the pass's
//! one zero test (see [`DurationModel`]); the music player's evidence (`table_walk`) is an input
//! the setup loaded through a pointer it advances.
//!
//! # What it refuses
//!
//! An undecoded instruction, a computed jump in the pass, a self-modification other than of an
//! immediate operand by the setup, any branch that is not one of the above, a call, return,
//! `HALT`, stack operation or store through a pointer in the pass, unbalanced diamonds, an
//! input the formulas cannot name, a setup path with a speaker `OUT` or that flips the level the
//! pass carries (a hidden channel), other code inside the pass's address range, a routine longer
//! than `SND_LEARNED_LEN_MAX`, more than three channels or 16 sites, and code or fixed-address
//! data in the contended range (4000h-7FFFh): its delays are fractions of a T-state per
//! instruction on average, which whole-T coefficients cannot carry.

mod cfg;
mod shape;
mod sym;

use crate::prog::reg;
use crate::z80::{decode, Flow, PortRef};
use std::collections::BTreeSet;
use zx_sys::zx_regs;

/// How a routine's channels make the speaker's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// One channel; every speaker `OUT` of the pass is an edge.
    Square,
    /// Each edge flips one channel's level; the speaker is their XOR.
    Flips,
    /// Each channel's `OUT` sets the speaker to that channel's level, once a pass.
    Slots,
}

/// A value the routine reads, as the bytecode's inputs (`crate::prog::Op`): the byte or word at
/// an offset from the anchor (`PeekRel`, `Peek16Rel`) or at an address (`Peek`, `Peek16`), the
/// byte at a register (8-12) plus a displacement (`PeekReg`), a register (`Reg`). Registers are
/// their values at the site's last edge (the `OUT`); memory is read at the frame's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Input {
    Rel(u16),
    Rel16(u16),
    Abs(u16),
    Abs16(u16),
    Indexed(u8, i8),
    Reg(u8),
}

impl Input {
    /// A 16-bit value (a word, or a register pair).
    pub fn wide(&self) -> bool {
        match self {
            Input::Rel16(_) | Input::Abs16(_) => true,
            Input::Reg(r) => matches!(r, 8..=13 | 15..=17),
            _ => false,
        }
    }

    /// The value: memory from `peek`, registers from `regs` (None for a register that is not
    /// one).
    pub fn read(&self, peek: &dyn Fn(u16) -> u8, anchor: u16, regs: &zx_regs) -> Option<u32> {
        let p16 = |a: u16| u32::from(peek(a)) | u32::from(peek(a.wrapping_add(1))) << 8;
        Some(match *self {
            Input::Rel(o) => peek(anchor.wrapping_add(o)).into(),
            Input::Rel16(o) => p16(anchor.wrapping_add(o)),
            Input::Abs(a) => peek(a).into(),
            Input::Abs16(a) => p16(a),
            Input::Indexed(r, d) if (8..=12).contains(&r) => peek((reg(regs, r)? as u16).wrapping_add(d as u16)).into(),
            Input::Indexed(..) => return None,
            Input::Reg(r) => reg(regs, r)?,
        })
    }

    /// As a count: 0 runs 256 times (65,536 for a 16-bit input).
    fn count(&self, peek: &dyn Fn(u16) -> u8, anchor: u16, regs: &zx_regs) -> Option<u32> {
        let v = self.read(peek, anchor, regs)?;
        Some(if v != 0 { v } else if self.wide() { 65536 } else { 256 })
    }
}

/// One part of a sum of T-states.
///
/// The input of `Lin` and `Prod` is a counter's reload: a count, 0 running 256 times (65,536 for
/// a 16-bit input); a program must map 0 so (`((x + 255) & 255) + 1`). `Recip`'s input is a
/// step: 0 is silence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Const(i64),
    Lin { coef_t: i64, x: Input },
    Prod { coef_t: i64, x: Input, y: Input },
    Recip { num_t: i64, x: Input },
}

/// One channel: its `OUT` sites and its half period (T-states between its toggles, a sum of
/// terms; a square's period is twice it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelModel {
    pub sites: Vec<u16>,
    pub half_period: Vec<Term>,
    /// The channel is silent when this input is 0 (none found by this reader yet).
    pub silent_if_zero: Option<Input>,
}

/// How long the note has held: its counter counts passes down from `counts_down_from` (the
/// value it had at the note's first edge), `per_unit_t` T-states each, so at an edge the note has
/// held `((counts_down_from - counter) mod 2^bits) x per_unit_t` T-states (bits: the counter's
/// width; a start of 0 counts 2^bits). For a 16-bit counter an `Indexed(r, d)` start is the
/// word at r + d. `counts_down_from` None would be a counter counting up from 0 (not found by
/// this reader yet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurationModel {
    pub counter: Input,
    pub per_unit_t: Vec<Term>,
    pub counts_down_from: Option<Input>,
}

impl DurationModel {
    /// T-states the note has held at the edge with registers `regs`.
    pub fn held(&self, peek: &dyn Fn(u16) -> u8, anchor: u16, regs: &zx_regs) -> Option<f64> {
        let wide = self.counter.wide();
        let now = self.counter.read(peek, anchor, regs)?;
        let start = match self.counts_down_from? {
            Input::Indexed(r, d) if wide => {
                let lo = Input::Indexed(r, d).read(peek, anchor, regs)?;
                lo | Input::Indexed(r, d.checked_add(1)?).read(peek, anchor, regs)? << 8
            }
            x => x.read(peek, anchor, regs)?,
        };
        let mask = if wide { 0xFFFF } else { 0xFF };
        Some(f64::from(start.wrapping_sub(now) & mask) * eval_terms(&self.per_unit_t, peek, anchor, regs)?)
    }
}

/// A routine read from its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineModel {
    /// The routine's first byte (the signature's start) and one past its last.
    pub anchor: u16,
    pub code_end: u16,
    pub family: Family,
    /// Ordered by their first site.
    pub channels: Vec<ChannelModel>,
    /// T-states of one pass when channels share a loop (Slots), else 0.
    pub pass_t: u16,
    /// Per channel: its `OUT`'s T-state in a pass, from channel 0's.
    pub pos: Vec<u16>,
    /// Each edge flips one channel's level (Flips).
    pub flips: bool,
    pub duration: Option<DurationModel>,
    /// Frame-end pcs where memory is not to be read (outside the loop): offset from the anchor
    /// and length, the range wrapping round.
    pub stale: Option<(u16, u16)>,
    /// The notes come from memory read through a pointer the routine advances.
    pub table_walk: bool,
    /// Offsets from the anchor of the operand bytes the routine rewrites, ascending.
    pub wild: Vec<u16>,
    /// Offsets of the absolute addresses (words) inside the routine, ascending.
    pub reloc: Vec<u16>,
}

/// Why a routine gets no model; the address is where the reader stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Undecoded(u16),
    ComputedJump(u16),
    SelfModUnmodelled(u16),
    UnknownShape(u16),
    Ambiguous(u16),
    NotSpeaker(u16),
}

/// The memory and the bytes the routine rewrites.
pub(super) struct Ctx<'a> {
    pub peek: &'a dyn Fn(u16) -> u8,
    pub wild: BTreeSet<u16>,
}

/// The sum `terms` with memory from `peek` and the registers `regs`; None when silent (a
/// `Recip` of 0) or an input cannot be read.
pub fn eval_terms(terms: &[Term], peek: &dyn Fn(u16) -> u8, anchor: u16, regs: &zx_regs) -> Option<f64> {
    let mut sum = 0.0;
    for t in terms {
        sum += match t {
            Term::Const(c) => *c as f64,
            Term::Lin { coef_t, x } => *coef_t as f64 * f64::from(x.count(peek, anchor, regs)?),
            Term::Prod { coef_t, x, y } => *coef_t as f64 * f64::from(x.count(peek, anchor, regs)?) * f64::from(y.count(peek, anchor, regs)?),
            Term::Recip { num_t, x } => match x.read(peek, anchor, regs)? {
                0 => return None,
                v => *num_t as f64 / f64::from(v),
            },
        };
    }
    Some(sum)
}

/// Reads the routine around the speaker `OUT` at `out_pc`.
pub fn read_engine(peek: impl Fn(u16) -> u8, out_pc: u16) -> Result<EngineModel, Refusal> {
    let peek: &dyn Fn(u16) -> u8 = &peek;
    let first = decode(peek, out_pc);
    match first.port_out {
        Some(PortRef::Imm(n)) if n & 1 == 0 => {}
        Some(PortRef::RegC) if first.flow == Flow::Next && first.t == first.t_taken => {}
        _ => return Err(Refusal::NotSpeaker(out_pc)),
    }
    let g = cfg::build(peek, out_pc);
    let items = cfg::walk(&g, peek, out_pc)?;
    shape::analyse(peek, &g, items, out_pc)
}

/// The candidate speaker `OUT`s in RAM (4000h-FFFFh; the ROM's routines are the built-in
/// entries'): `OUT (n),A` with an even `n`, and `OUT (C),r` where the straight code just before
/// (at most 16 bytes) sets C to an even value and nothing after changes it.
pub fn scan_out_sites(peek: impl Fn(u16) -> u8) -> Vec<u16> {
    let peek: &dyn Fn(u16) -> u8 = &peek;
    (0x4000..=0xFFFEu16)
        .filter(|&a| match (peek(a), peek(a + 1)) {
            (0xD3, n) => n & 1 == 0,
            (0xED, op) => op & 0xC7 == 0x41 && c_even_before(peek, a),
            _ => false,
        })
        .collect()
}

/// Whether some straight run of instructions ending at `a` sets C to an even value (`LD C,n`,
/// `LD BC,nn`) that nothing after it changes.
fn c_even_before(peek: &dyn Fn(u16) -> u8, a: u16) -> bool {
    (1..=16u16).filter_map(|back| a.checked_sub(back).filter(|&s| s >= 0x4000)).any(|start| {
        let mut pc = start;
        let mut c: Option<u8> = None;
        while pc < a {
            let i = decode(peek, pc);
            if i.flow != Flow::Next {
                return false;
            }
            match peek(pc) {
                0x0E | 0x01 if i.len == 2 + u8::from(peek(pc) == 0x01) => c = Some(peek(pc + 1)),
                _ if i.writes & 1 << 3 != 0 => c = None,
                _ => {}
            }
            pc = pc.saturating_add(u16::from(i.len));
        }
        pc == a && c.is_some_and(|c| c & 1 == 0)
    })
}
