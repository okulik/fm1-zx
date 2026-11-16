// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! From a pass to a model: the routine's extent and signature, one symbolic run of the pass,
//! the family and channels it shows, their formulas, the note's timing and the table walk.

use super::cfg::{bits_of, setup_paths, undecoded, Cfg, Item, SetupPath};
use super::sym::{pair_index, slot_reg, stack_pair, Bit, Loc, Pushed, Src, State, W};
use super::{ChannelModel, Ctx, DurationModel, EngineModel, Family, Input, Refusal, Term, Why};
use crate::z80::{Flow, Insn, MemRef, PortRef};
use std::collections::BTreeSet;
use zx_sys::{SND_LEARNED_LEN_MAX, SND_LEARNED_SITES_MAX, SND_TONAL};

/// A phase accumulator's half period is this many passes over its step (a 16-bit phase's top
/// bit).
const HALF_PHASE: i64 = 32768;

/// A count, a step or a constant: an input or a known value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    K(i64),
    In(Input),
}

/// T-states: a constant plus inputs times coefficients.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sum {
    k: i64,
    lin: Vec<(Input, i64)>,
}

impl Sum {
    /// `per` T-states for each of `v` counts plus `konst`; a known count of 0 runs 256 (or
    /// 65,536 for a pair) times.
    fn add_count(&mut self, per: i64, konst: i64, v: Value, wide: bool) {
        self.k += konst;
        match v {
            Value::K(n) => self.k += per * if n == 0 { if wide { 65536 } else { 256 } } else { n },
            Value::In(x) => match self.lin.iter_mut().find(|(y, _)| *y == x) {
                Some((_, c)) => *c += per,
                None => self.lin.push((x, per)),
            },
        }
    }
    fn konst(&self) -> Option<i64> {
        self.lin.is_empty().then_some(self.k)
    }
    /// The sum over `n`: None if a coefficient does not divide (the constant is rounded).
    fn div(&self, n: i64) -> Option<Sum> {
        let lin = self.lin.iter().map(|&(x, c)| (c % n == 0).then_some((x, c / n))).collect::<Option<Vec<_>>>()?;
        Some(Sum { k: (self.k + n / 2).div_euclid(n), lin })
    }
    fn terms(&self) -> Vec<Term> {
        let mut v: Vec<Term> = self.lin.iter().map(|&(x, coef_t)| Term::Lin { coef_t, x }).collect();
        if self.k != 0 || v.is_empty() {
            v.push(Term::Const(self.k));
        }
        v
    }
}

/// Turns sources into inputs: valid at a site's `OUT` (its registers) with the memory at the
/// frame's end.
struct Conv<'a> {
    ctx: &'a Ctx<'a>,
    anchor: u16,
    code_end: u16,
    /// Bytes the pass writes at fixed addresses.
    pass_mem: BTreeSet<u16>,
    /// Registers the pass writes (decoder bits).
    pass_writes: u32,
    /// Registers must keep their value over the whole pass (several sites, or the pass does
    /// not start at the one site).
    strict: bool,
    /// Registers (decoder bits) that hold their value at the pass's start at every speaker `OUT`
    /// and at its end (a pair saved and restored around a delay that counts it down).
    restored: u32,
}

impl Conv<'_> {
    fn in_code(&self, a: u16) -> bool {
        (self.anchor..self.code_end).contains(&a)
    }
    fn mem(&self, a: u16, wide: bool) -> Option<Input> {
        if self.pass_mem.contains(&a) || wide && self.pass_mem.contains(&a.wrapping_add(1)) {
            return None;
        }
        Some(match (self.in_code(a), wide) {
            (true, false) => Input::Rel(a - self.anchor),
            (true, true) => Input::Rel16(a - self.anchor),
            (false, false) => Input::Abs(a),
            (false, true) => Input::Abs16(a),
        })
    }
    fn kept(&self, reg: u8) -> bool {
        !self.strict || self.pass_writes & bits_of(reg) == 0 || self.restored & bits_of(reg) == bits_of(reg)
    }
    /// A byte of the pass's symbolic run (registers as at the pass's start).
    fn byte(&self, s: Src) -> Option<Value> {
        match s {
            Src::Const(k) => Some(Value::K(k.into())),
            Src::Imm(a) if self.ctx.wild.contains(&a) => Some(Value::In(Input::Rel(a.wrapping_sub(self.anchor)))),
            Src::Imm(a) => Some(Value::K((self.ctx.peek)(a).into())),
            Src::Mem(a) => self.mem(a, false).map(Value::In),
            Src::Idx { pair, base, d } => {
                let pi = pair_index(pair)?;
                (base == pi as u32 && self.kept(pair)).then_some(())?;
                Some(Value::In(Input::Indexed(pair, i8::try_from(d).ok()?)))
            }
            Src::Entry(slot) => {
                let r = slot_reg(slot as usize)?;
                self.kept(r).then_some(Value::In(Input::Reg(r)))
            }
            Src::Unknown | Src::Acc(..) => None,
        }
    }
    fn word(&self, w: W) -> Option<Value> {
        match w {
            W::Const(k) => Some(Value::K(k.into())),
            W::Imm(a) if self.ctx.wild.contains(&a) || self.ctx.wild.contains(&a.wrapping_add(1)) => {
                Some(Value::In(Input::Rel16(a.wrapping_sub(self.anchor))))
            }
            W::Imm(_) => State::word_value(w, self.ctx).map(|k| Value::K(k.into())),
            W::Mem(a) => self.mem(a, true).map(Value::In),
            W::Entry(pair) => self.kept(pair).then_some(Value::In(Input::Reg(pair))),
            W::Idx { .. } | W::Unknown | W::Acc(_) => None,
        }
    }
    /// A counter's start as a setup path left it (`end`): an input valid at the sites (the
    /// pass does not write a base register it uses). A 16-bit start through a pointer is the
    /// word there (see [`DurationModel`]).
    fn start(&self, end: &State, counter: u8) -> Option<Input> {
        let idx = |pair: u8, base: u32, d: i32| -> Option<Input> {
            let pi = pair_index(pair)?;
            (end.base[pi] == base && self.pass_writes & bits_of(pair) == 0).then_some(())?;
            Some(Input::Indexed(pair, i8::try_from(d.wrapping_sub(end.off[pi])).ok()?))
        };
        if counter >= 8 {
            match end.word(counter) {
                W::Imm(a) if self.in_code(a) => Some(Input::Rel16(a - self.anchor)),
                W::Mem(a) => self.mem(a, true),
                W::Idx { pair, base, d } => idx(pair, base, d),
                _ => None,
            }
        } else {
            match end.v[counter as usize].src {
                Src::Imm(a) if self.in_code(a) => Some(Input::Rel(a - self.anchor)),
                Src::Mem(a) => self.mem(a, false),
                Src::Idx { pair, base, d } => idx(pair, base, d),
                _ => None,
            }
        }
    }
}

/// A speaker `OUT` met in the run: its bit 4 and its time from the pass's start.
struct Out {
    pc: u16,
    bit: Bit,
    t: Sum,
    item: usize,
    /// The registers (decoder bits) holding their value at the pass's start here.
    kept: u32,
}

