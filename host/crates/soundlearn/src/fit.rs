// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A routine's model against what was heard ([`check_fit`]), and the model as a learned entry
//! ([`to_entry`]): the signature's digest, the sites, and the programs the generic reader runs.
//!
//! # The fit
//!
//! Each period sample of the heard site (its flips `k-2` to `k`, contention taken out) is set
//! against the model's channel evaluated on the sample's snapshot: the registers at the site's
//! `OUT`, the memory the values read then (and, for an address not read in that period, the value
//! it was read with last). A sample spans two of the site's flips, and a channel with `s` sites
//! flips at each of them in turn, so a sample is `2 s` half periods. The samples are grouped by
//! their predicted period; a sample is an inlier within 0.6% + 160/n T of its prediction, n its
//! group's size (the spike's tolerance). A group with two inliers or more is a note.
//!
//! A Slots routine with more than one channel cannot be fitted this way: its sites set the
//! speaker to each channel's level in turn, so a site's flips follow the other channels too.
//!
//! # The programs
//!
//! A channel's program is its full period, T-states x 256: each term of the half period mapped
//! to ops (`Lin`: count(x) x coef_t x 512, `Prod`: count(x) x count(y) x coef_t x 512, `Recip`:
//! num_t x 512 / x, `Const`: c x 512; 512 = 256 for the unit, 2 for the full period), summed.
//! A count is a counter's reload, 0 counting 256 (65,536 for a 16-bit input):
//! `((x - 1) & 255) + 1`. `silent_if_zero` comes first, as its input and `ZeroSilent`. The timing
//! program is `((start - counter) & mask) x per_unit_t` (see `DurationModel`).

use crate::entry::LearnedEntry;
use crate::listen::SiteHeard;
use crate::model::{eval_terms, ChannelModel, DurationModel, EngineModel, Family, Input, Term};
use crate::prog::{encode_prog, Op};
use std::cell::Cell;
use std::collections::HashMap;
use zx_sys::{SND_PROG_MAX, SND_SITE_TONAL};

const CLOCK: f64 = 3_500_000.0;
/// A learned channel's loudness.
const LEVEL: u8 = 200;
/// T-states x 256, and two half periods to the period.
const PERIOD_SCALE: i64 = 512;

/// How a model's channel agrees with a heard site's period samples.
#[derive(Clone, Debug, PartialEq)]
pub struct FitResult {
    /// The share of the samples (those the model could be evaluated for) within tolerance.
    pub inliers: f64,
    /// Predicted periods with two inliers or more.
    pub distinct_notes: u32,
    /// From the lowest of those notes to the highest.
    pub span_semitones: f64,
    /// The highest note's frequency.
    pub max_hz: f64,
}

