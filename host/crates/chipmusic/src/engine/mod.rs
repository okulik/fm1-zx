// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A game's beeper music read exactly from its music engine (`fmzx-pack engine-notes`).
//!
//! A known engine is recognised in the game's memory by its code ([`Engine::locate`]: byte
//! patterns with wildcards, [`pattern`] and [`find`], so that an engine moved to another address
//! or with its operands patched by the game still matches). The game then runs in the Spectrum
//! machine core under the run's inputs with the trace hook on ([`read_run`]): at the engine's own
//! row-read instruction the tap reads each channel's state from the registers and from memory (a
//! copy kept current by the run's writes), times the engine's sound loop from the spacing of an
//! `OUT` it makes once a pass, and takes each drum from the `OUT`s of the engine's drum code. The
//! rows become notes per channel ([`Engine::notes`]; by default a note runs while its channel's
//! value stays the same, [`split_notes`]), each with its start and end in T since the run's start,
//! its period (from the channel's value and the pass time), level and duty; the drum hits get a
//! kind (`kick`, `snare`, `hat`, `tom`, `tone` or `noise`).
//!
//! # Adding an engine
//!
//! One file in this folder holds an engine: a unit struct implementing [`Engine`] with its
//! signature ([`Engine::locate`]: where its row read, its pass `OUT` and its drum `OUT`s are,
//! found by their code), its tap ([`Engine::read`]: a [`Row`] from the registers and memory at the
//! row read), its period rule ([`Engine::period_t`]) and, where the default does not fit, its
//! pass time ([`Engine::pass_t`]) and note rules ([`Engine::notes`]). Add it to [`ENGINES`] and
//! give it a fixture test in `tests/engines.rs`.

pub mod ntropic;
pub mod stocker;
pub mod tritone;

use std::cell::RefCell;
use std::rc::Rc;
use zx_sys::{zx_regs, Machine, TraceEv, ZX_FRAME_TSTATES, ZX_TR_INSN, ZX_TR_OUT, ZX_TR_WRITE};

/// The machine's memory as the tap sees it.
pub type Mem = [u8; 0x10000];

/// A run's inputs per frame: the keyboard rows and the Kempston bits.
pub type InputsFn<'a> = &'a dyn Fn(u32) -> ([u8; 8], u8);

/// The T-states of a frame.
const FT: u64 = ZX_FRAME_TSTATES as u64;
/// Frames between two looks for a known engine while a run is searched.
const LOOK_EVERY: usize = 25;
/// A row lasting longer than this many times the median row is cut there (the engine stopped).
const ROW_SPAN_MAX: u64 = 8;
/// Notes shorter than this (T) are left out.
const NOTE_MIN_T: u64 = 64;

/// The engines known, in the order they are looked for.
pub static ENGINES: &[&dyn Engine] = &[&ntropic::Ntropic, &tritone::Tritone, &stocker::Stocker];

/// Where an engine lies in memory: found by [`Engine::locate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sites {
    /// Where its signature matched.
    pub anchor: u16,
    /// The instruction the tap reads a row (or a tick) at.
    pub row_pc: u16,
    /// Bytes that must be there when it runs (the engine's code, not another program's).
    pub check: Vec<(u16, u8)>,
    /// An `OUT` the sound loop makes once a pass: its spacing within a row is the pass time.
    pub pass_out: Option<u16>,
    /// The `OUT`s of its drum code and the drum each makes.
    pub drums: Vec<(u16, &'static str)>,
    /// Engine-specific addresses (operands read at each row).
    pub ops: Vec<u16>,
}

impl Sites {
    /// Whether `pc` lies within `span` bytes of the engine's code (its anchor, row read and
    /// `OUT`s).
    pub fn near(&self, pc: u16, span: u16) -> bool {
        let close = |a: u16| pc.abs_diff(a) <= span;
        close(self.anchor) || close(self.row_pc) || self.pass_out.is_some_and(close) || self.drums.iter().any(|&(a, _)| close(a))
    }
}

/// A channel's state at a row: its value (0: silent; a counter or a step, as its engine has it),
/// its duty (0-255 of a period high; 128 a square wave) and its volume (the engine's own steps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chan {
    pub value: u32,
    pub duty: u8,
    pub volume: u8,
}

