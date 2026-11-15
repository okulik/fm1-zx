// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! From a pass to a model: the routine's extent and signature, one symbolic run of the pass,
//! the family and channels it shows, their formulas, the note's timing and the table walk.

use super::cfg::{bits_of, setup_paths, undecoded, Cfg, Item, SetupPath};
use super::sym::{pair_index, slot_reg, Bit, Loc, Src, State, W};
use super::{ChannelModel, Ctx, DurationModel, EngineModel, Family, Input, Refusal, Term};
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
        !self.strict || self.pass_writes & bits_of(reg) == 0
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

struct DelayRun {
    counter: u8,
    value: Value,
    /// Speaker `OUT`s before it in the pass.
    seg: usize,
}

/// Bit 4 of what `i` writes to the speaker, if it is a speaker `OUT` (an even port).
fn speaker(st: &State, i: &Insn, ctx: &Ctx, out: u16) -> Result<Option<Bit>, Refusal> {
    let refuse = |pc: u16| if pc == out { Refusal::NotSpeaker(pc) } else { Refusal::UnknownShape(pc) };
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
fn run_pass(cfg: &Cfg, ctx: &Ctx, conv: &Conv, items: &[Item], mut st: State, out: u16, first: bool) -> Result<Run, Refusal> {
    let shape = Refusal::UnknownShape;
    let mut r = Run { outs: Vec::new(), delays: Vec::new(), diamonds: Vec::new(), t: Sum::default(), end: State::entry() };
    let spk = |s: &State, i: &Insn| match speaker(s, i, ctx, out) {
        Err(_) if first => Ok(Some(Bit::X)),
        x => x,
    };
    for (k, it) in items.iter().enumerate() {
        match it {
            Item::Op(pc) | Item::Test { pc, .. } => {
                let i = &cfg.n[pc];
                if let Some(bit) = spk(&st, i)? {
                    r.outs.push(Out { pc: *pc, bit, t: r.t.clone(), item: k });
                }
                st.step(i, ctx);
                r.t.k += match it {
                    Item::Test { stay_taken: true, .. } => i64::from(i.t_taken),
                    _ => i64::from(i.t),
                };
            }
            Item::Delay(h) => {
                let d = &cfg.delays[h];
                if first {
                    let wide = d.counter >= 8;
                    let value = if wide { conv.word(st.word(d.counter)) } else { conv.byte(st.v[d.counter as usize].src) };
                    let value = value.ok_or(shape(*h))?;
                    r.t.add_count(d.per, d.konst, value, wide);
                    r.delays.push(DelayRun { counter: d.counter, value, seg: r.outs.len() });
                }
                for p in &d.nodes {
                    st.clobber(&cfg.n[p]);
                }
                st.zero(d.counter);
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
                if ta.time != fa.time {
                    return Err(shape(*pc)); // unbalanced: each reload would shift the others
                }
                r.t.k += ta.time;
                st = ta.end.join(&fa.end);
                st.forget_bits();
                let (zero, skip) = if *zero_taken { (ta, fa) } else { (fa, ta) };
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
    let shape = Refusal::UnknownShape;
    // ---- the pass's instructions ----
    let mut main = BTreeSet::new();
    let mut inside = BTreeSet::new();
    let mut all: Vec<u16> = Vec::new();
    for it in &items {
        main.insert(it.pc());
        match it {
            Item::Op(pc) | Item::Test { pc, .. } => all.push(*pc),
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
    if distinct.len() != all.len() {
        return Err(shape(out));
    }
    if !distinct.contains(&out) {
        return Err(shape(out));
    }
    for &pc in &all {
        let i = &cfg.n[&pc];
        // whole-T formulas cannot carry the contended range's delays
        let contended = |a: u16, n: u16| (u32::from(a) + u32::from(n) > 0x4000) && a < 0x8000;
        let mem_contended = [i.mem_read, i.mem_write].iter().any(|m| matches!(m, Some(MemRef::Abs(a)) if contended(*a, 2)));
        if contended(pc, i.len.into()) || mem_contended {
            return Err(shape(pc));
        }
        // stores through pointers and the stack: not modelled
        let stack = i.reads & 1 << 13 != 0 || i.writes & 1 << 13 != 0;
        if stack || matches!(i.mem_write, Some(MemRef::Reg16(_) | MemRef::Indexed(..) | MemRef::Sp)) {
            return Err(shape(pc));
        }
    }

    // a zero test leaving the pass counts down to it: nothing else in the pass writes its counter
    // (else the side the pass takes is not known)
    for it in &items {
        if let Item::Test { pc, counter, .. } = it {
            let (_, idiom) = cfg.zero_idiom(peek, &cfg.n[pc]).ok_or(shape(*pc))?;
            if all.iter().any(|p| cfg.n[p].writes & bits_of(*counter) != 0 && !idiom.contains(p)) {
                return Err(shape(*pc));
            }
        }
    }

    // ---- the setup paths, the routine's extent ----
    let tests: Vec<(usize, u16, u8, u16)> = items
        .iter()
        .enumerate()
        .filter_map(|(k, it)| match it {
            Item::Test { pc, counter, exit, .. } => Some((k, *pc, *counter, *exit)),
            _ => None,
        })
        .collect();
    let paths: Vec<Vec<SetupPath>> = tests.iter().map(|t| setup_paths(cfg, t.3, &main, &inside)).collect();
    let mut region: BTreeSet<u16> = distinct.clone();
    region.extend(paths.iter().flatten().flat_map(|p| p.nodes.iter().copied()));
    let anchor = *region.first().ok_or(shape(out))?;
    let mut end = 0u32;
    for &pc in &region {
        let i = &cfg.n[&pc];
        if u32::from(pc) < end {
            return Err(shape(pc)); // overlapping instructions
        }
        if undecoded(peek, i) {
            return Err(Refusal::Undecoded(pc));
        }
        end = u32::from(pc) + u32::from(i.len);
    }
    let code_end = u16::try_from(end).map_err(|_| shape(out))?;
    if code_end - anchor > SND_LEARNED_LEN_MAX {
        return Err(shape(anchor));
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
    let conv = Conv {
        ctx: &ctx,
        anchor,
        code_end,
        pass_mem,
        pass_writes,
        strict: !(n_outs == 1 && matches!(items.first(), Some(Item::Op(pc)) if *pc == out)),
    };
    // values from the first traversal (registers as at the pass's start); bits and ports from
    // later ones, when what the pass's tail computes is in place
    let run = |st: State, first: bool| run_pass(cfg, &ctx, &conv, &items, st, out, first);
    let r0 = run(State::entry(), true)?;
    let r1 = run(r0.end.clone(), false)?;
    let r2 = run(r1.end.clone(), false)?;
    let r3 = run(r2.end.clone(), false)?;
    let pcs = |r: &Run| r.outs.iter().map(|o| o.pc).collect::<Vec<_>>();
    if pcs(&r0) != pcs(&r1) || pcs(&r1) != pcs(&r2) || pcs(&r2) != pcs(&r3) {
        return Err(shape(out));
    }
    let t = r0.t.clone();
    let outs: Vec<Out> = r1.outs.iter().zip(&r0.outs).map(|(o, o0)| Out { pc: o.pc, bit: o.bit, t: o0.t.clone(), item: o.item }).collect();
    let levels: Vec<Bit> = [&r1, &r2, &r3].iter().flat_map(|r| r.outs.iter().map(|o| o.bit)).collect();
    let (delays, diamonds) = (&r0.delays, &r1.diamonds);
    // ---- the family and its channels ----
    let in_pass = |site: u16| outs.iter().any(|o| o.pc == site) || diamonds.iter().any(|d| d.zero.outs.iter().any(|o| o.0 == site));
    if !in_pass(out) {
        return Err(Refusal::NotSpeaker(out));
    }
    let (family, channels, pass_t, pos, flips) = if !diamonds.is_empty() {
        let channels = flips_channels(cfg, peek, &conv, &all, &outs, diamonds, &r0.diamonds, &t, out)?;
        let n = channels.len();
        (Family::Flips, channels, 0, vec![0; n], true)
    } else if !outs.is_empty() && outs.iter().all(|o| matches!(o.bit, Bit::Acc { .. })) {
        let (channels, p, pos) = slots_channels(&conv, &all, cfg, &r1.end, &outs, &t, out)?;
        (Family::Slots, channels, p, pos, false)
    } else {
        (Family::Square, vec![square_channel(&levels, &outs, delays, &t, out)?], 0, vec![0], false)
    };
    if channels.len() > SND_TONAL || channels.iter().map(|c| c.sites.len()).sum::<usize>() > SND_LEARNED_SITES_MAX {
        return Err(shape(out));
    }

    // ---- the note's timing ----
    let ends: Vec<Vec<(u16, State)>> = paths.iter().map(|ps| ps.iter().map(|p| (p.reentry, run_path(cfg, &ctx, p))).collect()).collect();
    let site_items: Vec<usize> = outs.iter().map(|o| o.item).chain(diamonds.iter().map(|d| d.item)).collect();
    let duration = match (tests.as_slice(), ends.as_slice()) {
        ([test], [ends]) => duration(cfg, peek, &conv, &items, &site_items, *test, ends, &t),
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
    let lo = *distinct.first().ok_or(shape(out))?;
    let hi = distinct.iter().map(|p| u32::from(*p) + u32::from(cfg.n[p].len)).max().unwrap_or(0);
    let bytes: u32 = distinct.iter().map(|p| u32::from(cfg.n[p].len)).sum();
    if hi - u32::from(lo) != bytes {
        return Err(shape(lo)); // other code inside the pass's range
    }
    let hi = hi as u16;
    Ok(EngineModel {
        anchor,
        code_end,
        family,
        channels,
        pass_t,
        pos,
        flips,
        duration,
        stale: Some((hi - anchor, lo.wrapping_sub(hi))),
        table_walk,
        wild: ctx.wild.iter().map(|a| a - anchor).collect(),
        reloc,
    })
}

/// Whether input `x` was, at the end of a setup path, loaded through a pointer the path
/// advanced.
fn walked(end: &State, x: Input, anchor: u16) -> bool {
    let adv = |pair: u8| pair_index(pair).is_some_and(|i| end.advanced >> i & 1 != 0);
    let through = |s: Src| matches!(s, Src::Idx { pair, .. } if adv(pair));
    match x {
        Input::Indexed(pair, _) => adv(pair),
        Input::Reg(r) if r < 8 || r == 14 => through(end.v[if r == 14 { 8 } else { r as usize }].src),
        Input::Reg(r) => matches!(end.word(r), W::Idx { pair, .. } if adv(pair)),
        Input::Abs(a) | Input::Abs16(a) => through(end.read_mem(a).src),
        Input::Rel(o) | Input::Rel16(o) => through(end.read_mem(anchor.wrapping_add(o)).src),
    }
}

/// One channel toggled by every speaker `OUT` of the pass: half period = the pass over the
/// number of `OUT`s.
fn square_channel(levels: &[Bit], outs: &[Out], delays: &[DelayRun], t: &Sum, out: u16) -> Result<ChannelModel, Refusal> {
    let shape = Refusal::UnknownShape;
    if outs.is_empty() || outs.iter().any(|o| matches!(o.bit, Bit::Acc { .. })) {
        return Err(shape(out));
    }
    if !toggles(levels) {
        return Err(shape(out));
    }
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
            return Err(shape(out)); // the halves count different things: a duty cycle
        }
    }
    let half = t.div(outs.len() as i64).ok_or(shape(out))?;
    let mut sites: Vec<u16> = outs.iter().map(|o| o.pc).collect();
    sites.sort_unstable();
    Ok(ChannelModel { sites, half_period: half.terms(), silent_if_zero: None })
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
    let shape = Refusal::UnknownShape;
    let p = t.konst().filter(|&p| (1..=0xFFFF).contains(&p)).ok_or(shape(out))?;
    let mut order: Vec<&Out> = outs.iter().collect();
    order.sort_by_key(|o| o.pc);
    let mut channels = Vec::new();
    let mut pos = Vec::new();
    let mut used = BTreeSet::new();
    let t0 = order[0].t.konst().ok_or(shape(out))?;
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
        let half = match conv.word(acc.step).ok_or(shape(o.pc))? {
            Value::K(0) => return Err(shape(o.pc)),
            Value::K(s) => Term::Const((HALF_PHASE * p + s / 2) / s),
            Value::In(x) => Term::Recip { num_t: HALF_PHASE * p, x },
        };
        channels.push(ChannelModel { sites: vec![o.pc], half_period: vec![half], silent_if_zero: None });
        pos.push((o.t.konst().ok_or(shape(o.pc))? - t0).rem_euclid(p) as u16);
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
    let shape = Refusal::UnknownShape;
    if !outs.is_empty() {
        return Err(shape(out));
    }
    let p = t.konst().ok_or(shape(out))?;
    let mut level: Option<Loc> = None;
    let mut fed: Vec<(u8, Value, u16)> = Vec::new();
    let mut arms: BTreeSet<u16> = BTreeSet::new();
    if diamonds.len() != first.len() {
        return Err(shape(out));
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
            return Err(shape(d.pc));
        }
        // the reload, from the first traversal (registers as at the pass's start)
        let end0 = &d0.zero.end;
        let n = if d.counter >= 8 { conv.word(end0.word(d.counter)) } else { conv.byte(end0.v[d.counter as usize].src) };
        fed.push((d.counter, n.ok_or(shape(d.pc))?, site));
    }
    // the level changes only in the channels' flips
    if let Some(l) = level {
        if all.iter().any(|pc| !arms.contains(pc) && writes_loc(peek, &cfg.n[pc], l)) {
            return Err(shape(out));
        }
    }
    for &(r, v, _) in &fed {
        if fed.iter().any(|&(r2, v2, _)| r2 == r && v2 != v) || fed.iter().filter(|f| f.0 == r).count() > 1 {
            return Err(shape(out)); // one counter, two diamonds
        }
        if matches!(v, Value::In(_)) && fed.iter().any(|&(r2, v2, _)| v2 == v && r2 != r) {
            return Err(Refusal::Ambiguous(out));
        }
    }
    fed.sort_by_key(|f| f.2);
    Ok(fed
        .into_iter()
        .map(|(r, v, site)| {
            let mut s = Sum::default();
            s.add_count(p, 0, v, r >= 8);
            ChannelModel { sites: vec![site], half_period: s.terms(), silent_if_zero: None }
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
    Some(DurationModel { counter: Input::Reg(c), per_unit_t: pass.terms(), counts_down_from: Some(first) })
}
