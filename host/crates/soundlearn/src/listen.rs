// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Listening: runs a game for a fixed time under a generic input schedule and records, for each
//! `OUT` that flips the speaker, the periods it makes and what the code read for them. The
//! recording is streamed and bounded: never the whole trace.
//!
//! Per site (the first byte of the `OUT` instruction):
//!
//! - a *flip* is an `OUT` to an even port that changes the speaker level (bit 4);
//! - a *period sample* is the time from the site's flip `k-2` to flip `k`, less the contention
//!   T-states between (the period the routine's code makes, not the ULA's delays); one for every
//!   other flip, so no two share a half period; a period an interrupt was accepted in is not a
//!   sample (the interrupt routine's time is not the music routine's);
//! - the samples span the whole run: at most 4,096 are kept per site, and when they fill up
//!   every other one is dropped and the stride between kept candidates doubles;
//! - a *snapshot* is taken at the site's next `OUT` after each flip that precedes a sampled
//!   one (`k-1`): the registers then, and every value read since the flip before it (`k-2`), by
//!   (reader pc, read index in that instruction), the last value for each key. A period sample
//!   points at that snapshot: the reads of its whole period `k-2` to `k`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use zx_sys::{zx_regs, Machine, TraceEv, ZX_FRAME_TSTATES, ZX_TR_INSN, ZX_TR_INTA, ZX_TR_OUT, ZX_TR_READ};

/// Frame to (Spectrum key rows, Kempston bits).
pub type Inputs = Box<dyn Fn(u32) -> ([u8; 8], u8) + Send + Sync>;

/// Frames to listen to a game: 3 minutes of emulated time.
pub const LISTEN_FRAMES: u32 = 9000;
/// Frames before the first start key.
const QUIET_FRAMES: u32 = 2000;
/// Per site: period samples (and their snapshots) kept; more are thinned evenly over the run.
const KEEP: usize = 4096;
/// Sites kept (the first ones heard).
const MAX_SITES: usize = 64;
/// Reads kept for the snapshots (the newest).
const READ_RING: usize = 2048;

/// The keys and Kempston directions the schedule presses: name as in the manifest, and the
/// position. Spectrum keys are (half-row, bit): the rows are the keyboard's, from the lowest
/// address line: CAPS-V, A-G, Q-T, 1-5, 0-6, P-Y, ENTER-H, SPACE-B, with the key nearest the
/// edge of the half-row on bit 0 (0 9 8 7 6, P O I U Y). Kempston is row 8, the bit mask being
/// right 1, left 2, down 4, up 8, fire 16.
pub const PRESSES: [(&str, usize, u8); 17] = [
    ("key.ENTER", 6, 1),
    ("key.SPACE", 7, 1),
    ("key.0", 4, 1),
    ("key.1", 3, 1),
    ("key.Q", 2, 1),
    ("key.A", 1, 1),
    ("key.O", 5, 2),
    ("key.P", 5, 1),
    ("key.5", 3, 16),
    ("key.6", 4, 16),
    ("key.7", 4, 8),
    ("key.8", 4, 4),
    ("kempston.up", 8, 8),
    ("kempston.down", 8, 4),
    ("kempston.left", 8, 2),
    ("kempston.right", 8, 1),
    ("kempston.fire", 8, 16),
];
/// Indexes into [`PRESSES`]: the start keys (ENTER, SPACE, 0, 1, Kempston fire), and the random
/// play's choices (Q, A, O, P, 5-8, SPACE, Kempston up, down, left, right, fire).
const START: [usize; 5] = [0, 1, 2, 3, 16];
const PLAY: [usize; 14] = [4, 5, 6, 7, 8, 9, 10, 11, 1, 12, 13, 14, 15, 16];

fn press(set: &mut ([u8; 8], u8), i: usize) {
    if let Some(&(_, row, mask)) = PRESSES.get(i) {
        match set.0.get_mut(row) {
            Some(r) => *r |= mask,
            None => set.1 |= mask,
        }
    }
}

