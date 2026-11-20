// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The observing learner: a heard routine's channels found from what the speaker plays, not
//! from its code. For engines that mix several channels into the one speaker bit, where neither
//! the code reader ([`crate::model`]) nor a period per `OUT` site works: the time between one
//! site's flips says nothing about any one channel.
//!
//! # How it observes
//!
//! A routine's channels each keep their pitch in a byte or word that stays the same for a note:
//! a self-modified immediate operand the loop reads, a variable, or a register at the `OUT`.
//! Given a game and one heard speaker `OUT` (the *site*), [`observe`] runs the game twice under
//! the same inputs:
//!
//! 1. **Traced**, from the site's first sounding frame for at most [`TRACE_SOUND_FRAMES`] frames
//!    it sounds in: the instructions executed within [`REACH`] bytes of the site, the memory
//!    they read (reader pc, read index in the instruction, address, value, and the registers at
//!    the instruction), the writes into that range, the times of every `OUT` at the site (also
//!    those that leave the level), and the registers at the site's last flip of each frame
//!    ([`Machine::out_sites`]).
//!    - The **routine** is the hot code around the site: the instructions run, in the frames the
//!      site sounds, at least [`HOT_SHARE`] as often as the site's `OUT`, taken outward from the
//!      site while the gaps between them are at most [`GAP`] bytes; its range, from the first to
//!      the end of the last, at most `SND_LEARNED_LEN_MAX` bytes. Its start is the anchor; the
//!      bytes written into it are the signature's wild bytes; the `OUT`s in it that made edges
//!      are its sites.
//!    - The **pass** is the median time between consecutive `OUT`s at the site, contention
//!      taken out (an `OUT` that some passes skip runs at multiples of it: the finest unit that
//!      a sixth of the times show is taken); refused unless three in four times are whole
//!      multiples of it. Its time as played is the mean of the one-pass times, contention in.
//!    - The **candidates** are each byte the routine reads that took more than one value: at an
//!      offset from the anchor ([`Input::Rel`]) for a byte inside the routine (a self-modified
//!      immediate), at an address ([`Input::Abs`]) when it is always the same, else at a register
//!      plus a displacement ([`Input::Indexed`]) when that is always the address; the words of
//!      two bytes an instruction reads together; and the registers at the site's last flip
//!      ([`Input::Reg`], bytes and pairs, but F, and the byte halves of IX and IY:
//!      [`Input::RegHalf`]), each taking more than one value.
//! 2. **Untraced**, the whole run: each frame's speaker level goes into a [`Signal`] of the
//!    last [`MAX_FRAMES`] frames, and each candidate is read as the device reads it: memory at
//!    the frame's end, registers at the site's last flip. Every [`HOP`] frames (at most
//!    [`MAX_WINDOWS`] windows, spread over the run), each candidate's mappings predict a pitch
//!    from its value, tested with [`View::present`](crate::spectrum::View::present) over the
//!    last few frames: as many as the pitch needs to be told from its semitone neighbours
//!    ([`window_frames`]: 4 from 420 Hz up, up to 8 below), as far as the value has stayed the
//!    same (the note), at least [`WINDOW_FRAMES`]; the site must have sounded in at least half
//!    of those frames.
//!
//! # Mappings and score
//!
//! - **Counter**: the channel's period is `k x count(v)` T-states, `k` = 1 or 2 passes,
//!   `count(0)` = 256 (65,536 for a word): a counter reloaded from `v` that starts a pulse when
//!   it runs out (`k` = 1) or toggles a level (`k` = 2).
//! - **Accumulator**: `pass x 2^bits / v`, 8 bits for a byte and 16 for a word: a phase
//!   accumulator stepped by `v` each pass whose carry makes the sound; 0 is silent (no window).
//!
//! The *score* of a mapping is the share of its windows in which its pitch is present; a
//! window whose pitch is outside [`F_MIN`]-[`F_MAX`] Hz is a miss. A pulse train has a strong
//! second harmonic, so a prediction an octave too high finds a peak too: each candidate keeps
//! the mapping with the most *clean* windows, its pitch present and the octave below not (for a
//! counter: `k` = 1 against `k` = 2, and for `k` = 2 the octave below again), among those with
//! at least [`MIN_WINDOWS`] windows. A candidate must also take at least three values over the
//! frames the site sounds in and keep its value from one such frame to the next in at least half
//! of them (the loop's own counters change every pass). The channels are the best of those
//! scoring at least [`THRESHOLD`], at most `SND_TONAL`, strongest first, none predicting the
//! same pitch (within half a semitone) as a stronger one in four in five of the windows they
//! share (one value read from two places, or a counter caught one step after its reload).

use crate::listen::Inputs;
use crate::model::Input;
use crate::prog::Op;
use crate::spectrum::{Signal, CLOCK_HZ};
use crate::z80::decode;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use zx_sys::{zx_regs, Machine, TraceEv, SND_LEARNED_LEN_MAX, SND_TONAL, ZX_TR_INSN, ZX_TR_INTA, ZX_TR_OUT, ZX_TR_READ, ZX_TR_WRITE};

