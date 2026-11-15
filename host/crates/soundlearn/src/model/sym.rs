// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The symbolic machine the code reader runs a loop's pass (or a setup path) on: for every 8-bit
//! register and every byte stored to a fixed address, where its value came from ([`Src`]) and
//! what each of its bits is ([`Bit`]); for every register pair, which value it points from
//! (`base`) and how far it moved since (`off`), so a pointer advanced by `INC HL` still knows
//! the bytes it loaded. Instructions it does not model make what they write unknown.

use super::Ctx;
use crate::z80::{Flow, Insn, MemRef};
use std::collections::BTreeMap;

/// The 8-bit registers: A F B C D E H L, the alternate set, IXH IXL IYH IYL.
pub(super) const NSLOT: usize = 20;
pub(super) const A: usize = 0;
pub(super) const F: usize = 1;
const IXH: usize = 16;

/// The register pairs (numbers of [`crate::prog`]: 8 BC, 9 DE, 10 HL, 11 IX, 12 IY, 15 BC',
/// 16 DE', 17 HL') by index, and their (high, low) slots.
pub(super) const PAIRS: [u8; 8] = [8, 9, 10, 11, 12, 15, 16, 17];
const PAIR_SLOTS: [(usize, usize); 8] = [(2, 3), (4, 5), (6, 7), (16, 17), (18, 19), (10, 11), (12, 13), (14, 15)];

/// The index into [`PAIRS`] of pair number `p`.
pub(super) fn pair_index(p: u8) -> Option<usize> {
    PAIRS.iter().position(|&q| q == p)
}

/// Register number (as [`crate::prog`]) of slot `s`, for the ones an input can name.
pub(super) fn slot_reg(s: usize) -> Option<u8> {
    match s {
        0 => Some(0),
        2..=7 => Some(s as u8),
        8 => Some(14),
        _ => None,
    }
}

/// The slots register number `r` (as the decoder's register bits) covers.
fn reg_slots(r: u8) -> Vec<usize> {
    match r {
        0..=7 => vec![r as usize],
        14 => vec![8, 9],
        _ => pair_index(r).map(|i| vec![PAIR_SLOTS[i].0, PAIR_SLOTS[i].1]).unwrap_or_default(),
    }
}

/// A place a bit lives: a register slot or a byte of memory at a fixed address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Loc {
    R(u8),
    M(u16),
}

/// One bit: known, unknown, the bit of a place as it was at the start, or the top bit of a
/// phase accumulator; each possibly inverted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bit {
    Z,
    O,
    X,
    E { loc: Loc, bit: u8, inv: bool },
    Acc { id: u16, inv: bool },
}

impl Bit {
    pub(super) fn inv(self) -> Bit {
        match self {
            Bit::Z => Bit::O,
            Bit::O => Bit::Z,
            Bit::X => Bit::X,
            Bit::E { loc, bit, inv } => Bit::E { loc, bit, inv: !inv },
            Bit::Acc { id, inv } => Bit::Acc { id, inv: !inv },
        }
    }
    fn and(a: Bit, b: Bit) -> Bit {
        match (a, b) {
            (Bit::Z, _) | (_, Bit::Z) => Bit::Z,
            (Bit::O, x) | (x, Bit::O) => x,
            (x, y) if x == y => x,
            _ => Bit::X,
        }
    }
    fn or(a: Bit, b: Bit) -> Bit {
        match (a, b) {
            (Bit::O, _) | (_, Bit::O) => Bit::O,
            (Bit::Z, x) | (x, Bit::Z) => x,
            (x, y) if x == y => x,
            _ => Bit::X,
        }
    }
    fn xor(a: Bit, b: Bit) -> Bit {
        match (a, b) {
            (Bit::Z, x) | (x, Bit::Z) => x,
            (Bit::O, x) | (x, Bit::O) => x.inv(),
            (Bit::X, _) | (_, Bit::X) => Bit::X,
            (x, y) if x == y => Bit::Z,
            (x, y) if x == y.inv() => Bit::O,
            _ => Bit::X,
        }
    }
}

/// Where a byte's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Src {
    Unknown,
    /// Computed, known.
    Const(u8),
    /// The instruction operand byte at this address.
    Imm(u16),
    /// The byte at a fixed address.
    Mem(u16),
    /// The byte at pair `pair` + `d`, `d` counted from the pair's value `base` (see [`State`]).
    Idx { pair: u8, base: u32, d: i32 },
    /// The slot's value at the start.
    Entry(u8),
    /// A byte of phase accumulator `id`: its high byte if true.
    Acc(u16, bool),
}