/// The generic run, for frames `0..LISTEN_FRAMES` (nothing is held after them):
///
/// - frames 0-1999: nothing pressed;
/// - then ENTER, SPACE, 0, 1 and Kempston fire in turn, each held 5 frames and followed by 150
///   frames of nothing;
/// - then random play: every 5-25 frames (xorshift64 from `seed`) a random non-empty subset of
///   Q, A, O, P, 5-8, SPACE and the Kempston directions and fire, at most 3 at once.
///
/// The same seed gives the same inputs.
pub fn schedule(seed: u64) -> Inputs {
    let mut table = vec![([0u8; 8], 0u8); LISTEN_FRAMES as usize];
    let mut f = QUIET_FRAMES as usize;
    for &i in &START {
        let mut set = ([0u8; 8], 0u8);
        press(&mut set, i);
        for slot in table.iter_mut().skip(f).take(5) {
            *slot = set;
        }
        f += 155;
    }
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    if x == 0 {
        x = 1;
    }
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    while f < table.len() {
        let len = 5 + (next() % 21) as usize;
        let mut set = ([0u8; 8], 0u8);
        let mut chosen = [usize::MAX; 3];
        for c in chosen.iter_mut().take(1 + (next() % 3) as usize) {
            *c = PLAY[(next() % PLAY.len() as u64) as usize];
        }
        for &c in chosen.iter().filter(|&&c| c != usize::MAX) {
            press(&mut set, c);
        }
        for slot in table.iter_mut().skip(f).take(len) {
            *slot = set;
        }
        f += len;
    }
    Box::new(move |frame| table.get(frame as usize).copied().unwrap_or(([0; 8], 0)))
}

/// Frame at which a start key is pressed in the single-key runs.
const KEY_FRAME: u32 = 500;
/// Speaker flips a frame needs to count as music.
const MUSIC_EDGES: u32 = 3;
/// A single-key or quiet run replaces the generic one only at this many times its music frames
/// (as a ratio, numerator over denominator) ...
const WIN_RATIO: (u32, u32) = (3, 2);
/// ... and at this many more music frames.
const WIN_MARGIN: u32 = 500;

/// An input run the learner can listen under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    /// The generic [`schedule`].
    Generic,
    /// Nothing pressed.
    Nothing,
    /// The start key (an index into [`START`]) held 5 frames from frame 500, then nothing.
    Once(usize),
    /// A key of [`KEYS`] (its index) held 5 frames from frame 500, then nothing: the recorded
    /// scores' extra runs ([`key_runs`]).
    Key(usize),
}

/// The keys of the recorded scores' extra runs ([`RunKind::Key`]): 1-9, 0, ENTER and SPACE, as
/// (name, half-row, bit) like [`PRESSES`].
pub const KEYS: [(&str, usize, u8); 12] = [
    ("1", 3, 1),
    ("2", 3, 2),
    ("3", 3, 4),
    ("4", 3, 8),
    ("5", 3, 16),
    ("6", 4, 16),
    ("7", 4, 8),
    ("8", 4, 4),
    ("9", 4, 2),
    ("0", 4, 1),
    ("ENTER", 6, 1),
    ("SPACE", 7, 1),
];

impl RunKind {
    /// The run's name for the report: `ENTER once, then nothing pressed`.
    pub fn describe(self) -> String {
        match self {
            RunKind::Generic => "the generic run".into(),
            RunKind::Nothing => "nothing pressed".into(),
            RunKind::Once(i) => {
                let name = START.get(i).and_then(|&p| PRESSES.get(p)).map_or("", |p| p.0);
                let name = name.strip_prefix("key.").unwrap_or(name);
                let name = if name == "kempston.fire" { "Kempston fire" } else { name };
                format!("{name} once, then nothing pressed")
            }
            RunKind::Key(i) => format!("{} once, then nothing pressed", KEYS.get(i).map_or("", |k| k.0)),
        }
    }

    /// Its short name in a recorded score's line: `key 3 once`, `ENTER once`.
    pub fn short(self) -> String {
        let d = self.describe();
        let once = d.strip_suffix(", then nothing pressed").unwrap_or(&d);
        if once.starts_with(|c: char| c.is_ascii_digit()) { format!("key {once}") } else { once.to_string() }
    }

