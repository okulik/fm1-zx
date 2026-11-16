// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The code around an `OUT` as a graph of instructions, its counted delay loops, and the walk
//! that finds the sound loop's pass through the `OUT` and the setup paths back into it.

use super::{Refusal, Why};
use crate::z80::{decode, Flow, Insn};
use std::collections::{BTreeMap, BTreeSet};

/// Bytes either side of the `OUT` the reader decodes.
const WINDOW: i32 = 512;
/// Most instructions of one arm of a branch.
const ARM_MAX: usize = 48;
/// Most items of a pass.
const PASS_MAX: usize = 400;
/// Most bytes of a delay loop.
const DELAY_MAX: u16 = 64;
/// Bytes of a called routine decoded from its start.
const CALLEE_REACH: i32 = 256;
/// Most calls the walk follows one inside another.
const CALLS_MAX: usize = 3;
/// Most instructions that leave the flags alone between a counter's `DEC` and its test.
const NEUTRAL_MAX: usize = 4;

pub(super) struct Cfg {
    /// The pass's instructions inside the calls the walk follows (their delay loops' too), and
    /// the `CALL`s and `RET`s it follows (set from the walk).
    pub inlined: BTreeSet<u16>,
    pub callret: BTreeSet<u16>,
    pub n: BTreeMap<u16, Insn>,
    pub preds: BTreeMap<u16, Vec<u16>>,
    /// Delay loops by their first instruction, and every instruction of one (to its head).
    pub delays: BTreeMap<u16, Delay>,
    pub in_delay: BTreeMap<u16, u16>,
}

/// An instruction the decoder only stood in for: an ED opcode with no instruction or a prefix
/// alone (decoded as NOPs).
pub(super) fn undecoded(peek: &dyn Fn(u16) -> u8, i: &Insn) -> bool {
    i.text == "NOP" && peek(i.pc) != 0
}

/// Where control can go after `i`: calls and restarts return (their code is not followed).
pub(super) fn succs(i: &Insn) -> Vec<u16> {
    let next = (u32::from(i.pc) + u32::from(i.len) <= 0xFFFF).then(|| i.pc + u16::from(i.len));
    let mut v: Vec<u16> = match i.flow {
        Flow::Next | Flow::Halt | Flow::Call(_) | Flow::CondCall(_) | Flow::Rst(_) | Flow::CondRet => next.into_iter().collect(),
        Flow::Jump(t) => vec![t],
        Flow::CondJump(t) | Flow::Djnz(t) => next.into_iter().chain([t]).collect(),
        Flow::Ret | Flow::JumpIndirect => vec![],
    };
    v.dedup();
    v
}

/// The instructions reachable from `out` within the window, and the code of the routines its
/// `CALL`s in RAM go to (up to 256 bytes on from each, past the window too).
pub(super) fn build(peek: &dyn Fn(u16) -> u8, out: u16) -> Cfg {
    let lo = (i32::from(out) - WINDOW).max(0);
    let hi = (i32::from(out) + WINDOW).min(0x10000);
    let mut n = BTreeMap::new();
    let mut todo = vec![(out, lo, hi)];
    while let Some((pc, lo, hi)) = todo.pop() {
        if n.contains_key(&pc) || i32::from(pc) < lo {
            continue;
        }
        let i = decode(peek, pc);
        if i32::from(pc) + i32::from(i.len) > hi {
            continue;
        }
        todo.extend(succs(&i).into_iter().map(|s| (s, lo, hi)));
        if let Flow::Call(t) = i.flow {
            if t >= 0x4000 {
                todo.push((t, i32::from(t), (i32::from(t) + CALLEE_REACH).min(0x10000)));
            }
        }
        n.insert(pc, i);
    }
    let mut preds: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for (pc, i) in &n {
        for s in succs(i) {
            preds.entry(s).or_default().push(*pc);
        }
    }
    let mut cfg = Cfg { inlined: BTreeSet::new(), callret: BTreeSet::new(), n, preds, delays: BTreeMap::new(), in_delay: BTreeMap::new() };
    cfg.find_delays(peek);
    cfg
}

/// Bytes before the pass where a call or jump into the routine is looked for.
const CALLER_REACH: u16 = 256;