/// T-states between the signal's samples.
pub const DT: u32 = 128;
/// Frames in the shortest spectral window: 80 ms. A pitch whose semitone is narrower than
/// that window's Hann main lobe (two bins, 25 Hz: below 420 Hz) is tested over a longer one
/// ([`window_frames`]), up to [`MAX_FRAMES`], as far as its note lasts (and the site sounds).
pub const WINDOW_FRAMES: usize = 4;
/// Frames in the longest window: 160 ms.
pub const MAX_FRAMES: usize = 8;
/// Frames between windows tested.
pub const HOP: u32 = 2;
/// The least score a channel needs. On the test engines a true channel scores 0.98 or more. On
/// real multi-channel tunes (a 4-channel counter engine, a 4-channel accumulator engine, a
/// 2-channel toggling engine, an 8-channel summed-pulse engine) the channels found score
/// 0.43-0.99, and the values that are no channel's (counters caught mid-count, pulse lengths,
/// a byte of a channel's word) at most 0.37.
pub const THRESHOLD: f64 = 0.4;
/// Windows a mapping needs to be scored at all.
pub const MIN_WINDOWS: u32 = 16;
/// Windows tested at most: when more come, every other one is dropped and the stride between
/// those tested doubles, so they always span the whole run.
pub const MAX_WINDOWS: u32 = 512;
/// The pitches a channel may play, Hz.
pub const F_MIN: f64 = 25.0;
pub const F_MAX: f64 = 6000.0;
/// Frames the site sounds in that the traced run covers.
pub const TRACE_SOUND_FRAMES: u32 = 512;
/// Bytes either side of the site the traced run looks at.
pub const REACH: u16 = 1024;
/// Hot code: run at least this share of the site's `OUT`s.
pub const HOT_SHARE: f64 = 0.25;
/// The largest gap between hot instructions of one routine, bytes.
pub const GAP: u16 = 64;

/// How a channel's value gives its period.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mapping {
    /// `mult x pass x count(v)` T-states; `count(0)` is 256, or 65,536 for a 16-bit value.
    Counter { mult: u8 },
    /// `pass x 2^bits / v` T-states; 0 is silent.
    Accumulator { bits: u8 },
}

impl Mapping {
    /// The period in T-states for value `v` (`wide`: a 16-bit value) and the pass time; None
    /// when silent.
    pub fn period_t(&self, v: u32, wide: bool, pass_t: f64) -> Option<f64> {
        match *self {
            Mapping::Counter { mult } => {
                let count = if v != 0 { f64::from(v) } else if wide { 65536.0 } else { 256.0 };
                Some(f64::from(mult) * pass_t * count)
            }
            Mapping::Accumulator { bits } => (v != 0).then(|| pass_t * f64::from(1u32 << bits) / f64::from(v)),
        }
    }
}

/// One channel the observation found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObservedChannel {
    pub input: Input,
    pub mapping: Mapping,
    /// The share of its windows in which its pitch is present.
    pub score: f64,
    /// The share in which its pitch is present and the octave below is not.
    pub clean: f64,
    /// Windows tested.
    pub windows: u32,
}

/// A routine found by observing it: everything an entry needs.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedModel {
    /// The heard site observed.
    pub site: u16,
    /// The routine's first byte.
    pub anchor: u16,
    /// Its length in bytes (at most `SND_LEARNED_LEN_MAX`).
    pub len: u16,
    /// Offsets from the anchor of the bytes written while it played (self-modified operands,
    /// variables inside it): the signature's wild bytes. Ascending.
    pub wild: Vec<u16>,
    /// Offsets from the anchor of its `OUT`s that made speaker edges. Ascending.
    pub sites: Vec<u16>,
    /// T-states from one pass to the next, as played (contention in).
    pub pass_t: f64,
    /// The contention in that, on average.
    pub contention_t: f64,
    /// Strongest first, at most `SND_TONAL`.
    pub channels: Vec<ObservedChannel>,
    /// The best-scored candidates (kept or not, at most 8), for the report.
    pub considered: Vec<ObservedChannel>,
    /// Frames the site sounded in.
    pub music_frames: u32,
}

impl ObservedModel {
    /// Channel `ch`'s period in T-states from the memory and the registers at the site's last
    /// flip; None when silent or unreadable.
    pub fn period_t(&self, ch: usize, peek: &dyn Fn(u16) -> u8, regs: &zx_regs) -> Option<f64> {
        let c = self.channels.get(ch)?;
        let v = c.input.read(peek, self.anchor, regs)?;
        c.mapping.period_t(v, c.input.wide(), self.pass_t)
    }