    /// The key rows and Kempston bits it presses at frame 500 (none for the generic run and
    /// nothing pressed).
    fn press(self) -> Option<([u8; 8], u8)> {
        let mut set = ([0u8; 8], 0u8);
        match self {
            RunKind::Generic | RunKind::Nothing => return None,
            RunKind::Once(i) => press(&mut set, START.get(i).copied().unwrap_or(usize::MAX)),
            RunKind::Key(i) => {
                let &(_, row, mask) = KEYS.get(i)?;
                set.0[row] |= mask;
            }
        }
        Some(set)
    }
}

/// The recorded scores' extra runs: each key of [`KEYS`] held 5 frames at frame 500, then
/// nothing, but the one the learner's run `learner` presses so.
pub fn key_runs(learner: Option<RunKind>) -> Vec<RunKind> {
    let theirs = learner.and_then(RunKind::press);
    (0..KEYS.len()).map(RunKind::Key).filter(|k| k.press() != theirs).collect()
}

/// The candidate runs of [`choose_run`] in order of preference: the generic run, nothing pressed,
/// then each start key once.
pub fn candidates() -> Vec<RunKind> {
    let mut v = vec![RunKind::Generic, RunKind::Nothing];
    v.extend((0..START.len()).map(RunKind::Once));
    v
}

/// The inputs of a candidate run.
pub fn run_inputs(kind: RunKind, seed: u64) -> Inputs {
    match kind {
        RunKind::Generic => schedule(seed),
        RunKind::Nothing => Box::new(|_| ([0; 8], 0)),
        RunKind::Once(_) | RunKind::Key(_) => {
            let set = kind.press().unwrap_or_default();
            Box::new(move |f| if (KEY_FRAME..KEY_FRAME + 5).contains(&f) { set } else { ([0; 8], 0) })
        }
    }
}

/// The winner among candidate `scores` (music frames, the generic run's first): the generic run,
/// unless another has at least 1.5 times its music frames and 500 more; of those the highest,
/// the earlier on a tie. Returns the index into `scores`.
pub fn pick(scores: &[u32]) -> usize {
    let Some(&generic) = scores.first() else { return 0 };
    let (mut best, mut best_score) = (0, 0);
    for (i, &s) in scores.iter().enumerate().skip(1) {
        let wins = u64::from(s) * u64::from(WIN_RATIO.1) >= u64::from(generic) * u64::from(WIN_RATIO.0)
            && s >= generic.saturating_add(WIN_MARGIN);
        if wins && s > best_score {
            (best, best_score) = (i, s);
        }
    }
    best
}

/// Frames of `inputs` with at least 3 changes of the speaker (EAR) level, over [`LISTEN_FRAMES`]
/// frames of a freshly loaded machine, with no trace hook.
pub fn music_frames(m: &mut Machine, inputs: &Inputs) -> u32 {
    let (mut ear, mut count) = (m.edges_start_level() & 2, 0);
    for f in 0..LISTEN_FRAMES {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let mut flips = 0;
        for e in m.edges() {
            let now = e.level & 2;
            flips += u32::from(now != ear);
            ear = now;
        }
        count += u32::from(flips >= MUSIC_EDGES);
    }
    count
}

/// The run that hears the most music, for a game with no key script: the generic run, nothing
/// pressed, and each start key (ENTER, SPACE, 0, 1, Kempston fire) held 5 frames at frame 500 and
/// then nothing, each scored by [`music_frames`] and chosen by [`pick`]. `load` gives a freshly
/// loaded machine. Returns the run, its kind, its music frames and the generic run's.
pub fn choose_run(load: &dyn Fn() -> Machine, seed: u64) -> (Inputs, RunKind, u32, u32) {
    let kinds = candidates();
    let scores: Vec<u32> = kinds.iter().map(|&k| music_frames(&mut load(), &run_inputs(k, seed))).collect();
    let at = pick(&scores);
    let kind = kinds.get(at).copied().unwrap_or(RunKind::Generic);
    let (won, generic) = (scores.get(at).copied().unwrap_or(0), scores.first().copied().unwrap_or(0));
    (run_inputs(kind, seed), kind, won, generic)
}

/// What a run of a game sounds like.
#[derive(Clone, Debug, Default)]
pub struct Heard {
    pub sites: Vec<SiteHeard>,
    pub frames: u32,
    /// Frames with a speaker flip from any site.
    pub sound_frames: u32,
}