struct Arm {
    outs: Vec<(u16, Bit)>,
    end: State,
    time: i64,
    writes: u32,
}

struct DiamondRun {
    pc: u16,
    item: usize,
    counter: u8,
    zero: Arm,
    skip: Arm,
    nodes: Vec<u16>,
}

impl DiamondRun {
    /// T-states the zero (reload) arm takes beyond the other: a pass with the reload is this
    /// much longer (the pass's time counts the other arm).
    fn extra(&self) -> i64 {
        self.zero.time - self.skip.time
    }
}

struct DelayRun {
    counter: u8,
    value: Value,
    /// Speaker `OUT`s before it in the pass.
    seg: usize,
}

/// Bit 4 of what `i` writes to the speaker, if it is a speaker `OUT` (an even port).
fn speaker(st: &State, i: &Insn, ctx: &Ctx) -> Result<Option<Bit>, Refusal> {
    let refuse = |pc: u16| Refusal::Unmodelled(Why::PortUnknown, pc);
    match i.port_out {
        None => Ok(None),
        Some(PortRef::Imm(n)) => Ok((n & 1 == 0).then_some(st.v[super::sym::A].bits[4])),
        Some(PortRef::RegC) => {
            let op = (ctx.peek)(i.pc.wrapping_add(1));
            if (ctx.peek)(i.pc) != 0xED || op & 0xC7 != 0x41 {
                return Err(refuse(i.pc)); // a block output
            }
            let c = State::byte_value(st.v[3].src, ctx).ok_or(refuse(i.pc))?;
            if c & 1 != 0 {
                return Ok(None);
            }
            Ok(Some(match (op >> 3) & 7 {
                6 => Bit::Z,
                r => st.v[[2, 3, 4, 5, 6, 7, 0, 0][r as usize]].bits[4],
            }))
        }
    }
}

/// The bit at `loc` in state `st`.
fn bit_at(st: &State, loc: Loc, bit: u8) -> Bit {
    match loc {
        Loc::R(s) => st.v[s as usize].bits[bit as usize],
        Loc::M(a) => st.read_mem(a).bits[bit as usize],
    }
}

/// The speaker levels of consecutive `OUT`s change at every one (each `OUT` an edge), whatever
/// the start values of the bits they depend on.
fn toggles(levels: &[Bit]) -> bool {
    let mut vars: Vec<(Loc, u8)> = Vec::new();
    for b in levels {
        match *b {
            Bit::E { loc, bit, .. } if !vars.contains(&(loc, bit)) => vars.push((loc, bit)),
            Bit::E { .. } | Bit::Z | Bit::O => {}
            Bit::X | Bit::Acc { .. } => return false,
        }
    }
    if vars.len() > 4 {
        return false;
    }
    (0..1u32 << vars.len()).all(|a| {
        let ev = |b: &Bit| match *b {
            Bit::O => 1,
            Bit::E { loc, bit, inv } => (a >> vars.iter().position(|v| *v == (loc, bit)).unwrap_or(0) & 1) as u8 ^ u8::from(inv),
            _ => 0,
        };
        levels.windows(2).all(|w| ev(&w[0]) != ev(&w[1]))
    })
}

/// When the levels do not toggle at every `OUT` (see [`toggles`]): whether they do exactly when
/// the one bit of the pass's start they depend on has one value. Gives the bit and that value
/// (the channel is silent for the other).
fn toggle_condition(levels: &[Bit]) -> Option<(Loc, u8, bool)> {
    let mut vars: Vec<(Loc, u8)> = Vec::new();
    for b in levels {
        match *b {
            Bit::E { loc, bit, .. } if !vars.contains(&(loc, bit)) => vars.push((loc, bit)),
            Bit::E { .. } | Bit::Z | Bit::O => {}
            Bit::X | Bit::Acc { .. } => return None,
        }
    }
    if vars.len() != 1 {
        return None;
    }
    let ok = |v: u32| {
        let ev = |b: &Bit| match *b {
            Bit::O => 1u32,
            Bit::E { inv, .. } => v ^ u32::from(inv),
            _ => 0,
        };
        levels.windows(2).all(|w| ev(&w[0]) != ev(&w[1]))
    };
    match (ok(0), ok(1)) {
        (true, false) => Some((vars[0].0, vars[0].1, false)),
        (false, true) => Some((vars[0].0, vars[0].1, true)),
        _ => None,
    }
}

/// A `PUSH` (true) or `POP` (false) of a pair, and the pair ([`stack_pair`]).
fn push_pop(peek: &dyn Fn(u16) -> u8, i: &Insn) -> Option<(bool, u8)> {
    let (b0, b1) = (peek(i.pc), peek(i.pc.wrapping_add(1)));
    let (idx, op) = match b0 {
        0xDD if i.len == 2 => (Some(11), b1),
        0xFD if i.len == 2 => (Some(12), b1),
        _ if i.len == 1 => (None, b0),
        _ => return None,
    };
    if idx.is_some() && op & 0xEF != 0xE1 && op & 0xEF != 0xE5 {
        return None;
    }
    match op {
        0xC5 | 0xD5 | 0xE5 | 0xF5 => Some((true, stack_pair(op, idx))),
        0xC1 | 0xD1 | 0xE1 | 0xF1 => Some((false, stack_pair(op, idx))),
        _ => None,
    }
}

/// Whether `i` writes place `loc`.
fn writes_loc(peek: &dyn Fn(u16) -> u8, i: &Insn, loc: Loc) -> bool {
    match loc {
        Loc::R(s) => {
            let bit = match s {
                0..=7 => 1u32 << s,
                8 | 9 => 1 << 14,
                10 | 11 => 1 << 15,
                12 | 13 => 1 << 16,
                14 | 15 => 1 << 17,
                16 | 17 => 1 << 11,
                _ => 1 << 12,
            };
            i.writes & bit != 0
        }
        Loc::M(a) => abs_writes(peek, i).contains(&a),
    }
}

/// `LD (nn),HL`-style stores: two bytes.
fn store16(peek: &dyn Fn(u16) -> u8, i: &Insn) -> bool {
    let (b0, b1) = (peek(i.pc), peek(i.pc.wrapping_add(1)));
    b0 == 0x22 || (b0 == 0xED && b1 & 0xC7 == 0x43) || (matches!(b0, 0xDD | 0xFD) && b1 == 0x22)
}

/// The fixed addresses `i` writes (one byte, or two for a 16-bit store).
fn abs_writes(peek: &dyn Fn(u16) -> u8, i: &Insn) -> Vec<u16> {
    match i.mem_write {
        Some(MemRef::Abs(a)) if store16(peek, i) => vec![a, a.wrapping_add(1)],
        Some(MemRef::Abs(a)) => vec![a],
        _ => vec![],
    }
}