/// What the tap read at one row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// T-states since the run's start.
    pub t: u64,
    pub ch: Vec<Chan>,
    /// A noise burst starting at the row (ntropic): its length in passes.
    pub noise: Option<u32>,
    /// The passes of the sound loop the row plays, when the engine says.
    pub passes: u32,
    /// The pass time measured in the row (the [`Sites::pass_out`] `OUT`s' spacing), T.
    pub pass_t: Option<f64>,
}

/// A drum hit: the first and the last `OUT` of its burst (T since the run's start) and its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub t: u64,
    pub end_t: u64,
    pub kind: &'static str,
}

/// A note of a channel, before it is put on a timeline: T since the run's start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineNote {
    pub channel: u8,
    pub start_t: u64,
    pub end_t: u64,
    /// The channel's value (counter or step) at its start.
    pub value: u32,
    /// The full period in T (3.5 MHz).
    pub period_t: f64,
    /// 1-255.
    pub level: u8,
    pub duty: u8,
}

impl EngineNote {
    /// Its pitch as a MIDI note number (A4 = 69 at 440 Hz).
    pub fn midi(&self) -> f64 {
        69.0 + 12.0 * (3_500_000.0 / self.period_t / 440.0).log2()
    }
}

/// A known music engine (see the module docs, "Adding an engine").
pub trait Engine: Sync {
    /// Its name: `ntropic`, `Tritone`, `Stocker`.
    fn name(&self) -> &'static str;
    /// Its tone channels.
    fn channels(&self) -> usize;
    /// Where it lies in `mem`, if it is there.
    fn locate(&self, mem: &Mem) -> Option<Sites>;
    /// The row the tap reads at [`Sites::row_pc`] (`t` and `pass_t` left for the tap); None
    /// when the instruction does not read one (the song's end marker).
    fn read(&self, sites: &Sites, regs: &zx_regs, mem: &Mem) -> Option<Row>;
    /// A channel value's full period in T, given the pass time.
    fn period_t(&self, value: u32, pass_t: f64) -> f64;
    /// A note's level (1-255) from its channel's state.
    fn level(&self, c: &Chan) -> u8 {
        let _ = c;
        255
    }
    /// The pass time of the run from its rows: the median of the rows' measured pass times.
    fn pass_t(&self, rows: &[Row]) -> Option<f64> {
        median(rows.iter().filter_map(|r| r.pass_t).collect())
    }
    /// The notes of the rows, per channel ([`split_notes`] by default), and the drum hits the
    /// rows themselves hold (ntropic's noise), given the rows' ends.
    fn notes(&self, rows: &[Row], ends: &[u64], pass_t: f64) -> (Vec<EngineNote>, Vec<Hit>) {
        (split_notes(self, rows, ends, pass_t), Vec::new())
    }
    /// The longest gap (T) between two `OUT`s of one drum hit.
    fn drum_gap_t(&self) -> u64 {
        4000
    }
    /// What the tap reads, in words, for the take's source line.
    fn what(&self, sites: &Sites) -> String;
}

/// A signature as bytes: hex pairs, `??` for any byte.
pub fn pattern(s: &str) -> Vec<Option<u8>> {
    s.split_whitespace().map(|t| if t == "??" { None } else { Some(u8::from_str_radix(t, 16).expect("a hex byte")) }).collect()
}

/// The first address from `from` (to the end of memory) where `pat` matches.
pub fn find(mem: &Mem, pat: &[Option<u8>], from: usize) -> Option<u16> {
    let last = mem.len().checked_sub(pat.len())?;
    (from..=last).find(|&a| pat.iter().enumerate().all(|(i, p)| p.is_none_or(|b| mem[a + i] == b))).map(|a| a as u16)
}

/// [`find`] of the pattern text `s` in RAM (from 4000h).
pub fn find_ram(mem: &Mem, s: &str) -> Option<u16> {
    find(mem, &pattern(s), 0x4000)
}

/// [`find`] of the pattern text `s` within `span` bytes of `near` (the drum code beside an
/// engine; the first match after `near - span`).
pub fn find_near(mem: &Mem, s: &str, near: u16, span: u16) -> Option<u16> {
    let pat = pattern(s);
    let from = usize::from(near.saturating_sub(span)).max(0x4000);
    find(mem, &pat, from).filter(|&a| a.abs_diff(near) <= span)
}