/// One speaker `OUT` site's flips.
#[derive(Clone, Debug)]
pub struct SiteHeard {
    pub pc: u16,
    /// Flips of the speaker level.
    pub edges: u32,
    pub frames_with_edges: u32,
    /// At most 4,096, evenly spread over the run.
    pub periods: Vec<PeriodSample>,
    /// One for each period sample, in the same order.
    pub snapshots: Vec<Snapshot>,
}

/// One full period of a site: the time between its flips `k-2` and `k`, less contention.
#[derive(Clone, Copy, Debug)]
pub struct PeriodSample {
    pub period_t: f64,
    /// The contention taken out (the period played was `period_t + contention_t`).
    pub contention_t: f64,
    /// The frame of flip `k`.
    pub at_frame: u32,
    /// Index into the site's snapshots: the reads and registers for this period.
    pub snap: u32,
}

/// The registers at a site's `OUT` and the values read before it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub regs: zx_regs,
    /// (reader pc, read index in that instruction, address, value), by reader and index.
    pub reads: Vec<(u16, u8, u16, u8)>,
}

struct Flip {
    t: u64,
    cont: u32,
    seq: u64,
}

struct Site {
    pc: u16,
    edges: u32,
    frames_with_edges: u32,
    last_edge_frame: Option<u32>,
    /// The site's last two flips, oldest first.
    flips: VecDeque<Flip>,
    pending: bool,
    /// The snapshot taken after the site's latest flip but one, for the coming sample.
    held: Option<Snapshot>,
    /// Sample candidates seen, and the stride between those kept.
    candidates: u64,
    stride: u64,
    /// Kept samples (period, contention) with their snapshots.
    periods: Vec<(f64, f64, u32, Snapshot)>,
}

#[derive(Default)]
struct Rec {
    /// Samples kept per site before thinning.
    keep: usize,
    frame: u32,
    level: bool,
    last_sound_frame: Option<u32>,
    sound_frames: u32,
    insn_reads: u8,
    seq: u64,
    /// When the last interrupt was accepted (T-states since the start), if one was.
    last_int: Option<u64>,
    reads: VecDeque<(u64, u16, u8, u16, u8)>,
    sites: Vec<Site>,
}

/// The registers at the `OUT` and the last value read at each (reader pc, read index) since
/// read number `from`.
fn snapshot(log: &VecDeque<(u64, u16, u8, u16, u8)>, from: u64, ev: &TraceEv) -> Snapshot {
    let mut reads: Vec<(u16, u8, u16, u8)> = Vec::new();
    for &(_, pc, idx, addr, val) in log.iter().rev().take_while(|r| r.0 >= from) {
        // newest first: the first of each (pc, index) is its last value
        if !reads.iter().any(|r| r.0 == pc && r.1 == idx) {
            reads.push((pc, idx, addr, val));
        }
    }
    reads.sort_unstable_by_key(|r| (r.0, r.1));
    Snapshot { regs: ev.regs, reads }
}

impl Rec {
    fn event(&mut self, ev: &TraceEv) {
        match u32::from(ev.kind) {
            ZX_TR_INSN => self.insn_reads = 0,
            ZX_TR_READ => {
                if self.reads.len() >= READ_RING {
                    self.reads.pop_front();
                }
                self.reads.push_back((self.seq, ev.pc, self.insn_reads, ev.addr, ev.val));
                self.seq = self.seq.wrapping_add(1);
                self.insn_reads = self.insn_reads.saturating_add(1);
            }
            ZX_TR_OUT if ev.addr & 1 == 0 => self.out(ev),
            ZX_TR_INTA => self.last_int = Some(u64::from(self.frame) * u64::from(ZX_FRAME_TSTATES) + u64::from(ev.t)),
            _ => {}
        }
    }