/// Where a 16-bit value came from (a pair's two [`Src`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum W {
    Unknown,
    Const(u16),
    Imm(u16),
    Mem(u16),
    Idx { pair: u8, base: u32, d: i32 },
    /// The pair's value at the start (pair number).
    Entry(u8),
    Acc(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Val {
    pub src: Src,
    pub bits: [Bit; 8],
}

impl Val {
    const UNKNOWN: Val = Val { src: Src::Unknown, bits: [Bit::X; 8] };
    fn known(src: Src, v: u8) -> Val {
        Val { src, bits: std::array::from_fn(|i| if v >> i & 1 != 0 { Bit::O } else { Bit::Z }) }
    }
    fn at(loc: Loc, src: Src) -> Val {
        Val { src, bits: std::array::from_fn(|i| Bit::E { loc, bit: i as u8, inv: false }) }
    }
}

/// A phase accumulator: a 16-bit value that a step is added to every pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Acc {
    /// Where it lives: `Mem(a)` or `Entry(pair)`.
    pub base: W,
    pub step: W,
    /// Stores of it back to `base`.
    pub stored: u32,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    pub v: [Val; NSLOT],
    pub base: [u32; 8],
    pub off: [i32; 8],
    pub mem: BTreeMap<u16, Val>,
    /// A write through a pointer happened: memory not in `mem` is unknown.
    pub mem_lost: bool,
    /// Pairs (by index) moved by a constant: `INC`, `DEC`, `ADD` of a known value.
    pub advanced: u8,
    pub accs: Vec<Acc>,
    next: u32,
}

impl State {
    /// Every register as at the start: `Entry`, its bits its own; the pairs' bases 0-7.
    pub(super) fn entry() -> State {
        State {
            v: std::array::from_fn(|s| Val::at(Loc::R(s as u8), Src::Entry(s as u8))),
            base: std::array::from_fn(|i| i as u32),
            off: [0; 8],
            mem: BTreeMap::new(),
            mem_lost: false,
            advanced: 0,
            accs: Vec::new(),
            next: 8,
        }
    }

    /// Nothing known (a setup path: what the loop left is not tracked).
    pub(super) fn fresh() -> State {
        let mut s = State::entry();
        s.v = [Val::UNKNOWN; NSLOT];
        s.base = std::array::from_fn(|i| 100 + i as u32);
        s.next = 108;
        s
    }

    fn new_base(&mut self, i: usize) {
        self.base[i] = self.next;
        self.off[i] = 0;
        self.next += 1;
    }

    /// Every bit becomes its place's own (the reference point of a branch's arm).
    pub(super) fn refresh_bits(&mut self) {
        for s in 0..NSLOT {
            self.v[s].bits = Val::at(Loc::R(s as u8), Src::Unknown).bits;
        }
        for (a, v) in self.mem.iter_mut() {
            v.bits = Val::at(Loc::M(*a), Src::Unknown).bits;
        }
    }

    /// Two paths meeting: what they agree on.
    pub(super) fn join(&self, o: &State) -> State {
        let mut s = self.clone();
        for k in 0..NSLOT {
            if s.v[k].src != o.v[k].src {
                s.v[k].src = Src::Unknown;
            }
            for b in 0..8 {
                if s.v[k].bits[b] != o.v[k].bits[b] {
                    s.v[k].bits[b] = Bit::X;
                }
            }
        }
        for i in 0..8 {
            if s.base[i] != o.base[i] || s.off[i] != o.off[i] {
                s.new_base(i);
            }
        }
        let keys: Vec<u16> = s.mem.keys().chain(o.mem.keys()).copied().collect();
        for a in keys {
            if s.mem.get(&a) != o.mem.get(&a) {
                s.mem.insert(a, Val::UNKNOWN);
            }
        }
        s.mem_lost |= o.mem_lost;
        s.advanced |= o.advanced;
        s
    }

    /// Writes slot `s`; its pair (if any) no longer points where it did.
    fn set(&mut self, s: usize, v: Val) {
        self.v[s] = v;
        if let Some(i) = PAIR_SLOTS.iter().position(|&(h, l)| h == s || l == s) {
            self.new_base(i);
        }
    }