impl Cfg {
    /// Adds the code that leads into the pass from where the routine is entered: the targets of
    /// the `CALL nn` and `JP nn` (conditional too) anywhere in RAM that lie up to 256 bytes before
    /// the pass's first instruction, decoded forward within the window, each kept only if it
    /// reaches the pass (`main`). A routine called once a note or a burst has its setup there,
    /// before the loop, which the walk from the `OUT` never reaches.
    pub(super) fn add_callers(&mut self, peek: &dyn Fn(u16) -> u8, out: u16, main: &BTreeSet<u16>) {
        let Some(&first) = main.first() else { return };
        let from = first.saturating_sub(CALLER_REACH);
        let mut targets = BTreeSet::new();
        for a in 0x4000..=0xFFFDu16 {
            let op = peek(a);
            if op == 0xCD || op == 0xC3 || op & 0xC7 == 0xC4 || op & 0xC7 == 0xC2 {
                let t = u16::from(peek(a + 1)) | u16::from(peek(a + 2)) << 8;
                if (from..first).contains(&t) && !self.n.contains_key(&t) {
                    targets.insert(t);
                }
            }
        }
        let lo = (i32::from(out) - WINDOW).max(0);
        let hi = (i32::from(out) + WINDOW).min(0x10000);
        let mut added = false;
        for t in targets {
            if self.n.contains_key(&t) {
                continue;
            }
            let mut found: BTreeMap<u16, Insn> = BTreeMap::new();
            let mut todo = vec![t];
            let mut reaches = false;
            while let Some(pc) = todo.pop() {
                if self.n.contains_key(&pc) {
                    reaches |= main.contains(&pc);
                    continue;
                }
                if found.contains_key(&pc) || i32::from(pc) < lo {
                    continue;
                }
                let i = decode(peek, pc);
                if i32::from(pc) + i32::from(i.len) > hi {
                    continue;
                }
                todo.extend(succs(&i));
                found.insert(pc, i);
            }
            if reaches {
                self.n.extend(found);
                added = true;
            }
        }
        if added {
            self.preds.clear();
            for (pc, i) in &self.n {
                for s in succs(i) {
                    self.preds.entry(s).or_default().push(*pc);
                }
            }
        }
    }
}

/// A counted delay loop: from `head` to the branch back at `branch`, `per * n + konst` T-states
/// for a counter of `n` at entry (0: 256, or 65,536 for a pair), the counter 0 after it.
///
/// Nested (`outer`): that loop is the body of an outer one counted by a second register (the
/// inner loop, then straight code to the outer counter's zero test back to `head`). The inner
/// counter is 0 after its first run, so each later run counts it from 0 (256, or 65,536): the
/// outer loop adds `per2 * n2 + konst2` T-states for an outer count of `n2`.
#[derive(Clone, Debug)]
pub(super) struct Delay {
    pub head: u16,
    pub exit: u16,
    /// Register number (as [`crate::prog`]): 0, 2-7 or 8-10.
    pub counter: u8,
    pub per: i64,
    pub konst: i64,
    pub nodes: Vec<u16>,
    /// The outer counter, `per2` and `konst2`.
    pub outer: Option<(u8, i64, i64)>,
}

/// `DEC r` (one byte): r's register number.
fn dec8(op: u8) -> Option<u8> {
    match op {
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D => Some((op >> 3) + 2),
        0x3D => Some(0),
        _ => None,
    }
}

/// `DEC rr` (BC, DE, HL): the pair's number and its (high, low) register numbers.
fn dec16(op: u8) -> Option<(u8, u8, u8)> {
    match op {
        0x0B => Some((8, 2, 3)),
        0x1B => Some((9, 4, 5)),
        0x2B => Some((10, 6, 7)),
        _ => None,
    }
}

/// `LD A,r` / `OR r` for r B-L: r's register number.
fn ld_a(op: u8) -> Option<u8> {
    (0x78..=0x7D).contains(&op).then(|| op - 0x78 + 2)
}
fn or_r(op: u8) -> Option<u8> {
    (0xB0..=0xB5).contains(&op).then(|| op - 0xB0 + 2)
}