    /// Channel `ch`'s program: its period in T-states x 256 (empty for no such channel).
    /// Counter: `((x + 255) & 255) + 1` (0 counts 256; 65,535 for a word) times the pass x 256
    /// x `mult`. Accumulator: silent for 0, else the pass x 256 x `2^bits` over `x`. A constant
    /// past 32 bits is shifted down and the result back up.
    pub fn program(&self, ch: usize) -> Vec<Op> {
        let Some(c) = self.channels.get(ch) else { return Vec::new() };
        let x = c.input.ops();
        let pass256 = (self.pass_t * 256.0).round().max(1.0) as u64;
        let wide = c.input.wide();
        let shift_for = |k: u64, most: u64| (0..32u8).find(|&s| most.saturating_mul(k >> s) <= u64::from(u32::MAX)).unwrap_or(31);
        match c.mapping {
            Mapping::Counter { mult } => {
                let (mask, most) = if wide { (0xFFFFu32, 65536u64) } else { (0xFFu32, 256u64) };
                let k = pass256 * u64::from(mult);
                let s = shift_for(k, most);
                let mut ops = x;
                ops.extend([Op::K(mask), Op::Add, Op::K(mask), Op::And, Op::K(1), Op::Add, Op::K((k >> s) as u32), Op::Mul]);
                if s > 0 {
                    ops.push(Op::Shl(s));
                }
                ops
            }
            Mapping::Accumulator { bits } => {
                let k = pass256 << bits;
                let s = shift_for(k, 1);
                let mut ops = x.clone();
                ops.push(Op::ZeroSilent);
                ops.push(Op::K((k >> s) as u32));
                ops.extend(x);
                ops.push(Op::Div);
                if s > 0 {
                    ops.push(Op::Shl(s));
                }
                ops
            }
        }
    }
}

/// All registers zero.
fn no_regs() -> zx_regs {
    zx_regs { af: 0, bc: 0, de: 0, hl: 0, af_: 0, bc_: 0, de_: 0, hl_: 0, ix: 0, iy: 0, sp: 0, pc: 0, i: 0, r: 0, iff1: 0, iff2: 0, im: 0 }
}

/// Up to 4 distinct values.
#[derive(Clone, Copy, Debug, Default)]
struct Few {
    v: [u32; 4],
    n: u8,
}

impl Few {
    fn push(&mut self, x: u32) {
        let n = usize::from(self.n);
        if n < 4 && !self.v[..n].contains(&x) {
            self.v[n] = x;
            self.n += 1;
        }
    }
}

/// A register's address displacement at a read: unseen, always the same, or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disp {
    Unset,
    Same(i8),
    Mixed,
}

/// What one (reader pc, read index) read over the traced run.
#[derive(Clone, Copy, Debug)]
struct ReadKey {
    addr: u16,
    addr_const: bool,
    /// Per register 8-12 (BC, DE, HL, IX, IY): the address less the register.
    disp: [Disp; 5],
    vals: Few,
}

/// The traced run's recorder (the trace hook's state).
struct Trace {
    site: u16,
    /// The first address looked at: `site - REACH`.
    base: u16,
    /// This frame's executions, by pc - base.
    counts: Vec<u32>,
    insn_regs: zx_regs,
    read_idx: u8,
    reads: HashMap<(u16, u8), ReadKey>,
    writes: BTreeSet<u16>,
    /// The site's last `OUT` this frame: (T-state, contention so far).
    last_out: Option<(u32, u32)>,
    /// Between consecutive `OUT`s at the site: (T-states, contention).
    intervals: Vec<(u32, u32)>,
}

const MAX_INTERVALS: usize = 1 << 17;

impl Trace {
    fn near(&self, a: u16) -> bool {
        usize::from(a.wrapping_sub(self.base)) < self.counts.len()
    }

    fn event(&mut self, ev: &TraceEv) {
        match u32::from(ev.kind) {
            ZX_TR_INSN => {
                self.read_idx = 0;
                if let Some(c) = self.counts.get_mut(usize::from(ev.pc.wrapping_sub(self.base))) {
                    *c += 1;
                    self.insn_regs = ev.regs;
                }
            }
            ZX_TR_READ => {
                if self.near(ev.pc) {
                    let r = &self.insn_regs;
                    let pairs = [r.bc, r.de, r.hl, r.ix, r.iy];
                    let k = self.reads.entry((ev.pc, self.read_idx)).or_insert(ReadKey {
                        addr: ev.addr,
                        addr_const: true,
                        disp: [Disp::Unset; 5],
                        vals: Few::default(),
                    });
                    k.addr_const &= k.addr == ev.addr;
                    k.vals.push(ev.val.into());
                    for (d, &p) in k.disp.iter_mut().zip(&pairs) {
                        let off = ev.addr.wrapping_sub(p) as i16;
                        let now = i8::try_from(off).ok();
                        *d = match (*d, now) {
                            (Disp::Unset, Some(x)) => Disp::Same(x),
                            (Disp::Same(a), Some(x)) if a == x => Disp::Same(a),
                            _ => Disp::Mixed,
                        };
                    }
                }
                self.read_idx = self.read_idx.saturating_add(1);
            }
            ZX_TR_WRITE => {
                if self.near(ev.addr) {
                    self.writes.insert(ev.addr);
                }
            }
            ZX_TR_OUT if ev.pc == self.site && ev.addr & 1 == 0 => {
                if let Some((t0, c0)) = self.last_out {
                    if ev.t > t0 && self.intervals.len() < MAX_INTERVALS {
                        self.intervals.push((ev.t - t0, ev.cont.wrapping_sub(c0)));
                    }
                }
                self.last_out = Some((ev.t, ev.cont));
            }
            ZX_TR_INTA => self.last_out = None,
            _ => {}
        }
    }
}