/// The byte and the little-endian word at an address.
pub fn byte(mem: &Mem, a: u16) -> u8 {
    mem[usize::from(a)]
}
pub fn word(mem: &Mem, a: u16) -> u16 {
    u16::from(byte(mem, a)) | u16::from(byte(mem, a.wrapping_add(1))) << 8
}

/// The median of `v`; None when empty.
pub fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// The default note rule: per channel, a note while its value stays the same (0 is silence),
/// a new one when it changes; its level and duty from its first row.
pub fn split_notes<E: Engine + ?Sized>(e: &E, rows: &[Row], ends: &[u64], pass_t: f64) -> Vec<EngineNote> {
    let mut out = Vec::new();
    for c in 0..e.channels() {
        let mut cur: Option<EngineNote> = None;
        for (r, &end) in rows.iter().zip(ends) {
            let ch = r.ch.get(c).copied().unwrap_or_default();
            if let Some(n) = cur.as_mut() {
                if n.value == ch.value && n.end_t == r.t {
                    n.end_t = end;
                    continue;
                }
                out.push(*n);
                cur = None;
            }
            if ch.value != 0 {
                cur = Some(EngineNote {
                    channel: c as u8,
                    start_t: r.t,
                    end_t: end,
                    value: ch.value,
                    period_t: e.period_t(ch.value, pass_t),
                    level: e.level(&ch),
                    duty: ch.duty,
                });
            }
        }
        out.extend(cur);
    }
    out.retain(|n| n.end_t >= n.start_t + NOTE_MIN_T);
    out.sort_by_key(|n| (n.start_t, n.channel));
    out
}

/// The rows' ends: each the next row's start, a row at most [`ROW_SPAN_MAX`] medians long (the
/// last one median long).
pub fn row_ends(rows: &[Row]) -> Vec<u64> {
    let spans: Vec<f64> = rows.windows(2).map(|w| (w[1].t - w[0].t) as f64).collect();
    let med = median(spans).unwrap_or(FT as f64).max(1.0) as u64;
    rows.iter().enumerate().map(|(i, r)| rows.get(i + 1).map_or(r.t + med, |n| n.t.min(r.t + ROW_SPAN_MAX * med))).collect()
}

/// A run's memory, read from the machine.
pub fn memory(m: &Machine) -> Box<Mem> {
    let mut mem = Box::new([0u8; 0x10000]);
    for (a, b) in mem.iter_mut().enumerate() {
        *b = m.peek(a as u16);
    }
    mem
}

/// The first known engine in `mem`, and where it lies.
pub fn identify(mem: &Mem) -> Option<(&'static dyn Engine, Sites)> {
    identify_all(mem).into_iter().next()
}

/// Every known engine in `mem`, and where each lies, in the order of [`ENGINES`].
pub fn identify_all(mem: &Mem) -> Vec<(&'static dyn Engine, Sites)> {
    ENGINES.iter().filter_map(|&e| e.locate(mem).map(|s| (e, s))).collect()
}