/// The operand bytes of `i` that are immediate data (`LD r,n`, `LD rr,nn`, `ALU n`, `LD (HL),n`).
fn imm_bytes(peek: &dyn Fn(u16) -> u8, i: &Insn) -> Vec<u16> {
    let b0 = peek(i.pc);
    let op = if matches!(b0, 0xDD | 0xFD) && i.len > 1 { peek(i.pc.wrapping_add(1)) } else { b0 };
    let end = i.pc.wrapping_add(u16::from(i.len));
    if b0 == 0xED || op == 0xCB {
        return vec![];
    }
    match op {
        0x01 | 0x11 | 0x21 | 0x31 => vec![end.wrapping_sub(2), end.wrapping_sub(1)],
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E | 0x36 | 0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => vec![end.wrapping_sub(1)],
        _ => vec![],
    }
}

/// Where `i` holds an absolute address (its last two bytes), if it does.
fn addr16(peek: &dyn Fn(u16) -> u8, i: &Insn) -> Option<u16> {
    let (b0, b1) = (peek(i.pc), peek(i.pc.wrapping_add(1)));
    let has = match b0 {
        0xDD | 0xFD => matches!(b1, 0x21 | 0x22 | 0x2A),
        0xED => b1 & 0xC7 == 0x43,
        0x01 | 0x11 | 0x21 | 0x31 | 0x22 | 0x2A | 0x32 | 0x3A => true,
        _ => b0 & 0xC7 == 0xC2 || b0 & 0xC7 == 0xC4 || b0 == 0xC3 || b0 == 0xCD,
    };
    has.then(|| i.pc.wrapping_add(u16::from(i.len)).wrapping_sub(2))
}

/// Runs a setup path from nothing known; calls clobber everything.
fn run_path(cfg: &Cfg, ctx: &Ctx, p: &SetupPath) -> State {
    let mut st = State::fresh();
    for pc in &p.nodes {
        let i = &cfg.n[pc];
        match i.flow {
            Flow::Call(_) | Flow::CondCall(_) | Flow::Rst(_) => st.clobber_all(),
            _ => st.step(i, ctx),
        }
    }
    st
}

/// The setup paths of one zero test, checked against the pass.
struct Setup<'a> {
    cfg: &'a Cfg,
    ctx: &'a Ctx<'a>,
    conv: &'a Conv<'a>,
    items: &'a [Item],
    /// The state at the pass's start of a warmed-up traversal.
    start: &'a State,
    /// The zero test's item.
    test: usize,
    /// The level the Flips channels share.
    shared: &'a [Loc],
    /// The addresses the test's exit stores A to on its way out of the routine
    /// ([`level_stores`]): a path that loads A from one takes the level back as the test left it.
    level_mem: Vec<u16>,
}

impl Setup<'_> {
    fn run(&self, items: &[Item], st: State) -> Option<Run> {
        run_pass(self.cfg, self.ctx, self.conv, items, st, false).ok()
    }

    /// Whether path `p` has no speaker `OUT` and leaves the level as the pass would: from the
    /// state at the test, the pass's own way round to where `p` re-enters and `p` itself, each
    /// followed by the pass from there and one more, give the same speaker bits wherever the
    /// pass's depend on its state (`p` may set them to a constant); for Flips, `p` leaves the
    /// shared level as it was or sets its speaker bit to a constant. Calls may change anything.
    fn keeps_level(&self, p: &SetupPath) -> Result<(), Why> {
        let (cfg, items) = (self.cfg, self.items);
        let lost = Why::SetupLevel;
        let r = items.iter().position(|it| it.pc() == p.reentry).ok_or(lost)?;
        let at_test = self.run(&items[..=self.test], self.start.clone()).ok_or(lost)?;
        // the pass's own way: the test's staying side on to the re-entry
        let around: Vec<Item> = if r > self.test {
            items[self.test + 1..r].to_vec()
        } else {
            items[self.test + 1..].iter().chain(&items[..r]).cloned().collect()
        };
        let normal = self.run(&around, at_test.end.clone()).ok_or(lost)?;
        let mut st = at_test.end;
        let a_at_test = st.v[super::sym::A];
        let mut called = false;
        let mut set_shared = false;
        for pc in &p.nodes {
            let i = &cfg.n[pc];
            // A loaded back from where the exit stored it: the level the test left
            if (self.ctx.peek)(*pc) == 0x3A && i.len == 3 {
                let a = u16::from((self.ctx.peek)(pc.wrapping_add(1))) | u16::from((self.ctx.peek)(pc.wrapping_add(2))) << 8;
                if self.level_mem.contains(&a) {
                    st.step(i, self.ctx);
                    st.v[super::sym::A] = a_at_test;
                    continue;
                }
            }
            if matches!(i.port_out, Some(PortRef::Imm(n)) if n & 1 == 0) || i.port_out == Some(PortRef::RegC) {
                return Err(Why::SetupOut);
            }
            set_shared |= self.shared.iter().any(|&l| writes_loc(self.ctx.peek, i, l));
            match i.flow {
                Flow::Call(_) | Flow::CondCall(_) | Flow::Rst(_) => {
                    called = true;
                    st.clobber_all()
                }
                _ => st.step(i, self.ctx),
            }
        }
        // the level the Flips channels share may be set (to a constant: a note starting from a
        // known level), never left flipped or unknown
        if set_shared && self.shared.iter().any(|&l| !matches!(bit_at(&st, l, 4), Bit::Z | Bit::O)) {
            return Err(Why::SetupFlipsLevel);
        }
        let on: Vec<Item> = items[r..].iter().chain(items).cloned().collect();
        let (a, b) = (self.run(&on, normal.end).ok_or(lost)?, self.run(&on, st).ok_or(lost)?);
        let same = a.outs.len() == b.outs.len()
            && a.outs.iter().zip(&b.outs).all(|(x, y)| x.pc == y.pc && (!matches!(x.bit, Bit::E { .. }) || y.bit == x.bit || matches!(y.bit, Bit::Z | Bit::O)));
        match (same, called) {
            (true, _) => Ok(()),
            (false, true) => Err(Why::SetupCall),
            (false, false) => Err(lost),
        }
    }
}

/// A routine called once a note may keep the level between calls in memory: its exit stores
/// A there on the straight way to its return, and the next call loads it back. The addresses
/// of the `LD (nn),A` on the straight way from `exit` to a `RET` (at most eight instructions,
/// none writing A), or none when the way does not end at a `RET`.
fn level_stores(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, exit: u16) -> Vec<u16> {
    let mut stores = Vec::new();
    let mut pc = exit;
    for _ in 0..8 {
        let Some(i) = cfg.n.get(&pc) else { break };
        if peek(pc) == 0x32 && i.len == 3 {
            stores.push(u16::from(peek(pc.wrapping_add(1))) | u16::from(peek(pc.wrapping_add(2))) << 8);
        } else if i.writes & 1 != 0 || i.flow != Flow::Next {
            break;
        }
        pc = pc.wrapping_add(u16::from(i.len));
    }
    if cfg.n.get(&pc).is_some_and(|i| i.flow == Flow::Ret) {
        stores
    } else {
        Vec::new()
    }
}

/// One traversal of the pass.
struct Run {
    outs: Vec<Out>,
    delays: Vec<DelayRun>,
    diamonds: Vec<DiamondRun>,
    /// Its T-states (the first traversal's; later ones leave counts out).
    t: Sum,
    end: State,
}