    pub(super) fn word(&self, pair: u8) -> W {
        let Some(i) = pair_index(pair) else { return W::Unknown };
        let (h, l) = PAIR_SLOTS[i];
        match (self.v[l].src, self.v[h].src) {
            (Src::Const(lo), Src::Const(hi)) => W::Const(u16::from(lo) | u16::from(hi) << 8),
            (Src::Imm(a), Src::Imm(b)) if b == a.wrapping_add(1) => W::Imm(a),
            (Src::Mem(a), Src::Mem(b)) if b == a.wrapping_add(1) => W::Mem(a),
            (Src::Idx { pair: p, base, d }, Src::Idx { pair: q, base: b2, d: d2 }) if p == q && base == b2 && d2 == d + 1 => {
                W::Idx { pair: p, base, d }
            }
            (Src::Entry(a), Src::Entry(b)) if a as usize == l && b as usize == h => W::Entry(pair),
            (Src::Acc(x, false), Src::Acc(y, true)) if x == y => W::Acc(x),
            _ => W::Unknown,
        }
    }

    /// The value of a byte source, if it is a constant.
    pub(super) fn byte_value(src: Src, ctx: &Ctx) -> Option<u8> {
        match src {
            Src::Const(k) => Some(k),
            Src::Imm(a) if !ctx.wild.contains(&a) => Some((ctx.peek)(a)),
            _ => None,
        }
    }

    pub(super) fn word_value(w: W, ctx: &Ctx) -> Option<u16> {
        match w {
            W::Const(k) => Some(k),
            W::Imm(a) if !ctx.wild.contains(&a) && !ctx.wild.contains(&a.wrapping_add(1)) => {
                Some(u16::from((ctx.peek)(a)) | u16::from((ctx.peek)(a.wrapping_add(1))) << 8)
            }
            _ => None,
        }
    }

    fn imm(&self, a: u16, ctx: &Ctx) -> Val {
        if ctx.wild.contains(&a) {
            Val { src: Src::Imm(a), bits: [Bit::X; 8] }
        } else {
            Val::known(Src::Imm(a), (ctx.peek)(a))
        }
    }

    pub(super) fn read_mem(&self, a: u16) -> Val {
        match self.mem.get(&a) {
            Some(v) => *v,
            None if self.mem_lost => Val::UNKNOWN,
            None => Val::at(Loc::M(a), Src::Mem(a)),
        }
    }

    fn write_mem(&mut self, a: u16, v: Val) {
        self.mem.insert(a, v);
    }

    fn lose_mem(&mut self) {
        for v in self.mem.values_mut() {
            *v = Val::UNKNOWN;
        }
        self.mem_lost = true;
    }

    /// Register `r` (a delay loop's counter) is 0.
    pub(super) fn zero(&mut self, r: u8) {
        for s in reg_slots(r) {
            self.set(s, Val::known(Src::Const(0), 0));
        }
    }

    /// No bit is known any more (after a branch's arms met: their reference points differ).
    pub(super) fn forget_bits(&mut self) {
        for v in self.v.iter_mut().chain(self.mem.values_mut()) {
            v.bits = [Bit::X; 8];
        }
    }

    /// Everything unknown (a call to code not followed).
    pub(super) fn clobber_all(&mut self) {
        for s in 0..NSLOT {
            self.v[s] = Val::UNKNOWN;
        }
        for i in 0..8 {
            self.new_base(i);
        }
        self.lose_mem();
    }

    /// What `i` writes becomes unknown (an instruction not modelled, or run an unknown number of
    /// times).
    pub(super) fn clobber(&mut self, i: &Insn) {
        // BC, DE and HL: their halves' bits say which (an 8-bit write marks the pair too)
        for r in (0..18u8).filter(|r| !(8..=10).contains(r)) {
            if i.writes & 1 << r != 0 {
                for s in reg_slots(r) {
                    self.set(s, Val::UNKNOWN);
                }
            }
        }
        match i.mem_write {
            Some(MemRef::Abs(a)) => {
                self.write_mem(a, Val::UNKNOWN);
                self.write_mem(a.wrapping_add(1), Val::UNKNOWN);
            }
            Some(MemRef::Reg16(_) | MemRef::Indexed(..)) => self.lose_mem(),
            _ => {}
        }
    }

    /// The byte through pair `pair` + `d` (a load).
    fn through(&self, pair: u8, d: i32) -> Val {
        let i = pair_index(pair).unwrap_or(0);
        Val { src: Src::Idx { pair, base: self.base[i], d: d.wrapping_add(self.off[i]) }, bits: [Bit::X; 8] }
    }

    fn swap(&mut self, a: usize, b: usize) {
        self.v.swap(a, b);
    }