/// The byte halves of IX and IY, candidates beside the registers (a counter reloaded from IXh).
const HALVES: [Input; 4] = [Input::RegHalf(11, true), Input::RegHalf(11, false), Input::RegHalf(12, true), Input::RegHalf(12, false)];

/// What the traced run found.
struct Traced {
    anchor: u16,
    len: u16,
    wild: Vec<u16>,
    sites: Vec<u16>,
    pass_t: f64,
    contention_t: f64,
    candidates: Vec<Input>,
}

fn sounds(m: &Machine, site: u16) -> bool {
    m.edges().iter().any(|e| e.pc == site)
}

fn site_regs(m: &Machine, site: u16) -> Option<zx_regs> {
    m.out_sites().into_iter().find(|s| s.pc == site).map(|s| s.regs)
}

/// The pass from the times between the site's `OUT`s, each (T-states, contention in them):
/// the unit is found from the times without contention (the `OUT`'s own I/O contention varies
/// from pass to pass); the pass is the mean time of the one-unit intervals as played. Returns
/// (pass as played, its contention).
fn pass_time(intervals: &[(u32, u32)]) -> Result<(f64, f64), String> {
    if intervals.len() < 32 {
        return Err(format!("the site's OUT ran too few times to time the pass ({})", intervals.len()));
    }
    let net = |&(t, c): &(u32, u32)| f64::from(t.saturating_sub(c));
    let mut ts: Vec<f64> = intervals.iter().map(net).collect();
    ts.sort_by(f64::total_cmp);
    let med = ts[ts.len() / 2];
    let near = |x: f64, p: f64| (x - p).abs() <= 0.03 * p;
    let share = |p: f64| ts.iter().filter(|&&x| near(x, p)).count() as f64 / ts.len() as f64;
    let mut unit = med;
    for d in [4.0, 3.0, 2.0] {
        if share(med / d) >= 1.0 / 6.0 {
            unit = med / d;
            break;
        }
    }
    let whole = ts.iter().filter(|&&x| (1..=8).any(|k| near(x, unit * f64::from(k)))).count() as f64 / ts.len() as f64;
    if whole < 0.75 {
        return Err(format!("the pass time varies: {:.0}% of the times between the site's OUTs are whole multiples of {unit:.0} T", 100.0 * whole));
    }
    let one: Vec<&(u32, u32)> = intervals.iter().filter(|i| near(net(i), unit)).collect();
    let n = one.len().max(1) as f64;
    let pass = one.iter().map(|&&(t, _)| f64::from(t)).sum::<f64>() / n;
    let cont = one.iter().map(|&&(_, c)| f64::from(c)).sum::<f64>() / n;
    Ok((pass, cont))
}