/// Runs the pass from `st`. The first traversal names the delay loops' counts (from registers
/// as at the pass's start); later ones only carry on (a port not known yet, in the first, is
/// taken as a speaker's).
fn run_pass(cfg: &Cfg, ctx: &Ctx, conv: &Conv, items: &[Item], mut st: State, first: bool) -> Result<Run, Refusal> {
    let un = Refusal::Unmodelled;
    let mut r = Run { outs: Vec::new(), delays: Vec::new(), diamonds: Vec::new(), t: Sum::default(), end: State::entry() };
    let spk = |s: &State, i: &Insn| match speaker(s, i, ctx) {
        Err(_) if first => Ok(Some(Bit::X)),
        x => x,
    };
    for (k, it) in items.iter().enumerate() {
        match it {
            Item::Op(pc) | Item::Test { pc, .. } | Item::Fixed { pc, .. } => {
                let i = &cfg.n[pc];
                if let Some(bit) = spk(&st, i)? {
                    r.outs.push(Out { pc: *pc, bit, t: r.t.clone(), item: k, kept: st.unchanged() });
                }
                st.step(i, ctx);
                r.t.k += match it {
                    Item::Test { stay_taken: true, .. } | Item::Fixed { taken: true, .. } => i64::from(i.t_taken),
                    _ => i64::from(i.t),
                };
            }
            Item::Delay(h) => {
                let d = &cfg.delays[h];
                let counters = [Some((d.counter, d.per, d.konst)), d.outer];
                if first {
                    for (counter, per, konst) in counters.into_iter().flatten() {
                        let wide = counter >= 8;
                        let value = if wide { conv.word(st.word(counter)) } else { conv.byte(st.v[counter as usize].src) };
                        let value = value.ok_or(un(Why::UnnamedInput, *h))?;
                        r.t.add_count(per, konst, value, wide);
                        r.delays.push(DelayRun { counter, value, seg: r.outs.len() });
                    }
                }
                for p in &d.nodes {
                    st.clobber(&cfg.n[p]);
                }
                for (counter, _, _) in counters.into_iter().flatten() {
                    st.zero(counter);
                }
            }
            Item::Diamond { pc, zero_taken, taken, fall, counter, .. } => {
                let b = &cfg.n[pc];
                let run_arm = |nodes: &[u16], side_taken: bool| -> Result<Arm, Refusal> {
                    let mut s = st.clone();
                    s.refresh_bits();
                    let mut a = Arm { outs: Vec::new(), end: s.clone(), time: i64::from(if side_taken { b.t_taken } else { b.t }), writes: 0 };
                    for p in nodes {
                        let i = &cfg.n[p];
                        if let Some(bit) = spk(&s, i)? {
                            a.outs.push((*p, bit));
                        }
                        s.step(i, ctx);
                        a.time += i64::from(i.t);
                        a.writes |= i.writes;
                    }
                    a.end = s;
                    Ok(a)
                };
                let (ta, fa) = (run_arm(taken, true)?, run_arm(fall, false)?);
                st = ta.end.join(&fa.end);
                st.forget_bits();
                let (zero, skip) = if *zero_taken { (ta, fa) } else { (fa, ta) };
                // the pass's time counts the arm without the reload (see `DiamondRun::extra`)
                r.t.k += skip.time;
                let nodes = if *zero_taken { taken.clone() } else { fall.clone() };
                r.diamonds.push(DiamondRun { pc: *pc, item: k, counter: *counter, zero, skip, nodes });
            }
        }
    }
    r.end = st;
    Ok(r)
}