    fn swap_pairs(&mut self, i: usize, j: usize) {
        let ((h1, l1), (h2, l2)) = (PAIR_SLOTS[i], PAIR_SLOTS[j]);
        self.swap(h1, h2);
        self.swap(l1, l2);
        self.base.swap(i, j);
        self.off.swap(i, j);
        let (bi, bj) = (self.advanced >> i & 1, self.advanced >> j & 1);
        self.advanced = self.advanced & !(1 << i | 1 << j) | bi << j | bj << i;
    }

    /// Moves pair index `i` by `k` (a pointer step).
    fn advance(&mut self, i: usize, k: i32) {
        let (h, l) = PAIR_SLOTS[i];
        self.v[h] = Val::UNKNOWN;
        self.v[l] = Val::UNKNOWN;
        self.off[i] = self.off[i].wrapping_add(k);
        self.advanced |= 1 << i;
    }

    /// Runs instruction `i` (its bytes from `ctx`).
    pub(super) fn step(&mut self, i: &Insn, ctx: &Ctx) {
        let p = |k: u16| (ctx.peek)(i.pc.wrapping_add(k));
        let (b0, b1) = (p(0), p(1));
        let idx = match b0 {
            0xDD => Some(11u8),
            0xFD => Some(12u8),
            _ => None,
        };
        // A lone prefix decodes as a NOP: nothing.
        if idx.is_some() && i.len == 1 {
            return;
        }
        let op = if idx.is_some() { b1 } else { b0 };
        if b0 == 0xED {
            return self.step_ed(i, b1, ctx);
        }
        if op == 0xCB {
            return self.step_cb(i, idx, ctx);
        }
        // The slots of the r field (0-7 = B C D E H L (HL) A), H and L the index's halves.
        let r8 = |r: u8, plain: bool| -> usize {
            match (r, idx) {
                (4, Some(x)) if !plain => IXH + 2 * (x as usize - 11),
                (5, Some(x)) if !plain => IXH + 1 + 2 * (x as usize - 11),
                _ => [2, 3, 4, 5, 6, 7, 99, 0][r as usize],
            }
        };
        let hl = idx.unwrap_or(10);
        let disp = || -> i32 { if idx.is_some() { i32::from(p(2) as i8) } else { 0 } };
        let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
        let last2 = i.pc.wrapping_add(u16::from(i.len)).wrapping_sub(2);
        match op {
            0x00 | 0x76 | 0xF3 | 0xFB | 0xD3 => {}
            0x01 | 0x11 | 0x21 => {
                let pair = [8, 9, hl][(op >> 4) as usize];
                let pi = pair_index(pair).unwrap_or(0);
                let (h, l) = PAIR_SLOTS[pi];
                self.v[l] = self.imm(last2, ctx);
                self.v[h] = self.imm(last2.wrapping_add(1), ctx);
                self.new_base(pi);
            }
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E => {
                let v = self.imm(i.pc.wrapping_add(u16::from(i.len)).wrapping_sub(1), ctx);
                self.set(r8(y, false), v);
            }
            0x40..=0x7F if op != 0x76 => {
                if y == 6 {
                    self.lose_mem();
                } else if z == 6 {
                    let v = self.through(hl, disp());
                    self.set(r8(y, true), v);
                } else {
                    let v = self.v[r8(z, false)];
                    self.set(r8(y, false), v);
                }
            }
            0x0A | 0x1A => {
                let v = self.through(if op == 0x0A { 8 } else { 9 }, 0);
                self.set(A, v);
            }
            0x3A => {
                let v = self.read_mem(word_at(ctx, last2));
                self.set(A, v);
            }
            0x32 => {
                let a = word_at(ctx, last2);
                self.write_mem(a, self.v[A]);
            }
            0x2A => {
                let a = word_at(ctx, last2);
                self.load_pair(hl, a);
            }
            0x22 => {
                let a = word_at(ctx, last2);
                self.store_pair(hl, a);
            }
            0x03 | 0x13 | 0x23 | 0x0B | 0x1B | 0x2B => {
                let pair = [8, 9, hl][(op >> 4) as usize];
                let k = if op & 8 == 0 { 1 } else { -1 };
                self.advance(pair_index(pair).unwrap_or(0), k);
            }
            0x09 | 0x19 | 0x29 => self.add_pair(hl, [8, 9, hl][(op >> 4) as usize], ctx),
            0x07 | 0x0F | 0x17 | 0x1F => {
                let old = self.v[A].bits;
                let carry = self.v[F].bits[0];
                let mut new = [Bit::X; 8];
                let out = if op & 8 == 0 {
                    // left
                    new[1..8].copy_from_slice(&old[0..7]);
                    new[0] = if op == 0x07 { old[7] } else { carry };
                    old[7]
                } else {
                    new[0..7].copy_from_slice(&old[1..8]);
                    new[7] = if op == 0x0F { old[0] } else { carry };
                    old[0]
                };
                self.set(A, Val { src: Src::Unknown, bits: new });
                self.v[F].bits = [Bit::X; 8];
                self.v[F].bits[0] = out;
            }
            0x2F => {
                let bits = self.v[A].bits.map(Bit::inv);
                self.set(A, Val { src: Src::Unknown, bits });
            }
            0x37 => self.v[F].bits[0] = Bit::O,
            0x3F => self.v[F].bits[0] = self.v[F].bits[0].inv(),
            _ if x == 2 || (x == 3 && z == 6) => {
                let opd = if x == 3 {
                    self.imm(i.pc.wrapping_add(u16::from(i.len)).wrapping_sub(1), ctx)
                } else if z == 6 {
                    self.through(hl, disp())
                } else {
                    self.v[r8(z, false)]
                };
                let self_op = x == 2 && z != 6 && r8(z, false) == A;
                self.alu(y, opd, self_op, ctx);
            }
            0x08 => {
                self.swap(0, 8);
                self.swap(1, 9);
            }
            0xD9 => {
                for (a, b) in [(0, 5), (1, 6), (2, 7)] {
                    self.swap_pairs(a, b);
                }
            }
            0xEB => self.swap_pairs(1, 2),
            0x10 => self.set(2, Val::UNKNOWN),
            _ => {
                if matches!(i.flow, Flow::Next) || i.writes != 0 || i.mem_write.is_some() {
                    self.clobber(i);
                }
            }
        }
    }