    fn out(&mut self, ev: &TraceEv) {
        let level = ev.val & 0x10 != 0;
        let flip = level != self.level;
        self.level = level;
        let at = match self.sites.iter().position(|s| s.pc == ev.pc) {
            Some(i) => i,
            None if flip && self.sites.len() < MAX_SITES => {
                self.sites.push(Site {
                    pc: ev.pc,
                    edges: 0,
                    frames_with_edges: 0,
                    last_edge_frame: None,
                    flips: VecDeque::new(),
                    pending: false,
                    held: None,
                    candidates: 0,
                    stride: 1,
                    periods: Vec::new(),
                });
                self.sites.len() - 1
            }
            None => return,
        };
        let (frame, seq, last_int) = (self.frame, self.seq, self.last_int);
        let t = u64::from(frame) * u64::from(ZX_FRAME_TSTATES) + u64::from(ev.t);
        let Some(site) = self.sites.get_mut(at) else { return };
        if site.pending {
            site.pending = false;
            // only for a sample that will be kept
            if site.candidates % site.stride.max(1) == 0 {
                site.held = Some(snapshot(&self.reads, site.flips.front().map_or(seq, |f| f.seq), ev));
            }
        }
        if !flip {
            return;
        }
        site.edges = site.edges.saturating_add(1);
        if site.last_edge_frame != Some(frame) {
            site.last_edge_frame = Some(frame);
            site.frames_with_edges += 1;
        }
        if self.last_sound_frame != Some(frame) {
            self.last_sound_frame = Some(frame);
            self.sound_frames += 1;
        }
        // periods are taken from every other flip: the samples are independent (not two to a
        // period, sharing their halves)
        let quiet = site.flips.front().is_some_and(|old| last_int.is_none_or(|i| i < old.t));
        if site.edges % 2 == 1 && site.flips.len() == 2 && !quiet {
            site.held = None;
        }
        if site.edges % 2 == 1 && site.flips.len() == 2 && quiet {
            let keep = site.candidates % site.stride.max(1) == 0;
            site.candidates = site.candidates.wrapping_add(1);
            if let (true, Some(old), Some(snap)) = (keep, site.flips.front(), site.held.take()) {
                let cont = ev.cont.wrapping_sub(old.cont);
                let p = t.wrapping_sub(old.t).wrapping_sub(u64::from(cont));
                site.periods.push((p as f64, f64::from(cont), frame, snap));
                if site.periods.len() >= self.keep.max(2) {
                    // thin to every other sample, the stride doubling: the kept ones always span the run
                    let mut i = 0;
                    site.periods.retain(|_| {
                        i += 1;
                        i % 2 == 1
                    });
                    site.stride = site.stride.saturating_mul(2);
                }
            }
        }
        if site.flips.len() == 2 {
            site.flips.pop_front();
        }
        site.flips.push_back(Flip { t, cont: ev.cont, seq });
        site.pending = site.edges % 2 == 0;
    }

    fn finish(&mut self, frames: u32) -> Heard {
        let sites = self
            .sites
            .drain(..)
            .map(|s| {
                let mut periods = Vec::with_capacity(s.periods.len());
                let mut snapshots = Vec::with_capacity(s.periods.len());
                for (period_t, contention_t, at_frame, snap) in s.periods {
                    periods.push(PeriodSample { period_t, contention_t, at_frame, snap: snapshots.len() as u32 });
                    snapshots.push(snap);
                }
                SiteHeard { pc: s.pc, edges: s.edges, frames_with_edges: s.frames_with_edges, periods, snapshots }
            })
            .collect();
        Heard { sites, frames, sound_frames: self.sound_frames }
    }
}

/// Runs `m` for `frames` frames under `inputs` (each frame: its key rows and Kempston bits are
/// set, then the frame runs) and records the speaker sites. The machine keeps no trace hook
/// afterwards.
pub fn listen(m: &mut Machine, inputs: &Inputs, frames: u32) -> Heard {
    listen_with(m, inputs, frames, KEEP)
}

/// [`listen`] keeping at most `keep` period samples (and snapshots) per site, 4,096 for `listen`.
pub fn listen_with(m: &mut Machine, inputs: &Inputs, frames: u32, keep: usize) -> Heard {
    record(m, inputs, frames, keep, &mut |_| {})
}

/// [`listen`], `observe` given the machine after each frame (the learner's built-in table decides
/// the frames alongside: [`crate::roles::BuiltinRun`]).
pub fn listen_observed(m: &mut Machine, inputs: &Inputs, frames: u32, observe: &mut dyn FnMut(&Machine)) -> Heard {
    record(m, inputs, frames, KEEP, observe)
}