fn traced(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, site: u16) -> Result<Traced, String> {
    let mut m = load();
    let mut f = 0u32;
    // untraced up to the site's first sounding frame
    loop {
        if f >= frames {
            return Err(format!("the site {site:04X} never sounds"));
        }
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        f += 1;
        if sounds(&m, site) {
            break;
        }
    }
    let span = usize::from(REACH) * 2;
    let base = site.wrapping_sub(REACH);
    let rec = Rc::new(RefCell::new(Trace {
        site,
        base,
        counts: vec![0; span],
        insn_regs: no_regs(),
        read_idx: 0,
        reads: HashMap::new(),
        writes: BTreeSet::new(),
        last_out: None,
        intervals: Vec::new(),
    }));
    let hook = Rc::clone(&rec);
    // the hook must never panic: a panic inside the C callback aborts the process
    m.set_trace(
        Some(Box::new(move |ev: &TraceEv| {
            if let Ok(mut r) = hook.try_borrow_mut() {
                r.event(ev);
            }
        })),
        (1 << ZX_TR_INSN) | (1 << ZX_TR_READ) | (1 << ZX_TR_WRITE) | (1 << ZX_TR_OUT) | (1 << ZX_TR_INTA),
    );
    let mut hot = vec![0u64; span];
    let mut sounding = 0u32;
    let mut regs_seen = [Few::default(); 18];
    let mut halves_seen = [Few::default(); 4];
    let mut edge_sites: BTreeSet<u16> = BTreeSet::new();
    while f < frames && sounding < TRACE_SOUND_FRAMES {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        f += 1;
        let mut r = rec.borrow_mut();
        if sounds(&m, site) {
            sounding += 1;
            for (h, &c) in hot.iter_mut().zip(&r.counts) {
                *h += u64::from(c);
            }
            if let Some(regs) = site_regs(&m, site) {
                for (i, few) in regs_seen.iter_mut().enumerate() {
                    if let Some(v) = crate::prog::reg(&regs, i as u8) {
                        few.push(v);
                    }
                }
                for (few, half) in halves_seen.iter_mut().zip(HALVES) {
                    if let Some(v) = half.read(&|_| 0, 0, &regs) {
                        few.push(v);
                    }
                }
            }
            edge_sites.extend(m.edges().iter().map(|e| e.pc).filter(|&pc| usize::from(pc.wrapping_sub(base)) < span));
        }
        r.counts.fill(0);
        r.last_out = None;
    }
    m.set_trace(None, 0);
    let r = rec.borrow();

    // the routine: hot code around the site
    let at = |pc: u16| usize::from(pc.wrapping_sub(base));
    let avg = |i: usize| hot[i] as f64 / f64::from(sounding.max(1));
    let site_avg = avg(at(site));
    let is_hot = |i: usize| avg(i) >= (HOT_SHARE * site_avg).max(1.0);
    let (mut lo, mut hi) = (at(site), at(site));
    let mut i = lo;
    while i > 0 && lo - i <= usize::from(GAP) {
        i -= 1;
        if is_hot(i) {
            lo = i;
        }
    }
    let mut i = hi;
    while i + 1 < span && i - hi <= usize::from(GAP) {
        i += 1;
        if is_hot(i) {
            hi = i;
        }
    }
    let anchor = base.wrapping_add(lo as u16);
    let last = base.wrapping_add(hi as u16);
    let end = usize::from(decode(|a| m.peek(a), last).len.max(1)) + hi;
    let len = end - lo;
    if len > usize::from(SND_LEARNED_LEN_MAX) {
        return Err(format!("the routine's loop at {anchor:04X} spans {len} bytes (at most {SND_LEARNED_LEN_MAX})"));
    }
    let len = len as u16;
    let inside = |a: u16| a.wrapping_sub(anchor) < len;

    let (pass_t, contention_t) = pass_time(&r.intervals)?;

    let mut cands: BTreeSet<Input> = BTreeSet::new();
    for (&(pc, idx), k) in &r.reads {
        if !inside(pc) {
            continue;
        }
        if k.addr_const {
            let next = r.reads.get(&(pc, idx.wrapping_add(1))).filter(|n| n.addr_const && n.addr == k.addr.wrapping_add(1));
            if k.vals.n >= 2 {
                cands.insert(if inside(k.addr) { Input::Rel(k.addr.wrapping_sub(anchor)) } else { Input::Abs(k.addr) });
            }
            if let Some(n) = next.filter(|n| k.vals.n >= 2 || n.vals.n >= 2) {
                let _ = n;
                cands.insert(if inside(k.addr) { Input::Rel16(k.addr.wrapping_sub(anchor)) } else { Input::Abs16(k.addr) });
            }
        } else if k.vals.n >= 2 {
            if let Some((r, d)) = k.disp.iter().enumerate().find_map(|(i, d)| match d {
                Disp::Same(x) => Some((8 + i as u8, *x)),
                _ => None,
            }) {
                cands.insert(Input::Indexed(r, d));
            }
        }
    }
    for (i, few) in regs_seen.iter().enumerate() {
        if i != 1 && few.n >= 2 {
            cands.insert(Input::Reg(i as u8));
        }
    }
    for (few, half) in halves_seen.iter().zip(HALVES) {
        if few.n >= 2 {
            cands.insert(half);
        }
    }

    let mut wild: BTreeSet<u16> = r.writes.iter().filter(|&&a| inside(a)).map(|&a| a.wrapping_sub(anchor)).collect();
    let sites: Vec<u16> = edge_sites.iter().filter(|&&pc| inside(pc)).map(|&pc| pc.wrapping_sub(anchor)).collect();
    // a self-modified byte the loop reads is wild even if its writes fell outside the trace
    for c in &cands {
        match *c {
            Input::Rel(o) => {
                wild.insert(o);
            }
            Input::Rel16(o) => {
                wild.insert(o);
                if o + 1 < len {
                    wild.insert(o + 1);
                }
            }
            _ => {}
        }
    }
    Ok(Traced { anchor, len, wild: wild.into_iter().collect(), sites, pass_t, contention_t, candidates: cands.into_iter().collect() })
}

/// One window's test of a mapping.
#[derive(Clone, Copy, Debug)]
struct Test {
    /// The window's number (windows are numbered as they come, tested or not).
    window: u32,
    hz: f32,
    present: bool,
    /// Present, and the octave below is not.
    clean: bool,
}

/// A mapping's tests over the windows.
#[derive(Clone, Debug)]
struct Tally {
    mapping: Mapping,
    tests: Vec<Test>,
}