    fn load_pair(&mut self, pair: u8, a: u16) {
        let Some(pi) = pair_index(pair) else { return };
        let (h, l) = PAIR_SLOTS[pi];
        self.v[l] = self.read_mem(a);
        self.v[h] = self.read_mem(a.wrapping_add(1));
        self.new_base(pi);
    }

    fn store_pair(&mut self, pair: u8, a: u16) {
        let Some(pi) = pair_index(pair) else { return };
        let (h, l) = PAIR_SLOTS[pi];
        if let W::Acc(id) = self.word(pair) {
            if let Some(acc) = self.accs.get_mut(id as usize) {
                if acc.base == W::Mem(a) {
                    acc.stored += 1;
                }
            }
        }
        self.write_mem(a, self.v[l]);
        self.write_mem(a.wrapping_add(1), self.v[h]);
    }

    /// `ADD hl,rr`: a pointer step when rr is a known value; a phase accumulator when hl holds
    /// a value from memory or its own start value and rr a step that is not unknown.
    fn add_pair(&mut self, hl: u8, rr: u8, ctx: &Ctx) {
        let Some(pi) = pair_index(hl) else { return };
        let (h, l) = PAIR_SLOTS[pi];
        let (acc, step) = (self.word(hl), self.word(rr));
        let k = if rr != hl { Self::word_value(step, ctx) } else { None };
        let (base, off) = (self.base[pi], self.off[pi]);
        // the accumulator again (a later pass): the same one when the step is
        let again = match acc {
            W::Acc(j) => self.accs.get(j as usize).filter(|a| a.step == step).map(|_| j),
            _ => None,
        };
        let is_acc = (again.is_some() || matches!(acc, W::Mem(_) | W::Entry(_))) && !matches!(step, W::Unknown | W::Acc(_)) && rr != hl;
        if is_acc {
            let id = again.unwrap_or_else(|| {
                self.accs.push(Acc { base: acc, step, stored: 0 });
                (self.accs.len() - 1) as u16
            });
            let mut hi = [Bit::X; 8];
            hi[7] = Bit::Acc { id, inv: false };
            self.v[h] = Val { src: Src::Acc(id, true), bits: hi };
            self.v[l] = Val { src: Src::Acc(id, false), bits: [Bit::X; 8] };
        } else {
            self.v[h] = Val::UNKNOWN;
            self.v[l] = Val::UNKNOWN;
        }
        match k {
            Some(k) => {
                self.base[pi] = base;
                self.off[pi] = off.wrapping_add(i32::from(k as i16));
                self.advanced |= 1 << pi;
            }
            None => self.new_base(pi),
        }
        self.v[F].bits[0] = Bit::X;
    }