fn record(m: &mut Machine, inputs: &Inputs, frames: u32, keep: usize, observe: &mut dyn FnMut(&Machine)) -> Heard {
    let rec = Rc::new(RefCell::new(Rec { keep, ..Rec::default() }));
    let frame = Rc::new(Cell::new(0u32));
    let (hook_rec, hook_frame) = (Rc::clone(&rec), Rc::clone(&frame));
    // the hook must never panic: a panic inside the C callback aborts the process
    m.set_trace(
        Some(Box::new(move |ev: &TraceEv| {
            if let Ok(mut r) = hook_rec.try_borrow_mut() {
                r.frame = hook_frame.get();
                r.event(ev);
            }
        })),
        (1 << ZX_TR_INSN) | (1 << ZX_TR_READ) | (1 << ZX_TR_OUT) | (1 << ZX_TR_INTA),
    );
    for f in 0..frames {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        frame.set(f);
        m.run_frame();
        observe(m);
    }
    m.set_trace(None, 0);
    let heard = rec.borrow_mut().finish(frames);
    heard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_keeps_the_generic_run_unless_far_ahead() {
        assert_eq!(pick(&[]), 0);
        assert_eq!(pick(&[100, 0, 0, 0, 0, 0, 0]), 0);
        // 1.5 times but under 500 more
        assert_eq!(pick(&[300, 450, 0, 0, 0, 0, 0]), 0);
        // 500 more but under 1.5 times
        assert_eq!(pick(&[4000, 4500, 0, 0, 0, 0, 0]), 0);
        assert_eq!(pick(&[1000, 1500, 0, 0, 0, 0, 0]), 1);
        assert_eq!(pick(&[0, 0, 500, 0, 0, 0, 0]), 2);
        assert_eq!(pick(&[0, 0, 499, 0, 0, 0, 0]), 0);
    }

    #[test]
    fn pick_takes_the_highest_and_the_earlier_on_a_tie() {
        assert_eq!(pick(&[100, 900, 2000, 1500, 2000, 0, 0]), 2);
        assert_eq!(pick(&[100, 0, 0, 0, 900, 0, 900]), 4);
    }

    #[test]
    fn a_tune_that_plays_unprompted_keeps_the_generic_run() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna");
        let load = || {
            let mut m = Machine::new();
            m.load_sna(&std::fs::read(path).expect("tune.sna")).expect("loads");
            m
        };
        assert!(music_frames(&mut load(), &run_inputs(RunKind::Nothing, 1)) > 1000);
        let (_, kind, won, generic) = choose_run(&load, 1);
        assert_eq!(kind, RunKind::Generic);
        assert_eq!(won, generic);
    }

    #[test]
    fn runs_press_what_they_say() {
        let once = run_inputs(RunKind::Once(1), 1);
        assert_eq!(once(KEY_FRAME - 1), ([0; 8], 0));
        assert_eq!(once(KEY_FRAME).0[7], 1);
        assert_eq!(once(KEY_FRAME + 4).0[7], 1);
        assert_eq!(once(KEY_FRAME + 5), ([0; 8], 0));
        assert_eq!(run_inputs(RunKind::Once(4), 1)(KEY_FRAME).1, 16);
        assert_eq!(run_inputs(RunKind::Nothing, 1)(KEY_FRAME), ([0; 8], 0));
        assert_eq!(RunKind::Once(0).describe(), "ENTER once, then nothing pressed");
        assert_eq!(RunKind::Once(4).describe(), "Kempston fire once, then nothing pressed");
        // the extra runs: twelve keys, less the learner's
        assert_eq!(run_inputs(RunKind::Key(2), 1)(KEY_FRAME).0[3], 4);
        assert_eq!(run_inputs(RunKind::Key(10), 1)(KEY_FRAME + 4).0[6], 1);
        assert_eq!((RunKind::Key(2).short(), RunKind::Key(11).short(), RunKind::Once(3).short()), ("key 3 once".into(), "SPACE once".into(), "key 1 once".into()));
        assert_eq!(key_runs(None).len(), 12);
        assert_eq!(key_runs(Some(RunKind::Generic)).len(), 12);
        assert_eq!(key_runs(Some(RunKind::Once(4))).len(), 12, "Kempston fire is no key of theirs");
        let runs = key_runs(Some(RunKind::Once(3)));
        assert_eq!(runs.len(), 11);
        assert!(!runs.contains(&RunKind::Key(0)), "key 1 is the learner's");
    }
}
