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
//! group's size. A group of at least `MIN_GROUP` samples whose trimmed
//! mean is within that of its prediction is a note; one whose trimmed mean is not is a wrong
//! note ([`FitResult::wrong`]), which the gates refuse however few samples it holds against the
//! rest: the entry would play that note wrong.
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
/// Fewest samples of one predicted period that can agree with it.
const MIN_GROUP: usize = 8;
/// Notes closer than this (relative) are one.
const NOTE_TOL: f64 = 0.006;

/// How a model's channel agrees with a heard site's period samples.
#[derive(Clone, Debug, PartialEq)]
pub struct FitResult {
    /// The share of the samples (those the model could be evaluated for) within tolerance.
    pub inliers: f64,
    /// Agreeing predicted periods, those within 0.6% of each other counted once.
    pub distinct_notes: u32,
    /// From the lowest of those notes to the highest.
    pub span_semitones: f64,
    /// The highest note's frequency.
    pub max_hz: f64,
    /// The largest group of samples (at least `MIN_GROUP`) whose prediction they disagree with:
    /// a note the model gets wrong.
    pub wrong: Option<WrongNote>,
}

/// A predicted period its samples disagree with.
#[derive(Clone, Debug, PartialEq)]
pub struct WrongNote {
    /// The predicted period (T-states, the sample's span: `2 x sites` half periods).
    pub predicted_t: f64,
    /// The samples' trimmed mean.
    pub heard_t: f64,
    pub samples: u32,
}

/// Fits the model's channel holding the site `heard.pc` to the site's period samples (see the
/// module docs); Err when the site is not the model's, the family cannot be fitted from a
/// site's flips, or no sample could be evaluated.
pub fn check_fit(model: &EngineModel, heard: &SiteHeard) -> Result<FitResult, String> {
    fit_periods(model, heard, false)
}

/// [`check_fit`] against the periods as played, contention left in: what a routine whose times
/// depend on where its stack lies sounded.
pub fn check_fit_played(model: &EngineModel, heard: &SiteHeard) -> Result<FitResult, String> {
    fit_periods(model, heard, true)
}

fn fit_periods(model: &EngineModel, heard: &SiteHeard, played: bool) -> Result<FitResult, String> {
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
        let silent = ch.silent_if_zero.is_some_and(|x| x.read(&peek, model.anchor, &snap.regs) == Some(0))
            || ch.silent_if_bit.is_some_and(|(x, m, set)| x.read(&peek, model.anchor, &snap.regs).is_some_and(|v| (v & u32::from(m) != 0) == set));
        let half = eval_terms(&ch.half_period, &peek, model.anchor, &snap.regs);
        match half {
            Some(h) if !missing.get() && !silent && h > 0.0 => pairs.push((p.period_t + if played { p.contention_t } else { 0.0 }, per_sample * h)),
            _ => {}
        }
    }
    if pairs.is_empty() {
        return Err(format!("no period sample of {:04X}h could be evaluated", heard.pc));
    }
    // per predicted period (to half a T-state): the measured periods
    let key = |pr: f64| (pr * 2.0).round() as i64;
    let mut groups: HashMap<i64, (f64, Vec<f64>)> = HashMap::new();
    for &(got, pr) in &pairs {
        groups.entry(key(pr)).or_insert((pr, Vec::new())).1.push(got);
    }
    // a group agrees: at least MIN_GROUP samples, their trimmed mean within
    // 0.6% + 160/n T of the prediction; its inliers are its samples within that too. A group of
    // MIN_GROUP samples or more whose trimmed mean is not within it is a wrong note.
    let mut inliers = 0usize;
    let mut notes: Vec<f64> = Vec::new();
    let mut wrong: Option<WrongNote> = None;
    for (pr, got) in groups.values() {
        let n = got.len();
        let tol = 0.006 * pr + 160.0 / n as f64;
        if n < MIN_GROUP {
            continue;
        }
        let mean = trimmed_mean(got);
        if (mean - pr).abs() > tol {
            if wrong.as_ref().is_none_or(|w| (n as u32, -*pr) > (w.samples, -w.predicted_t)) {
                wrong = Some(WrongNote { predicted_t: *pr, heard_t: mean, samples: n as u32 });
            }
            continue;
        }
        inliers += got.iter().filter(|&&g| (g - pr).abs() <= tol).count();
        notes.push(pr / ch.sites.len() as f64);
    }
    // notes within the tolerance of each other are one
    notes.sort_by(f64::total_cmp);
    let mut distinct: Vec<f64> = Vec::new();
    for p in notes {
        if distinct.last().is_none_or(|&d| p > d * (1.0 + NOTE_TOL)) {
            distinct.push(p);
        }
    }
    let (lo, hi) = (distinct.first().copied().unwrap_or(0.0), distinct.last().copied().unwrap_or(0.0));
    Ok(FitResult {
        inliers: inliers as f64 / pairs.len() as f64,
        distinct_notes: distinct.len() as u32,
        span_semitones: if distinct.is_empty() { 0.0 } else { 12.0 * (hi / lo).log2() },
        max_hz: if distinct.is_empty() { 0.0 } else { CLOCK / lo },
        wrong,
    })
}