/// The first known engine `want` accepts in a run's memory within `frames` frames (looked for
/// every few frames): the engine, where it lies and the frame it was found after. Err: the first
/// engine found that `want` does not accept, if any (a game may hold an engine that does not play
/// the music asked for).
#[allow(clippy::type_complexity)]
pub fn locate_in_run(
    load: &dyn Fn() -> Machine,
    inputs: InputsFn,
    frames: usize,
    want: &dyn Fn(&Sites) -> bool,
) -> Result<(&'static dyn Engine, Sites, usize), Option<(&'static dyn Engine, Sites)>> {
    let mut m = load();
    let mut other = None;
    for k in 0..frames {
        let (rows, kemp) = inputs(k as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        if k % LOOK_EVERY == LOOK_EVERY - 1 || k + 1 == frames {
            for (e, s) in identify_all(&memory(&m)) {
                if want(&s) {
                    return Ok((e, s, k));
                }
                other.get_or_insert((e, s));
            }
        }
    }
    Err(other)
}

/// What was read from a run.
#[derive(Clone, Debug)]
pub struct Reading {
    pub engine: &'static str,
    pub channels: usize,
    pub sites: Sites,
    /// What was read, in words.
    pub what: String,
    /// The sound loop's pass time, T.
    pub pass_t: f64,
    pub rows: Vec<Row>,
    /// Per channel, by start then channel.
    pub notes: Vec<EngineNote>,
    /// By start.
    pub drums: Vec<Hit>,
}

/// The tap's state while a run plays.
struct Tap {
    engine: &'static dyn Engine,
    sites: Sites,
    mem: Box<Mem>,
    frame: u64,
    rows: Vec<Row>,
    /// The pass `OUT`s of the current row: count, first and last T.
    outs: (u32, u64, u64),
    hits: Vec<Hit>,
    /// The last drum `OUT`'s T.
    last_drum: Option<u64>,
}

impl Tap {
    fn close_row(&mut self) {
        let (n, a, b) = self.outs;
        if let Some(r) = self.rows.last_mut() {
            if n > 1 && b > a {
                r.pass_t = Some((b - a) as f64 / f64::from(n - 1));
            }
        }
        self.outs = (0, 0, 0);
    }

    fn event(&mut self, ev: &TraceEv) {
        let t = self.frame * FT + u64::from(ev.t);
        if ev.kind == ZX_TR_WRITE as u8 {
            self.mem[usize::from(ev.addr)] = ev.val;
        } else if ev.kind == ZX_TR_INSN as u8 {
            if ev.pc != self.sites.row_pc || !self.sites.check.iter().all(|&(a, b)| byte(&self.mem, a) == b) {
                return;
            }
            self.close_row();
            if let Some(mut r) = self.engine.read(&self.sites, &ev.regs, &self.mem) {
                r.t = t;
                self.rows.push(r);
            }
        } else if ev.kind == ZX_TR_OUT as u8 && ev.addr & 1 == 0 {
            if Some(ev.pc) == self.sites.pass_out {
                let o = &mut self.outs;
                if o.0 == 0 {
                    o.1 = t;
                }
                o.0 += 1;
                o.2 = t;
            } else if let Some(&(_, kind)) = self.sites.drums.iter().find(|&&(a, _)| a == ev.pc) {
                match self.hits.last_mut() {
                    Some(h) if self.last_drum.is_some_and(|l| t <= l + self.engine.drum_gap_t()) => h.end_t = t,
                    _ => self.hits.push(Hit { t, end_t: t, kind }),
                }
                self.last_drum = Some(t);
            }
        }
    }
}

/// Reads `engine` (at `sites`) over a run: a machine from `load` under `inputs` for `frames`
/// frames, the tap on from frame `from` (its memory copied from the machine before each frame:
/// a tape's loading does not go through the write events). `on_frame` sees each frame (its
/// number and the machine) after it ran.
pub fn read_run(
    load: &dyn Fn() -> Machine,
    inputs: InputsFn,
    frames: usize,
    engine: &'static dyn Engine,
    sites: &Sites,
    from: usize,
    on_frame: &mut dyn FnMut(usize, &Machine),
) -> Reading {
    let tap = Rc::new(RefCell::new(Tap {
        engine,
        sites: sites.clone(),
        mem: Box::new([0u8; 0x10000]),
        frame: 0,
        rows: Vec::new(),
        outs: (0, 0, 0),
        hits: Vec::new(),
        last_drum: None,
    }));
    let mut m = load();
    for k in 0..frames {
        if k == from {
            let t2 = tap.clone();
            m.set_trace(Some(Box::new(move |ev: &TraceEv| t2.borrow_mut().event(ev))), (1 << ZX_TR_INSN) | (1 << ZX_TR_WRITE) | (1 << ZX_TR_OUT));
        }
        if k >= from {
            let mem = memory(&m);
            let mut t = tap.borrow_mut();
            t.mem = mem;
            t.frame = k as u64;
        }
        let (rows, kemp) = inputs(k as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        on_frame(k, &m);
    }
    m.set_trace(None, 0);
    let mut tap = tap.borrow_mut();
    tap.close_row();
    let rows = std::mem::take(&mut tap.rows);
    let pass_t = engine.pass_t(&rows).unwrap_or(0.0);
    let ends = row_ends(&rows);
    let (notes, mut drums) = if pass_t > 0.0 { engine.notes(&rows, &ends, pass_t) } else { (Vec::new(), Vec::new()) };
    drums.extend(tap.hits.iter().copied());
    drums.sort_by_key(|h| h.t);
    Reading { engine: engine.name(), channels: engine.channels(), what: engine.what(sites), sites: sites.clone(), pass_t, rows, notes, drums }
}