impl Tally {
    fn rate(&self, f: impl Fn(&Test) -> bool) -> f64 {
        self.tests.iter().filter(|t| f(t)).count() as f64 / self.tests.len().max(1) as f64
    }
}

/// A candidate through the untraced run.
#[derive(Clone, Debug)]
struct Cand {
    input: Input,
    last: Option<u32>,
    /// The frame its current value started in.
    since: u32,
    /// Over the frames the site sounds in: distinct values, frames compared with the previous
    /// one, and how many of those kept the value.
    distinct: Few,
    pairs: u32,
    same: u32,
    prev: Option<u32>,
    tallies: Vec<Tally>,
}

impl Cand {
    fn new(input: Input) -> Self {
        let mappings: &[Mapping] = if input.wide() {
            &[Mapping::Counter { mult: 1 }, Mapping::Counter { mult: 2 }, Mapping::Accumulator { bits: 16 }]
        } else {
            &[Mapping::Counter { mult: 1 }, Mapping::Counter { mult: 2 }, Mapping::Accumulator { bits: 8 }]
        };
        Cand {
            input,
            last: None,
            since: 0,
            distinct: Few::default(),
            pairs: 0,
            same: 0,
            prev: None,
            tallies: mappings.iter().map(|&mapping| Tally { mapping, tests: Vec::new() }).collect(),
        }
    }
}

/// The frames a pitch is tested over: enough that a semitone is at least the Hann main lobe's
/// half width (`2 / window` Hz), from [`WINDOW_FRAMES`] up to [`MAX_FRAMES`], in steps of 2.
pub fn window_frames(hz: f64) -> usize {
    let frame_s = f64::from(zx_sys::ZX_FRAME_TSTATES) / CLOCK_HZ;
    let need = 2.0 / (hz * 0.059_463) / frame_s;
    let mut k = WINDOW_FRAMES;
    while (k as f64) < need && k < MAX_FRAMES {
        k += 2;
    }
    k.min(MAX_FRAMES)
}

/// The site sounded in at least half of the last `k` frames, the last one among them (`heard`:
/// bit i for i frames ago). A sparse engine's channels can all be between pulses for a frame.
fn sounded_in(heard: u32, k: usize) -> bool {
    let mask = if k >= 32 { u32::MAX } else { (1u32 << k) - 1 };
    heard & 1 != 0 && (heard & mask).count_ones() as usize * 2 >= k
}

/// One window's tests, each (frames, frequency)'s result kept.
struct Window<'a> {
    sig: &'a Signal,
    variance: HashMap<usize, f64>,
    seen: HashMap<(usize, u64), bool>,
}

impl Window<'_> {
    fn present(&mut self, hz: f64, frames: usize) -> bool {
        if !(F_MIN..=F_MAX).contains(&hz) {
            return false;
        }
        let sig = self.sig;
        let var = *self.variance.entry(frames).or_insert_with(|| sig.last(frames).variance());
        *self.seen.entry((frames, hz.to_bits())).or_insert_with(|| sig.last(frames).present_in(hz, var))
    }
}