/// Fits the model's channel holding the site `heard.pc` to the site's period samples (see the
/// module docs); Err when the site is not the model's, the family cannot be fitted from a
/// site's flips, or no sample could be evaluated.
pub fn check_fit(model: &EngineModel, heard: &SiteHeard) -> Result<FitResult, String> {
    let ch = model
        .channels
        .iter()
        .find(|c| c.sites.contains(&heard.pc))
        .ok_or_else(|| format!("site {:04X}h is not the routine's", heard.pc))?;
    if model.family == Family::Slots && model.channels.len() > 1 {
        return Err("a site of a routine with time slots does not show its own channel".into());
    }
    let per_sample = 2.0 * ch.sites.len() as f64;
    let mut mem: HashMap<u16, u8> = HashMap::new();
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for p in &heard.periods {
        let Some(snap) = heard.snapshots.get(p.snap as usize) else { continue };
        for &(_, _, addr, value) in &snap.reads {
            mem.insert(addr, value);
        }
        let missing = Cell::new(false);
        let peek = |a: u16| {
            mem.get(&a).copied().unwrap_or_else(|| {
                missing.set(true);
                0
            })
        };
        let silent = ch.silent_if_zero.is_some_and(|x| x.read(&peek, model.anchor, &snap.regs) == Some(0));
        let half = eval_terms(&ch.half_period, &peek, model.anchor, &snap.regs);
        match half {
            Some(h) if !missing.get() && !silent && h > 0.0 => pairs.push((p.period_t, per_sample * h)),
            _ => {}
        }
    }
    if pairs.is_empty() {
        return Err(format!("no period sample of {:04X}h could be evaluated", heard.pc));
    }
    // per predicted period (to half a T-state): samples, inliers
    let key = |pr: f64| (pr * 2.0).round() as i64;
    let mut groups: HashMap<i64, (u32, u32, f64)> = HashMap::new();
    for &(_, pr) in &pairs {
        groups.entry(key(pr)).or_insert((0, 0, pr)).0 += 1;
    }
    let mut inliers = 0usize;
    for &(got, pr) in &pairs {
        let g = groups.get_mut(&key(pr)).expect("grouped");
        if (got - pr).abs() <= 0.006 * pr + 160.0 / f64::from(g.0) {
            g.1 += 1;
            inliers += 1;
        }
    }
    let notes: Vec<f64> = groups.values().filter(|g| g.1 >= 2).map(|g| g.2 / ch.sites.len() as f64).collect();
    let (lo, hi) = notes.iter().fold((f64::MAX, 0.0f64), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    Ok(FitResult {
        inliers: inliers as f64 / pairs.len() as f64,
        distinct_notes: notes.len() as u32,
        span_semitones: if notes.is_empty() { 0.0 } else { 12.0 * (hi / lo).log2() },
        max_hz: if notes.is_empty() { 0.0 } else { CLOCK / lo },
    })
}

/// The learned entry of `model`, with `peek` the memory the routine sits in: the digest over
/// `[anchor, code_end)` with the wild bytes and relocated words as the model gives them, each
/// channel's sites (tonal), its period program and the timing program from the duration (none
/// if the model has no duration, or its program would not fit or could overflow).
pub fn to_entry(model: &EngineModel, peek: impl Fn(u16) -> u8) -> LearnedEntry {
    let len = model.code_end.wrapping_sub(model.anchor);
    let mut sites: Vec<(u16, u8, u8)> = model
        .channels
        .iter()
        .enumerate()
        .flat_map(|(c, ch)| ch.sites.iter().map(move |&pc| (pc.wrapping_sub(model.anchor), SND_SITE_TONAL, c as u8)))
        .collect();
    sites.sort_unstable();
    LearnedEntry {
        kind: 0,
        digest: digest(&peek, model.anchor, len, &model.wild, &model.reloc),
        len,
        wild: model.wild.clone(),
        reloc: model.reloc.clone(),
        sites,
        pass: model.pass_t,
        pos: model.pos.clone(),
        flips: model.flips,
        drum: 0,
        stale: model.stale,
        level: vec![LEVEL; model.channels.len()],
        progs: model.channels.iter().map(period_prog).collect(),
        timing: model.duration.as_ref().and_then(timing_prog),
    }
}

/// `snd_engine_digest`: FNV-1a 64 of the `len` bytes from `anchor`, the bytes at the `wild`
/// offsets left out and the words at the `reloc` offsets hashed as their offset from the anchor.
pub fn digest(peek: &dyn Fn(u16) -> u8, anchor: u16, len: u16, wild: &[u16], reloc: &[u16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for i in 0..len {
        if wild.contains(&i) {
            continue;
        }
        let byte = match reloc.iter().find(|&&r| r == i || r.wrapping_add(1) == i) {
            Some(&lo) => {
                let at = anchor.wrapping_add(lo);
                let word = u16::from_le_bytes([peek(at), peek(at.wrapping_add(1))]).wrapping_sub(anchor);
                if i == lo { word as u8 } else { (word >> 8) as u8 }
            }
            None => peek(anchor.wrapping_add(i)),
        };
        h = (h ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The op that pushes `x`.
fn input_op(x: &Input) -> Op {
    match *x {
        Input::Rel(o) => Op::PeekRel(o),
        Input::Rel16(o) => Op::Peek16Rel(o),
        Input::Abs(a) => Op::Peek(a),
        Input::Abs16(a) => Op::Peek16(a),
        Input::Indexed(r, d) => Op::PeekReg(r, d),
        Input::Reg(r) => Op::Reg(r),
    }
}

/// Pushes `x` as a count: 0 is 256 (65,536 for a 16-bit input).
fn push_count(ops: &mut Vec<Op>, x: &Input) {
    let mask = if x.wide() { 0xFFFF } else { 0xFF };
    ops.extend([input_op(x), Op::K(1), Op::Sub, Op::K(mask), Op::And, Op::K(1), Op::Add]);
}

/// Pushes `k` (two's complement for a negative one: the arithmetic wraps).
fn push_k(ops: &mut Vec<Op>, k: i64) {
    ops.push(Op::K(k as u32));
}

/// Pushes the sum of `terms` times `scale`.
fn push_sum(ops: &mut Vec<Op>, terms: &[Term], scale: i64) {
    let mut konst = 0i64;
    let mut first = true;
    for t in terms {
        match t {
            Term::Const(c) => {
                konst += c * scale;
                continue;
            }
            Term::Lin { coef_t, x } => {
                push_count(ops, x);
                push_k(ops, coef_t * scale);
                ops.push(Op::Mul);
            }
            Term::Prod { coef_t, x, y } => {
                push_count(ops, x);
                push_count(ops, y);
                ops.push(Op::Mul);
                push_k(ops, coef_t * scale);
                ops.push(Op::Mul);
            }
            Term::Recip { num_t, x } => {
                // num x scale / x, the numerator halved until it fits (and the quotient doubled)
                let mut num = num_t * scale;
                let mut shift = 0u8;
                while num > i64::from(u32::MAX) && shift < 31 {
                    num >>= 1;
                    shift += 1;
                }
                push_k(ops, num);
                ops.extend([input_op(x), Op::Div]);
                if shift > 0 {
                    ops.push(Op::Shl(shift));
                }
            }
        }
        if !first {
            ops.push(Op::Add);
        }
        first = false;
    }
    if first {
        push_k(ops, konst);
    } else if konst > 0 {
        push_k(ops, konst);
        ops.push(Op::Add);
    } else if konst < 0 {
        push_k(ops, -konst);
        ops.push(Op::Sub);
    }
}

/// A channel's period program: T-states x 256 of its full period.
pub(crate) fn period_prog(ch: &ChannelModel) -> Vec<Op> {
    let mut ops = Vec::new();
    if let Some(x) = &ch.silent_if_zero {
        ops.extend([input_op(x), Op::ZeroSilent]);
    }
    push_sum(&mut ops, &ch.half_period, PERIOD_SCALE);
    ops
}

/// The largest value the sum of `terms` x `scale` can take, its counts at their largest (a
/// `Recip` at its largest, 32 bits: its program fits the scaled numerator to them).
fn sum_max(terms: &[Term], scale: i64) -> f64 {
    let count = |x: &Input| if x.wide() { 65536.0 } else { 256.0 };
    terms
        .iter()
        .map(|t| match t {
            Term::Const(c) => (c * scale) as f64,
            Term::Lin { coef_t, x } => (coef_t * scale) as f64 * count(x),
            Term::Prod { coef_t, x, y } => (coef_t * scale) as f64 * count(x) * count(y),
            Term::Recip { .. } => f64::from(u32::MAX),
        })
        .filter(|&v| v > 0.0)
        .sum()
}

/// The channel's period program fits a program and its value never wraps.
pub(crate) fn period_ok(ch: &ChannelModel) -> bool {
    encode_prog(&period_prog(ch)).len() <= SND_PROG_MAX && sum_max(&ch.half_period, PERIOD_SCALE) <= f64::from(u32::MAX)
}

/// The timing program, `((start - counter) & mask) x per_unit_t`: None without a start, or if
/// it would not fit a program or could overflow.
pub(crate) fn timing_prog(d: &DurationModel) -> Option<Vec<Op>> {
    let wide = d.counter.wide();
    let mask: u32 = if wide { 0xFFFF } else { 0xFF };
    let mut ops = Vec::new();
    match d.counts_down_from? {
        Input::Indexed(r, dd) if wide => ops.extend([Op::PeekReg(r, dd), Op::PeekReg(r, dd.checked_add(1)?), Op::Shl(8), Op::Add]),
        x => ops.push(input_op(&x)),
    }
    ops.extend([input_op(&d.counter), Op::Sub, Op::K(mask), Op::And]);
    push_sum(&mut ops, &d.per_unit_t, 1);
    ops.push(Op::Mul);
    (encode_prog(&ops).len() <= SND_PROG_MAX && f64::from(mask) * sum_max(&d.per_unit_t, 1) <= f64::from(u32::MAX)).then_some(ops)
}