/// The model of the pass `items` through `out`.
pub(super) fn analyse(peek: &dyn Fn(u16) -> u8, cfg: &Cfg, items: Vec<Item>, out: u16) -> Result<EngineModel, Refusal> {
    let un = Refusal::Unmodelled;
    // ---- the pass's instructions ----
    let mut main = BTreeSet::new();
    let mut inside = BTreeSet::new();
    let mut all: Vec<u16> = Vec::new();
    for it in &items {
        if !cfg.inlined.contains(&it.pc()) {
            main.insert(it.pc());
        }
        match it {
            Item::Op(pc) | Item::Test { pc, .. } | Item::Fixed { pc, .. } => all.push(*pc),
            Item::Delay(h) => {
                let d = &cfg.delays[h];
                all.extend(&d.nodes);
                inside.extend(d.nodes.iter().filter(|&&p| p != *h));
            }
            Item::Diamond { pc, taken, fall, .. } => {
                all.push(*pc);
                all.extend(taken.iter().chain(fall));
                inside.extend(taken.iter().chain(fall));
            }
        }
    }
    let distinct: BTreeSet<u16> = all.iter().copied().collect();
    if !distinct.contains(&out) {
        return Err(un(Why::OutNotInLoop, out));
    }
    // the pass's own instructions run once (a routine it calls may run more than once)
    let own: Vec<u16> = all.iter().copied().filter(|p| !cfg.inlined.contains(p)).collect();
    let own_distinct: BTreeSet<u16> = own.iter().copied().collect();
    if own_distinct.len() != own.len() {
        return Err(un(Why::PassShape, out));
    }
    let mut contended_reads = false;
    for &pc in &all {
        let i = &cfg.n[&pc];
        // whole-T formulas cannot carry the contended range's delays
        let contended = |a: u16, n: u16| (u32::from(a) + u32::from(n) > 0x4000) && a < 0x8000;
        let mem_contended = matches!(i.mem_write, Some(MemRef::Abs(a)) if contended(a, 2));
        // a read of a fixed address there waits for the screen as a read through a pointer may:
        // the fit to what was played judges it (see `pointer_reads`)
        if matches!(i.mem_read, Some(MemRef::Abs(a)) if contended(a, 2)) {
            contended_reads = true;
        }
        if contended(pc, i.len.into()) || mem_contended {
            return Err(un(Why::Contended, pc));
        }
        // stores through pointers and the stack: not modelled, but for PUSH and POP in the pass's
        // straight code and the calls the walk follows (matched below)
        let stack = i.reads & 1 << 13 != 0 || i.writes & 1 << 13 != 0 || matches!(i.mem_write, Some(MemRef::Sp));
        if stack && !cfg.callret.contains(&pc) && (push_pop(peek, i).is_none() || inside.contains(&pc)) {
            return Err(un(Why::StackOp, pc));
        }
        if matches!(i.mem_write, Some(MemRef::Reg16(_) | MemRef::Indexed(..))) {
            return Err(un(Why::PointerStore, pc));
        }
    }

    // PUSH and POP in the pass: each POP takes what a PUSH of the same pair put, the pass going
    // round (a POP before any PUSH takes what the end of the pass pushes); the pass starts with
    // those pushed, as it leaves them (checked after the runs). A RET takes what its CALL put
    // (no pair: RETURN), and a called routine's POP never takes the return address
    const RETURN: u8 = 0xFF;
    let mut open: Vec<u8> = Vec::new();
    let mut leading: Vec<u8> = Vec::new();
    for it in &items {
        let (Item::Op(pc) | Item::Test { pc, .. }) = it else { continue };
        let call_ret = cfg.callret.contains(pc).then(|| (peek(*pc) == 0xCD, RETURN));
        match call_ret.or_else(|| push_pop(peek, &cfg.n[pc])) {
            Some((true, p)) => open.push(p),
            Some((false, p)) => match open.pop() {
                Some(q) if q == p => {}
                Some(_) => return Err(un(Why::StackOp, *pc)),
                None => leading.push(p),
            },
            None => {}
        }
    }
    if leading.iter().rev().ne(open.iter()) || open.contains(&RETURN) {
        return Err(un(Why::StackOp, out));
    }
    let pointer_reads = contended_reads || all.iter().any(|pc| matches!(cfg.n[pc].mem_read, Some(MemRef::Reg16(_) | MemRef::Indexed(..))));
    let uses_stack = !open.is_empty() || all.iter().any(|pc| push_pop(peek, &cfg.n[pc]).is_some());
    let seed: Vec<Pushed> = {
        let entry = State::entry();
        open.iter().map(|&p| entry.pushed(p)).collect()
    };

    // the zero tests leaving the pass, with their counters (a pair, when the test's exit is its
    // carry into the high half: the carry's instructions then belong to the routine, not to a
    // setup path) and where they leave
    let mut carries: Vec<u16> = Vec::new();
    let mut tests: Vec<(usize, u16, u8, u16)> = Vec::new();
    let mut carried: Vec<bool> = Vec::new();
    for (k, it) in items.iter().enumerate() {
        let Item::Test { pc, counter, exit, stay_taken } = it else { continue };
        // a way out of the pass from inside a routine it calls: the way may return into the pass
        // (setup paths that return are not followed)
        if cfg.inlined.contains(pc) {
            return Err(un(Why::TestInCallee, *pc));
        }
        let i = &cfg.n[pc];
        let stay = match (i.flow, stay_taken) {
            (Flow::CondJump(t) | Flow::Djnz(t), true) => t,
            _ => pc.wrapping_add(u16::from(i.len)),
        };
        match cfg.carry(peek, *exit, stay, *counter) {
            Some((pair, nodes, past)) => {
                carries.extend(nodes);
                tests.push((k, *pc, pair, past));
                carried.push(true);
            }
            None => {
                tests.push((k, *pc, *counter, *exit));
                carried.push(false);
            }
        }
    }
    inside.extend(&carries);
    // a zero test leaving the pass counts down to it: nothing else in the pass writes its counter
    // (else the side the pass takes is not known)
    for &(_, pc, counter, _) in &tests {
        let (_, idiom) = cfg.zero_idiom(peek, &cfg.n[&pc]).ok_or(un(Why::BranchIdiom, pc))?;
        if all.iter().any(|p| cfg.n[p].writes & bits_of(counter) != 0 && !idiom.contains(p)) {
            return Err(un(Why::CounterReloaded, pc));
        }
    }

    // the code of the routines the pass calls is not where a path comes back into it
    let avoid: BTreeSet<u16> = inside.union(&cfg.inlined).copied().collect();
    let mut paths: Vec<Vec<SetupPath>> = tests.iter().map(|t| setup_paths(cfg, t.3, &main, &avoid).ok_or(un(Why::SetupPaths, t.3))).collect::<Result<_, _>>()?;
    // the code that runs straight into the pass from elsewhere (a routine called once a note or
    // a burst, its setup before the loop): a path into the pass too; with one zero test, one of
    // its setup paths
    let leads = lead_ins(cfg, &main, &avoid, &carries, &paths);
    // the notes' setup loops back into the pass within the routine
    let looped = paths.iter().any(|ps| !ps.is_empty());
    let mut region: BTreeSet<u16> = distinct.clone();
    region.extend(&carries);
    region.extend(paths.iter().chain([&leads]).flatten().flat_map(|p| p.nodes.iter().copied()));
    if let [ps] = paths.as_mut_slice() {
        ps.extend(leads.iter().cloned());
    } else if leads.iter().flat_map(|p| &p.nodes).any(|pc| cfg.n[pc].port_out.is_some()) {
        return Err(un(Why::SetupOut, leads[0].nodes[0]));
    }
    // the routines the pass calls lie inside the signature, as the rest: the digest pins every
    // byte the formulas depend on
    let span = |r: &BTreeSet<u16>| match (r.first(), r.last()) {
        (Some(&a), Some(&b)) => u32::from(b) + u32::from(cfg.n[&b].len) - u32::from(a),
        _ => 0,
    };
    if span(&region) > u32::from(SND_LEARNED_LEN_MAX) {
        let own: BTreeSet<u16> = region.iter().copied().filter(|p| !cfg.inlined.contains(p)).collect();
        if own.len() != region.len() && span(&own) <= u32::from(SND_LEARNED_LEN_MAX) {
            return Err(un(Why::CalleeOutsideSignature, out));
        }
    }
    let anchor = *region.first().ok_or(un(Why::PassShape, out))?;
    let mut end = 0u32;
    for &pc in &region {
        let i = &cfg.n[&pc];
        if u32::from(pc) < end {
            return Err(un(Why::OverlappingCode, pc));
        }
        if undecoded(peek, i) {
            return Err(Refusal::Undecoded(pc));
        }
        end = u32::from(pc) + u32::from(i.len);
    }
    let code_end = u16::try_from(end).map_err(|_| un(Why::TooLong, out))?;
    if code_end - anchor > SND_LEARNED_LEN_MAX {
        return Err(un(Why::TooLong, anchor));
    }
    let owner = |a: u16| region.range(..=a).next_back().map(|pc| &cfg.n[pc]).filter(|i| u32::from(a) < u32::from(i.pc) + u32::from(i.len));

    // ---- self-modification: operand bytes the routine writes ----
    let mut wild = BTreeSet::new();
    for &w in &region {
        let wi = &cfg.n[&w];
        for a in abs_writes(peek, wi) {
            if !(anchor..code_end).contains(&a) {
                continue;
            }
            match owner(a) {
                None => {
                    wild.insert(a);
                }
                Some(o) if imm_bytes(peek, o).contains(&a) && !(distinct.contains(&w) && distinct.contains(&o.pc)) => {
                    wild.insert(a);
                }
                Some(_) => return Err(Refusal::SelfModUnmodelled(w)),
            }
        }
    }
    let mut reloc = Vec::new();
    for &pc in &region {
        if let Some(o) = addr16(peek, &cfg.n[&pc]) {
            let word = u16::from(peek(o)) | u16::from(peek(o.wrapping_add(1))) << 8;
            if (anchor..code_end).contains(&word) && !wild.contains(&o) && !wild.contains(&o.wrapping_add(1)) {
                reloc.push(o - anchor);
            }
        }
    }
    let ctx = Ctx { peek, wild };

    // ---- one symbolic run of the pass ----
    let n_outs = all.iter().filter(|p| cfg.n[p].port_out.is_some()).count();
    let mut pass_mem = BTreeSet::new();
    let mut pass_writes = 0u32;
    for pc in &all {
        let i = &cfg.n[pc];
        pass_writes |= i.writes;
        pass_mem.extend(abs_writes(peek, i));
    }
    let mut start = State::entry();
    start.stack = seed.clone();
    let mut conv = Conv {
        ctx: &ctx,
        anchor,
        code_end,
        pass_mem,
        pass_writes,
        strict: false,
        restored: 0,
    };
    // the registers the pass gives back: from a run that takes every register as kept
    if let Ok(r) = run_pass(cfg, &ctx, &conv, &items, start.clone(), true) {
        if r.diamonds.is_empty() && !r.end.stack_lost {
            conv.restored = r.outs.iter().fold(r.end.unchanged(), |m, o| m & o.kept);
        }
    }
    conv.strict = !(n_outs == 1 && matches!(items.first(), Some(Item::Op(pc)) if *pc == out));
    let conv = conv;
    // values from the first traversal (registers as at the pass's start); bits and ports from
    // later ones, when what the pass's tail computes is in place
    let run = |st: State, first: bool| run_pass(cfg, &ctx, &conv, &items, st, first);
    let r0 = run(start, true)?;
    let r1 = run(r0.end.clone(), false)?;
    let r2 = run(r1.end.clone(), false)?;
    let r3 = run(r2.end.clone(), false)?;
    // the pass leaves on the stack what it started with: the values it pushes at its end are
    // those its POPs at the start take
    if [&r0, &r1, &r2, &r3].iter().any(|r| r.end.stack_lost) || r0.end.stack.iter().map(|p| (p.hi.src, p.lo.src, p.base, p.off)).ne(seed.iter().map(|p| (p.hi.src, p.lo.src, p.base, p.off))) {
        return Err(un(Why::StackOp, out));
    }
    // and the pairs its leading POPs take are those the pass starts with: each pushed pair holds,
    // at the pass's end, the value it had at the start (else the POP gives the pair from before,
    // which the model would take for the pair as the pass found it)
    let key = |p: &Pushed| (p.hi.src, p.lo.src, p.base, p.off);
    if open.iter().zip(&seed).any(|(&p, s)| key(&r0.end.pushed(p)) != key(s)) {
        return Err(un(Why::StackOp, out));
    }
    let pcs = |r: &Run| r.outs.iter().map(|o| o.pc).collect::<Vec<_>>();
    if pcs(&r0) != pcs(&r1) || pcs(&r1) != pcs(&r2) || pcs(&r2) != pcs(&r3) {
        return Err(un(Why::OutsVary, out));
    }
    let t = r0.t.clone();
    let outs: Vec<Out> = r1.outs.iter().zip(&r0.outs).map(|(o, o0)| Out { pc: o.pc, bit: o.bit, t: o0.t.clone(), item: o.item, kept: o0.kept }).collect();
    let levels: Vec<Bit> = [&r1, &r2, &r3].iter().flat_map(|r| r.outs.iter().map(|o| o.bit)).collect();
    let (delays, diamonds) = (&r0.delays, &r1.diamonds);
    // ---- the family and its channels ----
    let in_pass = |site: u16| outs.iter().any(|o| o.pc == site) || diamonds.iter().any(|d| d.zero.outs.iter().any(|o| o.0 == site));
    if !in_pass(out) {
        return Err(un(Why::OutOffPath, out));
    }
    let (family, channels, pass_t, pos, flips) = if !diamonds.is_empty() {
        let channels = flips_channels(cfg, peek, &conv, &all, &outs, diamonds, &r0.diamonds, &t, out)?;
        let n = channels.len();
        (Family::Flips, channels, 0, vec![0; n], true)
    } else if !outs.is_empty() && outs.iter().all(|o| matches!(o.bit, Bit::Acc { .. })) {
        let (channels, p, pos) = slots_channels(&conv, &all, cfg, &r1.end, &outs, &t, out)?;
        (Family::Slots, channels, p, pos, false)
    } else {
        (Family::Square, vec![square_channel(&conv, &levels, &outs, delays, &t, out)?], 0, vec![0], false)
    };
    if channels.len() > SND_TONAL || channels.iter().map(|c| c.sites.len()).sum::<usize>() > SND_LEARNED_SITES_MAX {
        return Err(un(Why::TooManyChannels, out));
    }
    // the setup between notes makes no edges of its own: no speaker OUT, and the level the pass
    // carries comes back as the pass would leave it, or set (never flipped: a hidden channel)
    let shared: Vec<Loc> = diamonds
        .iter()
        .flat_map(|d| d.zero.outs.iter())
        .filter_map(|o| match o.1 {
            Bit::E { loc, .. } => Some(loc),
            _ => None,
        })
        .collect();
    for (test, ps) in tests.iter().zip(&paths) {
        let level_mem = level_stores(cfg, peek, test.3);
        let setup = Setup { cfg, ctx: &ctx, conv: &conv, items: &items, start: &r0.end, test: test.0, shared: &shared, level_mem };
        for p in ps {
            setup.keeps_level(p).map_err(|w| un(w, test.3))?;
        }
    }

    // ---- the note's timing ----
    let ends: Vec<Vec<(u16, State)>> = paths.iter().map(|ps| ps.iter().map(|p| (p.reentry, run_path(cfg, &ctx, p))).collect()).collect();
    let site_items: Vec<usize> = outs.iter().map(|o| o.item).chain(diamonds.iter().map(|d| d.item)).collect();
    let duration = match (tests.as_slice(), ends.as_slice()) {
        ([test], [ends]) => duration(cfg, peek, &conv, &items, &site_items, *test, ends, &t).map(|d| DurationModel { carry: carried[0], ..d }),
        _ => None,
    };

    // ---- the music player's evidence: inputs read through a pointer the setup advances ----
    let mut inputs: Vec<Input> = channels
        .iter()
        .flat_map(|c| c.half_period.iter())
        .filter_map(|t| match t {
            Term::Lin { x, .. } | Term::Recip { x, .. } => Some(*x),
            _ => None,
        })
        .collect();
    inputs.extend(duration.as_ref().and_then(|d| d.counts_down_from));
    let table_walk = ends.iter().flatten().any(|(_, end)| inputs.iter().any(|&x| walked(end, x, anchor)));

    // ---- where the memory is not to be read: outside the pass ----
    let lo = *own_distinct.first().ok_or(un(Why::PassShape, out))?;
    let hi = own_distinct.iter().map(|p| u32::from(*p) + u32::from(cfg.n[p].len)).max().unwrap_or(0);
    let bytes: u32 = own_distinct.iter().map(|p| u32::from(cfg.n[p].len)).sum();
    if hi - u32::from(lo) != bytes {
        return Err(un(Why::PassNotContiguous, lo));
    }
    let hi = hi as u16;
    let mut stale = Some((hi - anchor, lo.wrapping_sub(hi)));
    // a routine its notes do not loop back into (called once a note or a burst: frames often
    // end outside it) whose formulas read only registers at the site's last edge and bytes the
    // signature fixes: nothing of them goes stale between notes, and the registers at the frame's
    // end say nothing of the note's timing
    let fixed = |x: &Input| match *x {
        Input::Reg(_) => true,
        Input::Rel(o) => !ctx.wild.contains(&anchor.wrapping_add(o)),
        Input::Rel16(o) => !ctx.wild.contains(&anchor.wrapping_add(o)) && !ctx.wild.contains(&anchor.wrapping_add(o).wrapping_add(1)),
        _ => false,
    };
    let stable = channels.iter().flat_map(|c| &c.half_period).all(|t| match t {
        Term::Const(_) => true,
        Term::Lin { x, .. } | Term::Recip { x, .. } => fixed(x),
        Term::Prod { x, y, .. } => fixed(x) && fixed(y),
    });
    let mut duration = duration;
    if !looped && stable {
        stale = None;
        duration = None;
    }
    Ok(EngineModel {
        anchor,
        code_end,
        family,
        channels,
        pass_t,
        pos,
        flips,
        duration,
        stale,
        table_walk,
        wild: ctx.wild.iter().map(|a| a - anchor).collect(),
        reloc,
        stack: uses_stack || !cfg.callret.is_empty(),
        pointer_reads,
    })
}