/// Observes the routine of the heard speaker `OUT` at `site`: runs the game from `load` for
/// `frames` frames under `inputs`, twice (traced, then untraced; see the module docs), and finds
/// its channels, or says why not.
pub fn observe(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, site: u16) -> Result<ObservedModel, String> {
    let t = traced(load, inputs, frames, site)?;
    if t.candidates.is_empty() {
        return Err("the loop reads no value that changes".into());
    }
    let mut cands: Vec<Cand> = t.candidates.iter().map(|&i| Cand::new(i)).collect();
    let mut m = load();
    let mut sig = Signal::new(DT, MAX_FRAMES);
    // bit i: the site sounded i frames ago
    let (mut heard, mut music) = (0u32, 0u32);
    let (mut window_no, mut stride, mut tested) = (0u32, 1u32, 0u32);
    let mut prev_sounded = false;
    let zero = no_regs();
    for f in 0..frames {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        sig.push_frame(m.edges(), m.edges_start_level());
        let sounded = sounds(&m, site);
        heard = heard << 1 | u32::from(sounded);
        music += u32::from(sounded);
        let regs = site_regs(&m, site);
        let peek = |a: u16| m.peek(a);
        for c in &mut cands {
            let v = match (c.input, regs.as_ref()) {
                (Input::Reg(_) | Input::RegHalf(..) | Input::Indexed(..), None) => None,
                (i, r) => i.read(&peek, t.anchor, r.unwrap_or(&zero)),
            };
            if v != c.last || f == 0 {
                c.last = v;
                c.since = f;
            }
            if sounded {
                if let Some(x) = v {
                    c.distinct.push(x);
                }
                if prev_sounded {
                    c.pairs += 1;
                    c.same += u32::from(v == c.prev);
                }
                c.prev = v;
            }
        }
        prev_sounded = sounded;
        if !sounded_in(heard, WINDOW_FRAMES) || f % HOP != 0 {
            continue;
        }
        let no = window_no;
        window_no += 1;
        if no % stride != 0 {
            continue;
        }
        let mut w = Window { sig: &sig, variance: HashMap::new(), seen: HashMap::new() };
        for c in &mut cands {
            let Some(v) = c.last else { continue };
            let held = f + 1 - c.since;
            let wide = c.input.wide();
            for tl in &mut c.tallies {
                let Some(period) = tl.mapping.period_t(v, wide, t.pass_t) else { continue };
                let hz = CLOCK_HZ / period;
                // the longest window the note and the sound allow, up to what the pitch needs
                let Some(k) = (WINDOW_FRAMES..=window_frames(hz)).rev().step_by(2).find(|&k| held >= k as u32 && sounded_in(heard, k)) else { continue };
                let present = w.present(hz, k);
                let clean = present && !w.present(hz / 2.0, window_frames(hz / 2.0));
                tl.tests.push(Test { window: no, hz: hz as f32, present, clean });
            }
        }
        tested += 1;
        if tested >= MAX_WINDOWS {
            stride *= 2;
            tested = tested.div_ceil(2);
            for tl in cands.iter_mut().flat_map(|c| c.tallies.iter_mut()) {
                tl.tests.retain(|t| t.window % stride == 0);
            }
        }
    }

    // each candidate's best mapping (by its clean windows: a counter's octave), if its values
    // look like notes
    let mut scored: Vec<(ObservedChannel, Vec<(u32, f32)>)> = Vec::new();
    for c in cands {
        if c.distinct.n < 3 || c.pairs == 0 || f64::from(c.same) < 0.5 * f64::from(c.pairs) {
            continue;
        }
        let best = c.tallies.into_iter().filter(|tl| tl.tests.len() >= MIN_WINDOWS as usize).max_by(|a, b| a.rate(|t| t.clean).total_cmp(&b.rate(|t| t.clean)));
        if let Some(tl) = best {
            let ch = ObservedChannel { input: c.input, mapping: tl.mapping, score: tl.rate(|t| t.present), clean: tl.rate(|t| t.clean), windows: tl.tests.len() as u32 };
            scored.push((ch, tl.tests.iter().map(|t| (t.window, t.hz)).collect()));
        }
    }
    scored.sort_by(|a, b| b.0.score.total_cmp(&a.0.score).then(a.0.input.cmp(&b.0.input)));
    let considered: Vec<ObservedChannel> = scored.iter().take(8).map(|s| s.0).collect();
    let mut chosen: Vec<(ObservedChannel, &Vec<(u32, f32)>)> = Vec::new();
    for (ch, log) in &scored {
        if chosen.len() >= SND_TONAL || ch.score < THRESHOLD {
            break;
        }
        if chosen.iter().any(|(_, other)| same_pitch(log, other)) {
            continue;
        }
        chosen.push((*ch, log));
    }
    if chosen.is_empty() {
        return Err(match considered.first() {
            Some(b) => format!(
                "no value the loop reads follows the pitch heard (best: {}, {}, {:.0}% of {} windows; at least {:.0}%)",
                crate::report::input(&b.input, t.anchor),
                crate::report::mapping(b.mapping),
                100.0 * b.score,
                b.windows,
                100.0 * THRESHOLD
            ),
            None => "no value the loop reads holds a note (none changes from note to note and stays for one)".into(),
        });
    }
    Ok(ObservedModel {
        site,
        anchor: t.anchor,
        len: t.len,
        wild: t.wild,
        sites: t.sites,
        pass_t: t.pass_t,
        contention_t: t.contention_t,
        channels: chosen.into_iter().map(|c| c.0).collect(),
        considered,
        music_frames: music,
    })
}