/// Register bits of register `r` in the decoder's masks (8-bit ones alone; a pair with its
/// halves).
pub(super) fn bits_of(r: u8) -> u32 {
    match r {
        8 => 1 << 8 | 1 << 2 | 1 << 3,
        9 => 1 << 9 | 1 << 4 | 1 << 5,
        10 => 1 << 10 | 1 << 6 | 1 << 7,
        _ => 1 << r,
    }
}

impl Cfg {
    fn straight(&self, peek: &dyn Fn(u16) -> u8, pc: u16) -> Option<&Insn> {
        let i = self.n.get(&pc)?;
        (i.flow == Flow::Next && i.t == i.t_taken && i.port_out.is_none() && i.mem_write.is_none() && !undecoded(peek, i)).then_some(i)
    }

    /// The idiom that sets the flags a zero test `b` (`JR`/`JP` `Z`/`NZ`, or `DJNZ`) reads, as
    /// the instructions just before it: `DEC r`, or `DEC rr; LD A,h; OR l` (either half first).
    /// Up to four instructions that leave the flags alone ([`flag_neutral`]) may stand between
    /// the idiom and the test. Gives the counter's register number and the idiom's instructions.
    pub(super) fn zero_idiom(&self, peek: &dyn Fn(u16) -> u8, b: &Insn) -> Option<(u8, Vec<u16>)> {
        if let Flow::Djnz(_) = b.flow {
            return Some((2, vec![b.pc]));
        }
        if !matches!(peek(b.pc), 0x20 | 0x28 | 0xC2 | 0xCA) {
            return None;
        }
        // the unique instruction before `pc`, running into it
        let before = |pc: u16| -> Option<&Insn> {
            match self.preds.get(&pc).map(Vec::as_slice) {
                Some([p]) => self.n.get(p).filter(|i| i.flow == Flow::Next && i.pc.wrapping_add(u16::from(i.len)) == pc),
                _ => None,
            }
        };
        let mut p1 = before(b.pc)?;
        for _ in 0..NEUTRAL_MAX {
            if !flag_neutral(peek, p1) {
                break;
            }
            p1 = before(p1.pc)?;
        }
        if let Some(r) = dec8(peek(p1.pc)).filter(|_| p1.len == 1) {
            return Some((r, vec![p1.pc, b.pc]));
        }
        let y = or_r(peek(p1.pc)).filter(|_| p1.len == 1)?;
        let p2 = before(p1.pc)?;
        let x = ld_a(peek(p2.pc)).filter(|_| p2.len == 1)?;
        let p3 = before(p2.pc)?;
        let (pair, h, l) = dec16(peek(p3.pc)).filter(|_| p3.len == 1)?;
        ((x, y) == (h, l) || (x, y) == (l, h)).then(|| (pair, vec![p3.pc, p2.pc, p1.pc, b.pc]))
    }

    /// A zero test's carry: its exit (`exit`) is `DEC h; JR NZ` (or `JP NZ`) back to where the
    /// test stays (`stay`), `h` the high half of the pair whose low half the test counts
    /// (`counter`: C, E or L). The pair then counts the passes down as one: each time the low
    /// half reaches 0 the high half loses one, and the loop leaves when that reaches 0. Gives the
    /// pair, the carry's two instructions and where the loop leaves, past them.
    pub(super) fn carry(&self, peek: &dyn Fn(u16) -> u8, exit: u16, stay: u16, counter: u8) -> Option<(u8, [u16; 2], u16)> {
        let (pair, hi) = match counter {
            3 => (8, 2),
            5 => (9, 4),
            7 => (10, 6),
            _ => return None,
        };
        let dec = self.n.get(&exit).filter(|i| i.len == 1 && dec8(peek(i.pc)) == Some(hi))?;
        let jump = self.n.get(&dec.pc.checked_add(1)?).filter(|i| matches!(peek(i.pc), 0x20 | 0xC2) && i.flow == Flow::CondJump(stay))?;
        Some((pair, [dec.pc, jump.pc], jump.pc.checked_add(u16::from(jump.len))?))
    }