/// The straight runs of code into the pass from elsewhere: for each instruction of the pass
/// (`main`) entered from outside the pass's code (`main`, `inside`, `carries`) and the setup paths
/// (`paths`), the run of instructions before it, back to one with other than a single
/// predecessor running into it (at most `PATH_LEN`).
fn lead_ins(cfg: &Cfg, main: &BTreeSet<u16>, inside: &BTreeSet<u16>, carries: &[u16], paths: &[Vec<SetupPath>]) -> Vec<SetupPath> {
    const PATH_LEN: usize = 256;
    let known: BTreeSet<u16> = paths.iter().flatten().flat_map(|p| p.nodes.iter().copied()).chain(carries.iter().copied()).collect();
    let outside = |p: u16| !main.contains(&p) && !inside.contains(&p) && !known.contains(&p);
    // p runs into q: falls through or jumps, with no other way out
    let runs_into = |p: u16, q: u16| {
        cfg.n.get(&p).is_some_and(|i| match i.flow {
            Flow::Next => i.t == i.t_taken && i.pc.checked_add(u16::from(i.len)) == Some(q),
            Flow::Jump(t) => t == q,
            _ => false,
        })
    };
    let mut leads = Vec::new();
    for &pc in main {
        for &p in cfg.preds.get(&pc).map(Vec::as_slice).unwrap_or(&[]) {
            if !outside(p) || !runs_into(p, pc) {
                continue;
            }
            let mut nodes = vec![p];
            while nodes.len() < PATH_LEN {
                let first = nodes[0];
                match cfg.preds.get(&first).map(Vec::as_slice) {
                    Some(&[q]) if outside(q) && runs_into(q, first) && !nodes.contains(&q) => nodes.insert(0, q),
                    _ => break,
                }
            }
            leads.push(SetupPath { nodes, reentry: pc });
        }
    }
    leads
}