    /// ALU operation `y` (ADD ADC SUB SBC AND XOR OR CP) on A with `opd`; `self_op`: the operand
    /// is A itself.
    fn alu(&mut self, y: u8, opd: Val, self_op: bool, ctx: &Ctx) {
        let a = self.v[A];
        let k = Self::byte_value(opd.src, ctx);
        let bits = match y {
            0 => match k {
                // adding a constant: the bits below its lowest set bit stay, that bit flips
                Some(0) => a.bits,
                Some(k) => {
                    let j = k.trailing_zeros() as usize;
                    std::array::from_fn(|b| match b.cmp(&j) {
                        std::cmp::Ordering::Less => a.bits[b],
                        std::cmp::Ordering::Equal => a.bits[b].inv(),
                        std::cmp::Ordering::Greater => Bit::X,
                    })
                }
                None => [Bit::X; 8],
            },
            4 => std::array::from_fn(|b| Bit::and(a.bits[b], opd.bits[b])),
            5 if self_op => [Bit::Z; 8],
            5 => std::array::from_fn(|b| Bit::xor(a.bits[b], opd.bits[b])),
            6 => std::array::from_fn(|b| Bit::or(a.bits[b], opd.bits[b])),
            7 => a.bits,
            _ => [Bit::X; 8],
        };
        let src = match y {
            5 if self_op => Src::Const(0),
            4 | 6 if self_op => a.src,
            7 => a.src,
            _ => Src::Unknown,
        };
        if y != 7 {
            self.set(A, Val { src, bits });
        }
        self.v[F].bits = [Bit::X; 8];
        if (4..=6).contains(&y) {
            self.v[F].bits[0] = Bit::Z;
        }
    }

    fn step_cb(&mut self, i: &Insn, idx: Option<u8>, ctx: &Ctx) {
        let op = (ctx.peek)(i.pc.wrapping_add(if idx.is_some() { 3 } else { 1 }));
        let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
        if idx.is_some() || z == 6 {
            // memory through a pointer
            if x != 1 {
                self.lose_mem();
            }
            if idx.is_some() && z != 6 && x != 1 {
                self.set([2, 3, 4, 5, 6, 7, 0, 0][z as usize], Val::UNKNOWN);
            }
            self.v[F].bits = [Bit::X; 8];
            return;
        }
        let s = [2, 3, 4, 5, 6, 7, 99, 0][z as usize];
        let old = self.v[s].bits;
        let carry = self.v[F].bits[0];
        let mut new = old;
        let mut out = Bit::X;
        match x {
            0 => {
                let left = y & 1 == 0;
                if left {
                    new[1..8].copy_from_slice(&old[0..7]);
                    out = old[7];
                    new[0] = match y {
                        0 => old[7],
                        2 => carry,
                        4 => Bit::Z,
                        6 => Bit::O,
                        _ => Bit::X,
                    };
                } else {
                    new[0..7].copy_from_slice(&old[1..8]);
                    out = old[0];
                    new[7] = match y {
                        1 => old[0],
                        3 => carry,
                        5 => old[7],
                        7 => Bit::Z,
                        _ => Bit::X,
                    };
                }
            }
            1 => {
                self.v[F].bits = [Bit::X; 8];
                self.v[F].bits[0] = carry;
                return;
            }
            2 => new[y as usize] = Bit::Z,
            _ => new[y as usize] = Bit::O,
        }
        self.set(s, Val { src: Src::Unknown, bits: new });
        if x == 0 {
            self.v[F].bits = [Bit::X; 8];
            self.v[F].bits[0] = out;
        }
    }

    fn step_ed(&mut self, i: &Insn, op: u8, ctx: &Ctx) {
        let a = i.pc.wrapping_add(2);
        match op {
            0x4B | 0x5B | 0x6B => self.load_pair([8, 9, 10][((op >> 4) - 4) as usize], word_at(ctx, a)),
            0x43 | 0x53 | 0x63 => self.store_pair([8, 9, 10][((op >> 4) - 4) as usize], word_at(ctx, a)),
            // OUT (C),r; IM; the ones that write nothing tracked
            0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x71 | 0x79 | 0x46 | 0x56 | 0x5E | 0x47 | 0x4F => {}
            _ => self.clobber(i),
        }
    }
}

/// The little-endian word at `a`.
fn word_at(ctx: &Ctx, a: u16) -> u16 {
    u16::from((ctx.peek)(a)) | u16::from((ctx.peek)(a.wrapping_add(1))) << 8
}