    /// The taken side of zero test `b` is the side the counter reaching 0 takes.
    pub(super) fn zero_taken(peek: &dyn Fn(u16) -> u8, b: &Insn) -> bool {
        matches!(peek(b.pc), 0x28 | 0xCA)
    }

    fn find_delays(&mut self, peek: &dyn Fn(u16) -> u8) {
        let mut found = Vec::new();
        for b in self.n.values() {
            let head = match b.flow {
                Flow::Djnz(t) | Flow::CondJump(t) if t <= b.pc && b.pc - t <= DELAY_MAX => t,
                _ => continue,
            };
            let mut chain = Vec::new();
            let mut pc = head;
            let mut ok = true;
            while pc != b.pc {
                match self.straight(peek, pc) {
                    Some(i) if i.pc.wrapping_add(u16::from(i.len)) <= b.pc => {
                        chain.push(i);
                        pc = i.pc.wrapping_add(u16::from(i.len));
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let (counter, idiom_len) = match b.flow {
                Flow::Djnz(_) => (2, 0),
                _ if matches!(peek(b.pc), 0x20 | 0xC2) => {
                    let ops: Vec<u8> = chain.iter().map(|i| peek(i.pc)).collect();
                    let n = ops.len();
                    if let Some(r) = ops.last().and_then(|&o| dec8(o)).filter(|_| chain[n - 1].len == 1) {
                        (r, 1)
                    } else if n >= 3 && chain[n - 3..].iter().all(|i| i.len == 1) {
                        match (dec16(ops[n - 3]), ld_a(ops[n - 2]), or_r(ops[n - 1])) {
                            (Some((pair, h, l)), Some(x), Some(y)) if (x, y) == (h, l) || (x, y) == (l, h) => (pair, 3),
                            _ => continue,
                        }
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            // nothing else in the loop touches the counter
            if chain[..chain.len() - idiom_len].iter().any(|i| i.writes & bits_of(counter) != 0) {
                continue;
            }
            let body: i64 = chain.iter().map(|i| i64::from(i.t)).sum();
            let mut nodes: Vec<u16> = chain.iter().map(|i| i.pc).collect();
            nodes.push(b.pc);
            found.push(Delay {
                head,
                exit: b.pc.wrapping_add(u16::from(b.len)),
                counter,
                per: body + i64::from(b.t_taken),
                konst: i64::from(b.t) - i64::from(b.t_taken),
                nodes,
                outer: None,
            });
        }
        for d in found {
            if self.delays.contains_key(&d.head) || d.nodes.iter().any(|p| self.in_delay.contains_key(p)) {
                continue;
            }
            for &p in &d.nodes {
                self.in_delay.insert(p, d.head);
            }
            self.delays.insert(d.head, d);
        }
        self.find_nested(peek);
    }

    /// Nested delays (see [`Delay`]): a delay loop, then straight code (no `OUT`, no store,
    /// nothing writing either counter but the idiom) ending in another counter's zero idiom whose
    /// branch goes back to the delay's head.
    fn find_nested(&mut self, peek: &dyn Fn(u16) -> u8) {
        let mut found = Vec::new();
        for d in self.delays.values().filter(|d| d.outer.is_none()) {
            let mut body: Vec<&Insn> = Vec::new();
            let mut pc = d.exit;
            let branch = loop {
                if body.len() > 8 || self.in_delay.contains_key(&pc) {
                    break None;
                }
                let Some(i) = self.n.get(&pc) else { break None };
                match i.flow {
                    Flow::Djnz(t) | Flow::CondJump(t) if t == d.head => break Some(i),
                    _ => {}
                }
                match self.straight(peek, pc) {
                    Some(i) => {
                        body.push(i);
                        pc = i.pc.wrapping_add(u16::from(i.len));
                    }
                    None => break None,
                }
            };
            let Some(b) = branch else { continue };
            // the outer counter's idiom: DJNZ, DEC r; JR/JP NZ, DEC rr; LD A,h; OR l; JR/JP NZ
            let ops: Vec<u8> = body.iter().map(|i| peek(i.pc)).collect();
            let n = ops.len();
            let (counter, idiom) = match b.flow {
                Flow::Djnz(_) => (2, 0),
                _ if !matches!(peek(b.pc), 0x20 | 0xC2) => continue,
                _ if n >= 1 && body[n - 1].len == 1 && dec8(ops[n - 1]).is_some() => (dec8(ops[n - 1]).unwrap_or(0), 1),
                _ if n >= 3 && body[n - 3..].iter().all(|i| i.len == 1) => match (dec16(ops[n - 3]), ld_a(ops[n - 2]), or_r(ops[n - 1])) {
                    (Some((pair, h, l)), Some(x), Some(y)) if (x, y) == (h, l) || (x, y) == (l, h) => (pair, 3),
                    _ => continue,
                },
                _ => continue,
            };
            // two separate counters, neither written in the body but by the outer's idiom
            if bits_of(counter) & bits_of(d.counter) != 0 || body[..n - idiom].iter().any(|i| i.writes & (bits_of(counter) | bits_of(d.counter)) != 0) {
                continue;
            }
            let wraps: i64 = if d.counter >= 8 { 65536 } else { 256 };
            let straight: i64 = body.iter().map(|i| i64::from(i.t)).sum();
            let (taken, fall) = (i64::from(b.t_taken), i64::from(b.t));
            let mut nodes = d.nodes.clone();
            nodes.extend(body.iter().map(|i| i.pc));
            nodes.push(b.pc);
            found.push(Delay {
                exit: b.pc.wrapping_add(u16::from(b.len)),
                nodes,
                outer: Some((counter, wraps * d.per + d.konst + straight + taken, fall - taken - wraps * d.per - d.konst)),
                ..d.clone()
            });
        }
        for d in found {
            for &p in &d.nodes {
                self.in_delay.insert(p, d.head);
            }
            self.delays.insert(d.head, d);
        }
    }
}

/// One step of a pass.
#[derive(Clone, Debug)]
pub(super) enum Item {
    /// An instruction run straight through (a `JR`/`JP` included).
    Op(u16),
    /// A delay loop, by its head.
    Delay(u16),
    /// A zero test whose zero side leaves the pass (to the next note's setup): the pass goes on
    /// by the other side.
    Test { pc: u16, stay_taken: bool, counter: u8, exit: u16 },
    /// A branch on a keyboard read whose side an idle keyboard takes ([`idle_keyboard`]): the
    /// pass goes that way, `taken` or not.
    Fixed { pc: u16, taken: bool },
    /// A zero test whose two sides meet again straight away, one of them with an `OUT`.
    Diamond { pc: u16, zero_taken: bool, taken: Vec<u16>, fall: Vec<u16>, merge: u16, counter: u8 },
}

impl Item {
    /// The instruction the item starts at.
    pub(super) fn pc(&self) -> u16 {
        match self {
            Item::Op(pc) | Item::Delay(pc) | Item::Test { pc, .. } | Item::Diamond { pc, .. } | Item::Fixed { pc, .. } => *pc,
        }
    }
}

/// The arm of a branch from `start`: straight instructions (`OUT`s and jumps included) up to
/// where it stops, the stop included last.
fn arm(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, start: u16) -> Vec<u16> {
    let mut v = Vec::new();
    let mut pc = start;
    for _ in 0..ARM_MAX {
        v.push(pc);
        if cfg.in_delay.contains_key(&pc) {
            break;
        }
        let Some(i) = cfg.n.get(&pc) else { break };
        if undecoded(peek, i) || i.t != i.t_taken {
            break;
        }
        match i.flow {
            Flow::Next => match i.pc.checked_add(u16::from(i.len)) {
                Some(n) => pc = n,
                None => break,
            },
            Flow::Jump(t) => pc = t,
            _ => break,
        }
    }
    v
}

/// A diamond at zero test `b`: both sides meet within [`ARM_MAX`] straight instructions and
/// one of them holds an `OUT`.
fn diamond(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, b: &Insn, target: u16) -> Option<Item> {
    let next = b.pc.checked_add(u16::from(b.len))?;
    let (ta, fa) = (arm(cfg, peek, target), arm(cfg, peek, next));
    let (ia, merge) = ta.iter().enumerate().find(|(_, p)| fa.contains(p)).map(|(i, &p)| (i, p))?;
    let jf = fa.iter().position(|&p| p == merge)?;
    let (taken, fall) = (ta[..ia].to_vec(), fa[..jf].to_vec());
    let has_out = |arm: &[u16]| arm.iter().any(|p| cfg.n.get(p).is_some_and(|i| i.port_out.is_some()));
    if !has_out(&taken) && !has_out(&fall) {
        return None;
    }
    let (counter, _) = cfg.zero_idiom(peek, b)?;
    Some(Item::Diamond { pc: b.pc, zero_taken: Cfg::zero_taken(peek, b), taken, fall, merge, counter })
}

/// The pass found by [`walk`].
pub(super) struct Walk {
    pub items: Vec<Item>,
    /// As [`Cfg::inlined`] and [`Cfg::callret`].
    pub inlined: BTreeSet<u16>,
    pub callret: BTreeSet<u16>,
}

/// The pass of the loop through `out`: from `out` forward, through delay loops, both arms of
/// diamonds, the staying side of zero tests, the side an idle keyboard takes, and into the
/// routines the pass calls (up to three deep) and back from their returns, until the walk comes
/// back to where it has been with the same calls open. The pass is the cycle from there.
pub(super) fn walk(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, out: u16) -> Result<Walk, Refusal> {
    let mut items: Vec<Item> = Vec::new();
    // per item, whether it lies inside a call
    let mut called: Vec<bool> = Vec::new();
    let mut at: BTreeMap<(u16, Vec<u16>), usize> = BTreeMap::new();
    // the return addresses of the calls open
    let mut calls: Vec<u16> = Vec::new();
    let mut callret = BTreeSet::new();
    let mut pc = out;
    loop {
        if let Some(&k) = at.get(&(pc, calls.clone())) {
            let items = items.split_off(k);
            let mut inlined = BTreeSet::new();
            for (it, _) in items.iter().zip(&called[k..]).filter(|(_, c)| **c) {
                match it {
                    Item::Delay(h) => inlined.extend(&cfg.delays[h].nodes),
                    it => {
                        inlined.insert(it.pc());
                    }
                }
            }
            callret.retain(|p| items.iter().any(|it| it.pc() == *p));
            return Ok(Walk { items, inlined, callret });
        }
        if items.len() >= PASS_MAX {
            return Err(Refusal::Unmodelled(Why::PassTooLong, out));
        }
        at.insert((pc, calls.clone()), items.len());
        called.push(!calls.is_empty());
        if let Some(d) = cfg.delays.get(&pc) {
            items.push(Item::Delay(pc));
            pc = d.exit;
            continue;
        }
        if cfg.in_delay.contains_key(&pc) {
            return Err(Refusal::Unmodelled(Why::IntoDelay, pc)); // into a delay loop's middle
        }
        let Some(i) = cfg.n.get(&pc) else { return Err(Refusal::Unmodelled(Why::LeavesWindow, pc)) };
        if undecoded(peek, i) {
            return Err(Refusal::Undecoded(pc));
        }
        let next = || i.pc.checked_add(u16::from(i.len)).ok_or(Refusal::Unmodelled(Why::LeavesWindow, pc));
        match i.flow {
            Flow::Next if i.t == i.t_taken => {
                items.push(Item::Op(pc));
                pc = next()?;
            }
            Flow::Jump(t) => {
                items.push(Item::Op(pc));
                pc = t;
            }
            Flow::CondJump(t) | Flow::Djnz(t) => {
                if let Some(d) = diamond(cfg, peek, i, t) {
                    if let Item::Diamond { merge, .. } = &d {
                        pc = *merge;
                    }
                    items.push(d);
                } else if let Some((counter, _)) = cfg.zero_idiom(peek, i) {
                    let zero_taken = Cfg::zero_taken(peek, i);
                    let (stay, exit) = if zero_taken { (next()?, t) } else { (t, next()?) };
                    items.push(Item::Test { pc, stay_taken: !zero_taken, counter, exit });
                    pc = stay;
                } else if let Some(taken) = idle_keyboard(cfg, peek, i) {
                    items.push(Item::Fixed { pc, taken });
                    pc = if taken { t } else { next()? };
                } else {
                    return Err(Refusal::Unmodelled(branch_kind(peek(pc)), pc));
                }
            }
            Flow::JumpIndirect => return Err(Refusal::ComputedJump(pc)),
            Flow::Call(t) if calls.len() < CALLS_MAX && cfg.n.contains_key(&t) => {
                items.push(Item::Op(pc));
                callret.insert(pc);
                calls.push(next()?);
                pc = t;
            }
            Flow::Ret if !calls.is_empty() => {
                items.push(Item::Op(pc));
                callret.insert(pc);
                pc = calls.pop().unwrap_or(pc);
            }
            Flow::Call(_) | Flow::CondCall(_) | Flow::Rst(_) => return Err(Refusal::Unmodelled(Why::CallInPass, pc)),
            Flow::Ret | Flow::CondRet => return Err(Refusal::Unmodelled(Why::ReturnInPass, pc)),
            Flow::Halt => return Err(Refusal::Unmodelled(Why::HaltInPass, pc)),
            Flow::Next => return Err(Refusal::Unmodelled(Why::BlockRepeat, pc)),
        }
    }
}

/// Which flag a conditional jump (opcode `op`) that is not a counter's zero test reads.
fn branch_kind(op: u8) -> Why {
    match op {
        0x30 | 0x38 | 0xD2 | 0xDA => Why::BranchCarry,
        0xE2 | 0xEA | 0xF2 | 0xFA => Why::BranchSign,
        _ => Why::BranchIdiom,
    }
}

/// A setup path: from a zero test's exit back into the pass (at `reentry`).
#[derive(Clone, Debug)]
pub(super) struct SetupPath {
    pub nodes: Vec<u16>,
    pub reentry: u16,
}

/// Most setup paths, their length, and visits of one instruction in a path.
const PATHS_MAX: usize = 64;
const PATH_LEN: usize = 256;
const VISITS: u8 = 2;
const STEPS_MAX: u32 = 20_000;

/// The paths from `from` back into the pass (`main`: the pass's own instructions, where a path
/// ends) that avoid the rest of the pass's code (`inside`: the arms and delay loops). Calls are
/// stepped over; paths that return, jump through a register or leave the window are dropped.
/// None when there are more paths, or longer ones, than the reader follows (then some path
/// back into the pass would go unchecked).
pub(super) fn setup_paths(cfg: &Cfg, from: u16, main: &BTreeSet<u16>, inside: &BTreeSet<u16>) -> Option<Vec<SetupPath>> {
    struct Dfs<'a> {
        cfg: &'a Cfg,
        main: &'a BTreeSet<u16>,
        inside: &'a BTreeSet<u16>,
        path: Vec<u16>,
        visits: BTreeMap<u16, u8>,
        out: Vec<SetupPath>,
        steps: u32,
        cut: bool,
    }
    impl Dfs<'_> {
        fn go(&mut self, pc: u16) {
            self.steps += 1;
            if self.steps > STEPS_MAX || self.out.len() > PATHS_MAX {
                self.cut = true;
                return;
            }
            if self.main.contains(&pc) {
                self.out.push(SetupPath { nodes: self.path.clone(), reentry: pc });
                return;
            }
            if self.inside.contains(&pc) {
                return;
            }
            if self.path.len() >= PATH_LEN {
                self.cut = true;
                return;
            }
            let Some(i) = self.cfg.n.get(&pc) else { return };
            let v = self.visits.entry(pc).or_default();
            if *v >= VISITS {
                return;
            }
            *v += 1;
            self.path.push(pc);
            for s in succs(i) {
                self.go(s);
            }
            self.path.pop();
            if let Some(v) = self.visits.get_mut(&pc) {
                *v -= 1;
            }
        }
    }
    let mut d = Dfs { cfg, main, inside, path: Vec::new(), visits: BTreeMap::new(), out: Vec::new(), steps: 0, cut: false };
    d.go(from);
    (!d.cut && d.out.len() <= PATHS_MAX).then_some(d.out)
}

/// A `JR`/`JP` `Z`/`NZ` whose flag comes from a keyboard read (`IN A,(n)`, n even) through
/// `OR n`, `AND n`, `XOR n`, `CPL`, `INC A`, `DEC A`, `CP n`, `OR A`, `AND A` only, run straight
/// into it: the side an idle keyboard takes, with the key bits (0-4) all 1 and the others not
/// known (a side that depends on them is not known). The reader assumes no key is down while
/// the music plays: a key that ends a note early is caught on the device, whose strict check of
/// a learned entry (each rise one period of the formula after the last) leaves a frame that
/// fails it to the beeper.
fn idle_keyboard(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, b: &Insn) -> Option<bool> {
    let op = peek(b.pc);
    if !matches!(op, 0x20 | 0x28 | 0xC2 | 0xCA) {
        return None;
    }
    // the unique instruction before `pc`, running into it
    let before = |pc: u16| -> Option<&Insn> {
        match cfg.preds.get(&pc).map(Vec::as_slice) {
            Some([p]) => cfg.n.get(p).filter(|i| i.flow == Flow::Next && i.t == i.t_taken && i.pc.wrapping_add(u16::from(i.len)) == pc),
            _ => None,
        }
    };
    let mut chain = Vec::new();
    let mut pc = b.pc;
    loop {
        let i = before(pc)?;
        if peek(i.pc) == 0xDB && peek(i.pc.wrapping_add(1)) & 1 == 0 {
            break;
        }
        if chain.len() >= 6 {
            return None;
        }
        chain.push(i.pc);
        pc = i.pc;
    }
    chain.reverse();
    // A's value and the bits of it known; Z when known
    let (mut a, mut known): (u8, u8) = (0x1F, 0x1F);
    let mut z: Option<bool> = None;
    let zero = |a: u8, known: u8| {
        if a & known != 0 {
            Some(false)
        } else {
            (known == 0xFF).then_some(true)
        }
    };
    for &p in &chain {
        let n = peek(p.wrapping_add(1));
        match peek(p) {
            0xF6 => {
                (a, known) = (a | n, known | n);
                z = zero(a, known);
            }
            0xE6 => {
                (a, known) = (a & n, known | !n);
                z = zero(a, known);
            }
            0xEE => {
                a ^= n;
                z = zero(a, known);
            }
            0x2F => a = !a,
            0x3C | 0x3D if known == 0xFF => {
                a = if peek(p) == 0x3C { a.wrapping_add(1) } else { a.wrapping_sub(1) };
                z = Some(a == 0);
            }
            0xFE => z = zero(a ^ n, known),
            0xB7 | 0xA7 => z = zero(a, known),
            _ => return None,
        }
    }
    let z = z?;
    Some(if matches!(op, 0x28 | 0xCA) { z } else { !z })
}

/// An instruction that leaves the flags as they are, runs straight on and keeps the registers
/// where they are (no exchange): loads, `INC`/`DEC` of a pair, `PUSH`, `POP` of a pair but AF,
/// `OUT`.
pub(super) fn flag_neutral(peek: &dyn Fn(u16) -> u8, i: &Insn) -> bool {
    if i.flow != Flow::Next || i.t != i.t_taken {
        return false;
    }
    let b0 = peek(i.pc);
    if b0 == 0xED {
        // OUT (C),r; LD (nn),rr; LD rr,(nn)
        let o = peek(i.pc.wrapping_add(1));
        return o & 0xC7 == 0x41 || o & 0xCF == 0x43 || o & 0xCF == 0x4B;
    }
    let op = if matches!(b0, 0xDD | 0xFD) && i.len > 1 { peek(i.pc.wrapping_add(1)) } else { b0 };
    matches!(op, 0x00 | 0x01 | 0x11 | 0x21 | 0x31 | 0x02 | 0x12 | 0x0A | 0x1A | 0x22 | 0x2A | 0x32 | 0x3A | 0x03 | 0x13 | 0x23 | 0x33 | 0x0B | 0x1B | 0x2B | 0x3B | 0xD3 | 0xC5 | 0xD5 | 0xE5 | 0xF5 | 0xC1 | 0xD1 | 0xE1)
        || op & 0xC7 == 0x06
        || ((0x40..=0x7F).contains(&op) && op != 0x76)
}