/// Two mappings predict the same pitch (within half a semitone, 3%: closer than the spectrum
/// tells apart) in four in five of the windows both were tested in (and they share at least 8).
fn same_pitch(a: &[(u32, f32)], b: &[(u32, f32)]) -> bool {
    let bm: BTreeMap<u32, f32> = b.iter().copied().collect();
    let (mut both, mut same) = (0u32, 0u32);
    for &(w, fa) in a {
        if let Some(&fb) = bm.get(&w) {
            both += 1;
            same += u32::from((fa / fb - 1.0).abs() < 0.03);
        }
    }
    both >= 8 && same * 5 >= both * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prog::eval_prog;

    fn model(input: Input, mapping: Mapping, pass_t: f64) -> ObservedModel {
        let ch = ObservedChannel { input, mapping, score: 1.0, clean: 1.0, windows: 100 };
        ObservedModel {
            site: 0x8010,
            anchor: 0x8000,
            len: 0x40,
            wild: vec![],
            sites: vec![0x10],
            pass_t,
            contention_t: 0.0,
            channels: vec![ch],
            considered: vec![ch],
            music_frames: 100,
        }
    }

    /// Each program, run by the reference evaluator, gives the model's period x 256.
    #[test]
    fn programs_compute_the_models_period() {
        let mut mem = vec![0u8; 0x10000];
        let inputs = [Input::Rel(5), Input::Abs(0x9000), Input::Rel16(6), Input::Abs16(0x9002), Input::Indexed(11, -3), Input::Reg(4), Input::Reg(9), Input::RegHalf(11, true), Input::RegHalf(12, false)];
        let mappings = [Mapping::Counter { mult: 1 }, Mapping::Counter { mult: 2 }, Mapping::Accumulator { bits: 8 }, Mapping::Accumulator { bits: 16 }];
        let regs = zx_regs { de: 0x1234, ix: 0x9103, ..no_regs() };
        for &pass in &[174.0, 97.5, 260.0, 1000.0] {
            for &input in &inputs {
                for &mapping in &mappings {
                    let om = model(input, mapping, pass);
                    let prog = om.program(0);
                    for v in [0u32, 1, 2, 77, 255, 256, 4321, 65535] {
                        let (lo, hi) = ((v & 0xFF) as u8, (v >> 8) as u8);
                        let wide = input.wide();
                        if !wide && v > 255 {
                            continue;
                        }
                        // place v where the input reads it
                        let mut r = regs;
                        match input {
                            Input::Rel(o) => mem[0x8000 + usize::from(o)] = lo,
                            Input::Abs(a) => mem[usize::from(a)] = lo,
                            Input::Rel16(o) => {
                                mem[0x8000 + usize::from(o)] = lo;
                                mem[0x8001 + usize::from(o)] = hi;
                            }
                            Input::Abs16(a) => {
                                mem[usize::from(a)] = lo;
                                mem[usize::from(a) + 1] = hi;
                            }
                            Input::Indexed(..) => mem[0x9100] = lo,
                            Input::Reg(4) => r.de = u16::from(lo) << 8 | 0x34,
                            Input::Reg(_) => r.de = v as u16,
                            Input::RegHalf(_, true) => r.ix = u16::from(lo) << 8 | 0x03,
                            Input::RegHalf(..) => r.iy = 0xAB00 | u16::from(lo),
                        }
                        let peek = |a: u16| mem[usize::from(a)];
                        let want = om.period_t(0, &peek, &r);
                        let got = eval_prog(&prog, peek, om.anchor, &r).expect("valid program");
                        match (want, got) {
                            (None, None) => {}
                            (Some(w), Some(g)) if w * 256.0 > f64::from(u32::MAX) => {
                                // out of the program's range: the period is inaudible anyway
                                let _ = g;
                            }
                            (Some(w), Some(g)) => {
                                let g = f64::from(g) / 256.0;
                                // the integer division floors: within 1/256 T (x 2^shift)
                                assert!((g - w).abs() <= 2e-3 * w + 0.05, "{input:?} {mapping:?} pass {pass} v {v}: {g} vs {w}");
                            }
                            other => panic!("{input:?} {mapping:?} pass {pass} v {v}: {other:?}"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_pass_is_the_finest_whole_unit() {
        let iv = |ts: &[u32]| ts.iter().map(|&t| (t, 0)).collect::<Vec<_>>();
        // the OUT's contention varies: the unit is found without it, the pass is as played
        let cont: Vec<(u32, u32)> = (0..100).map(|i| (174 + i % 7, i % 7)).collect();
        let (p, c) = pass_time(&cont).unwrap();
        assert!((p - 177.0).abs() < 0.1 && (c - 3.0).abs() < 0.1, "{p} {c}");
        // an OUT some passes skip: twice and three times the pass, the pass itself rarer
        let mut ts = vec![348u32; 50];
        ts.extend([522; 30]);
        ts.extend([174; 25]);
        assert_eq!(pass_time(&iv(&ts)).unwrap().0, 174.0);
        // times that are no multiple of one pass
        let ts: Vec<u32> = (0..200).map(|i| 150 + (i * 37) % 200).collect();
        assert!(pass_time(&iv(&ts)).is_err());
        assert!(pass_time(&iv(&[174; 10])).is_err());
    }

    #[test]
    fn low_pitches_get_longer_windows() {
        assert_eq!(window_frames(2000.0), 4);
        assert_eq!(window_frames(430.0), 4);
        assert_eq!(window_frames(300.0), 6);
        assert_eq!(window_frames(200.0), 8);
        assert_eq!(window_frames(60.0), 8);
    }

    #[test]
    fn same_pitch_needs_most_shared_windows_to_agree() {
        let a: Vec<(u32, f32)> = (0..20).map(|w| (w, 440.0)).collect();
        let mut b = a.clone();
        assert!(same_pitch(&a, &b));
        for x in b.iter_mut().take(5) {
            x.1 = 600.0;
        }
        assert!(!same_pitch(&a, &b));
        assert!(!same_pitch(&a[..5], &b[..5]));
    }
}
