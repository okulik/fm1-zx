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

pub(super) struct Cfg {
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

/// The instructions reachable from `out` within the window.
pub(super) fn build(peek: &dyn Fn(u16) -> u8, out: u16) -> Cfg {
    let lo = (i32::from(out) - WINDOW).max(0);
    let hi = (i32::from(out) + WINDOW).min(0x10000);
    let mut n = BTreeMap::new();
    let mut todo = vec![out];
    while let Some(pc) = todo.pop() {
        if n.contains_key(&pc) || i32::from(pc) < lo {
            continue;
        }
        let i = decode(peek, pc);
        if i32::from(pc) + i32::from(i.len) > hi {
            continue;
        }
        todo.extend(succs(&i));
        n.insert(pc, i);
    }
    let mut preds: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for (pc, i) in &n {
        for s in succs(i) {
            preds.entry(s).or_default().push(*pc);
        }
    }
    let mut cfg = Cfg { n, preds, delays: BTreeMap::new(), in_delay: BTreeMap::new() };
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
#[derive(Clone, Debug)]
pub(super) struct Delay {
    pub head: u16,
    pub exit: u16,
    /// Register number (as [`crate::prog`]): 0, 2-7 or 8-10.
    pub counter: u8,
    pub per: i64,
    pub konst: i64,
    pub nodes: Vec<u16>,
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
    /// Gives the counter's register number and the idiom's instructions.
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
        let p1 = before(b.pc)?;
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
    /// A zero test whose two sides meet again straight away, one of them with an `OUT`.
    Diamond { pc: u16, zero_taken: bool, taken: Vec<u16>, fall: Vec<u16>, merge: u16, counter: u8 },
}

impl Item {
    /// The instruction the item starts at.
    pub(super) fn pc(&self) -> u16 {
        match self {
            Item::Op(pc) | Item::Delay(pc) | Item::Test { pc, .. } | Item::Diamond { pc, .. } => *pc,
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

/// The pass of the loop through `out`: from `out` forward, through delay loops, both arms of
/// diamonds and the staying side of zero tests, until the walk comes back to where it has been.
/// The pass is the cycle from there.
pub(super) fn walk(cfg: &Cfg, peek: &dyn Fn(u16) -> u8, out: u16) -> Result<Vec<Item>, Refusal> {
    let mut items: Vec<Item> = Vec::new();
    let mut at: BTreeMap<u16, usize> = BTreeMap::new();
    let mut pc = out;
    loop {
        if let Some(&k) = at.get(&pc) {
            return Ok(items.split_off(k));
        }
        if items.len() >= PASS_MAX {
            return Err(Refusal::Unmodelled(Why::PassTooLong, out));
        }
        at.insert(pc, items.len());
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
        match i.flow {
            Flow::Next if i.t == i.t_taken => {
                items.push(Item::Op(pc));
                pc = i.pc.checked_add(u16::from(i.len)).ok_or(Refusal::Unmodelled(Why::LeavesWindow, pc))?;
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
                    let next = i.pc.checked_add(u16::from(i.len)).ok_or(Refusal::Unmodelled(Why::LeavesWindow, pc))?;
                    let (stay, exit) = if zero_taken { (next, t) } else { (t, next) };
                    items.push(Item::Test { pc, stay_taken: !zero_taken, counter, exit });
                    pc = stay;
                } else {
                    return Err(Refusal::Unmodelled(branch_kind(peek(pc)), pc));
                }
            }
            Flow::JumpIndirect => return Err(Refusal::ComputedJump(pc)),
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