/// Whether input `x` was, at the end of a setup path, read through a pointer the path advanced:
/// the pointer still holds the value it was loaded through (the same base), moved since.
fn walked(end: &State, x: Input, anchor: u16) -> bool {
    let adv = |pair: u8| pair_index(pair).is_some_and(|i| end.advanced >> i & 1 != 0);
    let through = |s: Src| matches!(s, Src::Idx { pair, base, .. } if adv(pair) && pair_index(pair).is_some_and(|i| end.base[i] == base));
    match x {
        Input::Indexed(pair, _) => adv(pair),
        Input::Reg(r) if r < 8 || r == 14 => through(end.v[if r == 14 { 8 } else { r as usize }].src),
        Input::Reg(r) => matches!(end.word(r), W::Idx { pair, base, d } if through(Src::Idx { pair, base, d })),
        Input::Abs(a) | Input::Abs16(a) => through(end.read_mem(a).src),
        Input::Rel(o) | Input::Rel16(o) => through(end.read_mem(anchor.wrapping_add(o)).src),
    }
}

/// One channel toggled by every speaker `OUT` of the pass: half period = the pass over the
/// number of `OUT`s.
fn square_channel(conv: &Conv, levels: &[Bit], outs: &[Out], delays: &[DelayRun], t: &Sum, out: u16) -> Result<ChannelModel, Refusal> {
    let un = Refusal::Unmodelled;
    if outs.is_empty() || outs.iter().any(|o| matches!(o.bit, Bit::Acc { .. })) {
        return Err(un(Why::PhaseShape, out));
    }
    // a level that toggles only for one value of a bit in memory: silent for the other
    let cond = if toggles(levels) { None } else { Some(toggle_condition(levels).ok_or(un(Why::NoToggle, out))?) };
    // the partition: one reload counter feeds every half period; two registers fed from one
    // source in different half periods may be two channels in unison
    let fed: Vec<(usize, u8, Input)> = delays
        .iter()
        .filter_map(|d| match d.value {
            Value::In(x) => Some((d.seg, d.counter, x)),
            Value::K(_) => None,
        })
        .collect();
    for &(s, r, x) in &fed {
        if fed.iter().any(|&(s2, r2, x2)| x2 == x && r2 != r && s2 != s) {
            return Err(Refusal::Ambiguous(out));
        }
    }
    if outs.len() > 1 {
        let seg = |s: usize| -> BTreeSet<(u8, Input)> {
            fed.iter().filter(|f| f.0 == s || (s == outs.len() && f.0 == 0)).map(|f| (f.1, f.2)).collect()
        };
        let first = seg(1);
        if (2..=outs.len()).any(|s| seg(s) != first) {
            return Err(un(Why::DutyCycle, out)); // the halves count different things
        }
    }
    let half = t.div(outs.len() as i64).ok_or(un(Why::UnevenHalves, out))?;
    let mut sites: Vec<u16> = outs.iter().map(|o| o.pc).collect();
    sites.sort_unstable();
    let silent_if_bit = match cond {
        Some((Loc::M(a), bit, want)) => Some((conv.mem(a, false).ok_or(un(Why::NoToggle, out))?, 1u8 << bit, !want)),
        Some(_) => return Err(un(Why::NoToggle, out)),
        None => None,
    };
    Ok(ChannelModel { sites, half_period: half.terms(), silent_if_zero: None, silent_if_bit })
}