/// The mean of the values within 6% of their median.
fn trimmed_mean(v: &[f64]) -> f64 {
    let mut w = v.to_vec();
    w.sort_by(f64::total_cmp);
    let med = w[w.len() / 2];
    let near: Vec<f64> = w.into_iter().filter(|x| (x - med).abs() <= 0.06 * med).collect();
    near.iter().sum::<f64>() / near.len() as f64
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
        digest: digest(&peek, model.anchor, len, &model.wild, &model.reloc),
        len,
        wild: model.wild.clone(),
        reloc: model.reloc.clone(),
        sites,
        pass: model.pass_t,
        pos: model.pos.clone(),
        flips: model.flips,
        observed: false,
        drum: 0,
        stale: model.stale,
        level: vec![LEVEL; model.channels.len()],
        instrument: [0; 3],
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

/// Pushes `x` as a count: 0 is 256 (65,536 for a 16-bit input).
fn push_count(ops: &mut Vec<Op>, x: &Input) {
    let mask = if x.wide() { 0xFFFF } else { 0xFF };
    ops.extend(x.ops());
    ops.extend([Op::K(1), Op::Sub, Op::K(mask), Op::And, Op::K(1), Op::Add]);
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
                ops.extend(x.ops());
                ops.push(Op::Div);
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
        ops.extend(x.ops());
        ops.push(Op::ZeroSilent);
    }
    if let Some((x, m, set)) = &ch.silent_if_bit {
        // silent when set: m - (x & m) is 0 then; silent when clear: x & m is 0 then
        if *set {
            ops.push(Op::K(u32::from(*m)));
        }
        ops.extend(x.ops());
        ops.extend([Op::K(u32::from(*m)), Op::And]);
        if *set {
            ops.push(Op::Sub);
        }
        ops.push(Op::ZeroSilent);
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

/// The smallest value the sum of `terms` x `scale` can take: a count at 1 for a term that grows
/// with it, at its largest for one that falls; a `Recip` at its largest step (0 is silence), as
/// its program divides (a negative numerator: no bound).
fn sum_min(terms: &[Term], scale: i64) -> f64 {
    let count = |x: &Input| if x.wide() { 65536.0 } else { 256.0 };
    terms
        .iter()
        .map(|t| match t {
            Term::Const(c) => (c * scale) as f64,
            Term::Lin { coef_t, x } => (coef_t * scale) as f64 * if *coef_t >= 0 { 1.0 } else { count(x) },
            Term::Prod { coef_t, x, y } => (coef_t * scale) as f64 * if *coef_t >= 0 { 1.0 } else { count(x) * count(y) },
            Term::Recip { num_t, x } if *num_t >= 0 => {
                let mut num = num_t * scale;
                let mut shift = 0u8;
                while num > i64::from(u32::MAX) && shift < 31 {
                    num >>= 1;
                    shift += 1;
                }
                ((num / (count(x) as i64 - 1)) << shift) as f64
            }
            Term::Recip { .. } => f64::NEG_INFINITY,
        })
        .sum()
}

/// The channel's period program fits a program, and its value is a period (at least 1) that
/// never wraps, whatever its inputs.
pub(crate) fn period_ok(ch: &ChannelModel) -> bool {
    encode_prog(&period_prog(ch)).len() <= SND_PROG_MAX
        && sum_max(&ch.half_period, PERIOD_SCALE) <= f64::from(u32::MAX)
        && sum_min(&ch.half_period, PERIOD_SCALE) >= 1.0
}

/// The timing program, `((start - counter) & mask) x per_unit_t`: None without a start, or if
/// it would not fit a program, could overflow, or a pass could take less than 1 T.
pub(crate) fn timing_prog(d: &DurationModel) -> Option<Vec<Op>> {
    let wide = d.counter.wide();
    let mask: u32 = if wide { 0xFFFF } else { 0xFF };
    let start = d.counts_down_from?;
    let mut ops = Vec::new();
    if d.carry {
        // passes to go at the start, less those at the edge (see DurationModel)
        let (sh, sl) = halves(&start)?;
        let (ch, cl) = halves(&d.counter)?;
        for (h, l) in [(sh, sl), (ch, cl)] {
            ops.extend([h, Op::K(1), Op::Sub, Op::K(0xFF), Op::And, Op::Shl(8), l, Op::K(1), Op::Sub, Op::K(0xFF), Op::And, Op::Add, Op::K(1), Op::Add]);
        }
        ops.extend([Op::Sub, Op::K(mask), Op::And]);
    } else {
        match start {
            Input::Indexed(r, dd) if wide => ops.extend([Op::PeekReg(r, dd), Op::PeekReg(r, dd.checked_add(1)?), Op::Shl(8), Op::Add]),
            x => ops.extend(x.ops()),
        }
        ops.extend(d.counter.ops());
        ops.extend([Op::Sub, Op::K(mask), Op::And]);
    }
    push_sum(&mut ops, &d.per_unit_t, 1);
    ops.push(Op::Mul);
    let fits = encode_prog(&ops).len() <= SND_PROG_MAX && f64::from(mask) * sum_max(&d.per_unit_t, 1) <= f64::from(u32::MAX);
    (fits && sum_min(&d.per_unit_t, 1) >= 1.0).then_some(ops)
}

/// The ops that push a 16-bit input's high and low bytes.
fn halves(x: &Input) -> Option<(Op, Op)> {
    Some(match *x {
        Input::Rel16(o) => (Op::PeekRel(o.checked_add(1)?), Op::PeekRel(o)),
        Input::Abs16(a) => (Op::Peek(a.checked_add(1)?), Op::Peek(a)),
        Input::Indexed(r, d) => (Op::PeekReg(r, d.checked_add(1)?), Op::PeekReg(r, d)),
        Input::Reg(8) => (Op::Reg(2), Op::Reg(3)),
        Input::Reg(9) => (Op::Reg(4), Op::Reg(5)),
        Input::Reg(10) => (Op::Reg(6), Op::Reg(7)),
        _ => return None,
    })
}