/// A channel per speaker `OUT`, each a phase accumulator's top bit.
fn slots_channels(
    conv: &Conv,
    all: &[u16],
    cfg: &Cfg,
    end: &State,
    outs: &[Out],
    t: &Sum,
    out: u16,
) -> Result<(Vec<ChannelModel>, u16, Vec<u16>), Refusal> {
    let un = Refusal::Unmodelled;
    let shape = |pc: u16| un(Why::PhaseShape, pc);
    let p = t.konst().filter(|&p| (1..=0xFFFF).contains(&p)).ok_or(un(Why::PassTimeVaries, out))?;
    let mut order: Vec<&Out> = outs.iter().collect();
    order.sort_by_key(|o| o.pc);
    let mut channels = Vec::new();
    let mut pos = Vec::new();
    let mut used = BTreeSet::new();
    let t0 = order[0].t.konst().ok_or(un(Why::PassTimeVaries, out))?;
    for o in &order {
        let Bit::Acc { id, .. } = o.bit else { return Err(shape(o.pc)) };
        let acc = end.accs.get(id as usize).ok_or(shape(o.pc))?;
        if !used.insert(id) {
            return Err(shape(o.pc));
        }
        // the phase is carried from pass to pass, nothing else writing it
        let carried = match acc.base {
            W::Mem(a) => {
                let stores = all.iter().filter(|pc| abs_writes(conv.ctx.peek, &cfg.n[pc]).iter().any(|&w| w == a || w == a.wrapping_add(1))).count();
                acc.stored >= 1 && stores == 1
            }
            W::Entry(pair) => end.word(pair) == W::Acc(id),
            _ => false,
        };
        if !carried {
            return Err(shape(o.pc));
        }
        let half = match conv.word(acc.step).ok_or(un(Why::UnnamedInput, o.pc))? {
            Value::K(0) => return Err(shape(o.pc)),
            Value::K(s) => Term::Const((HALF_PHASE * p + s / 2) / s),
            Value::In(x) => Term::Recip { num_t: HALF_PHASE * p, x },
        };
        channels.push(ChannelModel { sites: vec![o.pc], half_period: vec![half], silent_if_zero: None, silent_if_bit: None });
        pos.push((o.t.konst().ok_or(un(Why::PassTimeVaries, o.pc))? - t0).rem_euclid(p) as u16);
    }
    Ok((channels, p as u16, pos))
}

/// A channel per diamond: its counter reloaded every n passes flips a level all channels share
/// and `OUT`s it.
#[allow(clippy::too_many_arguments)]
fn flips_channels(
    cfg: &Cfg,
    peek: &dyn Fn(u16) -> u8,
    conv: &Conv,
    all: &[u16],
    outs: &[Out],
    diamonds: &[DiamondRun],
    first: &[DiamondRun],
    t: &Sum,
    out: u16,
) -> Result<Vec<ChannelModel>, Refusal> {
    let un = Refusal::Unmodelled;
    let shape = |pc: u16| un(Why::FlipsShape, pc);
    if !outs.is_empty() {
        return Err(shape(out));
    }
    let p = t.konst().ok_or(un(Why::PassTimeVaries, out))?;
    let mut level: Option<Loc> = None;
    let mut fed: Vec<(u8, Value, u16, i64)> = Vec::new();
    let mut arms: BTreeSet<u16> = BTreeSet::new();
    if diamonds.len() != first.len() {
        return Err(shape(out));
    }
    // unbalanced arms: each reload would shift the other channels' flips (their half periods
    // then depend on each other's counts); one channel alone just has a longer flipping pass
    if diamonds.len() > 1 {
        if let Some(d) = diamonds.iter().find(|d| d.extra() != 0) {
            return Err(un(Why::UnbalancedFlips, d.pc));
        }
    }
    for (d, d0) in diamonds.iter().zip(first) {
        arms.extend(&d.nodes);
        let (site, bit) = match (d.zero.outs.as_slice(), d.skip.outs.is_empty()) {
            ([o], true) => *o,
            _ => return Err(shape(d.pc)),
        };
        let Bit::E { loc, bit: 4, inv: true } = bit else { return Err(shape(site)) };
        if level.is_some_and(|l| l != loc)
            || bit_at(&d.zero.end, loc, 4) != (Bit::E { loc, bit: 4, inv: true })
            || bit_at(&d.skip.end, loc, 4) != (Bit::E { loc, bit: 4, inv: false })
        {
            return Err(shape(site));
        }
        level = Some(loc);
        // the zero arm reloads the counter; nothing else but its decrement writes it
        if d.zero.writes & bits_of(d.counter) == 0 {
            return Err(shape(d.pc));
        }
        let (_, idiom) = cfg.zero_idiom(peek, &cfg.n[&d.pc]).ok_or(shape(d.pc))?;
        if all.iter().any(|pc| cfg.n[pc].writes & bits_of(d.counter) != 0 && !idiom.contains(pc) && !d.nodes.contains(pc)) {
            return Err(un(Why::CounterReloaded, d.pc));
        }
        // the reload, from the first traversal (registers as at the pass's start)
        let end0 = &d0.zero.end;
        let n = if d.counter >= 8 { conv.word(end0.word(d.counter)) } else { conv.byte(end0.v[d.counter as usize].src) };
        fed.push((d.counter, n.ok_or(un(Why::UnnamedInput, d.pc))?, site, d.extra()));
    }
    // the level changes only in the channels' flips
    if let Some(l) = level {
        if all.iter().any(|pc| !arms.contains(pc) && writes_loc(peek, &cfg.n[pc], l)) {
            return Err(un(Why::LevelWrittenElsewhere, out));
        }
    }
    for &(r, v, _, _) in &fed {
        if fed.iter().any(|&(r2, v2, _, _)| r2 == r && v2 != v) || fed.iter().filter(|f| f.0 == r).count() > 1 {
            return Err(shape(out)); // one counter, two diamonds
        }
        if matches!(v, Value::In(_)) && fed.iter().any(|&(r2, v2, _, _)| v2 == v && r2 != r) {
            return Err(Refusal::Ambiguous(out));
        }
    }
    fed.sort_by_key(|f| f.2);
    // n passes from flip to flip: n - 1 without the reload and the one with it
    Ok(fed
        .into_iter()
        .map(|(r, v, site, extra)| {
            let mut s = Sum::default();
            s.add_count(p, extra, v, r >= 8);
            ChannelModel { sites: vec![site], half_period: s.terms(), silent_if_zero: None, silent_if_bit: None }
        })
        .collect())
}

/// The note's timing from the pass's one zero test: its counter (written only by its decrement,
/// [`analyse`] checks) counts passes down from a start the setup paths load; known only when
/// the paths agree, the counter is decremented after every site (so it is the note's start at
/// the first) and a pass does not depend on it.
#[allow(clippy::too_many_arguments)]
fn duration(
    cfg: &Cfg,
    peek: &dyn Fn(u16) -> u8,
    conv: &Conv,
    items: &[Item],
    site_items: &[usize],
    test: (usize, u16, u8, u16),
    ends: &[(u16, State)],
    pass: &Sum,
) -> Option<DurationModel> {
    let (_, pc, c, _) = test;
    let (_, idiom) = cfg.zero_idiom(peek, &cfg.n[&pc])?;
    let related = |r: u8| bits_of(r) & bits_of(c) != 0;
    if pass.lin.iter().any(|&(x, _)| matches!(x, Input::Reg(r) | Input::Indexed(r, _) if related(r))) {
        return None;
    }
    let (reentry, _) = ends.first()?;
    if ends.iter().any(|(r, _)| r != reentry) {
        return None;
    }
    let len = items.len();
    let start = items.iter().position(|it| it.pc() == *reentry)?;
    let at = |k: usize| (k + len - start) % len;
    let dec = items.iter().position(|it| it.pc() == idiom[0])?;
    if site_items.iter().any(|&s| at(s) >= at(dec)) {
        return None;
    }
    let first = conv.start(&ends[0].1, c)?;
    if ends.iter().any(|(_, e)| conv.start(e, c) != Some(first)) {
        return None;
    }
    Some(DurationModel { counter: Input::Reg(c), per_unit_t: pass.terms(), counts_down_from: Some(first), carry: false })
}
