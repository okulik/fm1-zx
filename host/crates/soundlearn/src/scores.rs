// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recorded scores: the music of a game's routines that nothing else plays, recorded from the
//! learner's run as kind-2 records ([`ScoreRecord`]) that method B plays in step with the beeper.
//!
//! 1. **Which music** (chosen by the learner, [`crate::learn`]): each heard routine (the
//!    learner's block, from its speaker `OUT` heard in the most frames) that sounded in at least
//!    [`SCORE_FRAMES`] frames, that no kept learned entry gives and no built-in entry matches,
//!    and whose `OUT` lies in no routine before it.
//! 2. **The stretch.** A *trigger* is the first speaker edge from the routine's `OUT` after at
//!    least [`TRIGGER_QUIET`] frames with none from it (or with none since the run's start). The
//!    stretch runs from it while the inputs stay as they were in its frame (a key press may
//!    change the music: it ends with the frame before they change), until the `OUT` has been
//!    silent for [`END_SILENCE`] frames (it ends with the bucket of the `OUT`'s last edge),
//!    [`SND_SCORE_BUCKETS_MAX`] buckets, or the end of the frames method B decides in the run.
//!    Of the routine's stretches the longest is taken; one under [`SCORE_FRAMES`] frames is not
//!    recorded, nor one more than a tenth of whose sounding frames method B's table (the
//!    built-in and kept entries) claims: that music is played already. Times are T-states from
//!    the trigger edge (T0); a bucket is a frame long from T0, and a stretch is whole buckets.
//! 3. **The record.** Its fingerprint is the speaker edges per bucket ([`score_fingerprint`]),
//!    its code the hash of the bytes around the `OUT` at the trigger's frame ([`score_code`]),
//!    its `quiet` the frames with no edge from the `OUT` before the trigger, at most
//!    [`TRIGGER_QUIET`]: the device arms at every edge the learner would call a trigger (and,
//!    for a trigger with fewer quiet frames since the run's start, at the same edge: the device
//!    counts a game's first frames as quiet). The notes come from the transcriber
//!    ([`crate::transcribe`]), at least [`MUSIC_NOTES`] over [`MUSIC_PITCHES`] pitches (a held
//!    tone is not music): a note on with its period and level at each note's start, a note
//!    off at its end unless the voice's next note starts there (a legato change of pitch); a
//!    note's end at or past the fingerprint's end is left to the player, which releases the
//!    voices there. Its voices are the most notes sounding at once (the notes moved to voices
//!    below that where needed), its roles by pitch: the voice lowest on average the bass, the
//!    highest the lead, a third the pad, a lone voice the lead.
//! 4. **Verified by replay** ([`record`]): the game is run again under the same inputs through
//!    method B with the game's whole list, and the score must arm at its trigger, follow its
//!    fingerprint to the end and get at least [`CLAIM_SHARE`] of the frames its stretch sounds
//!    in. One that stops early is shortened to the frames it held (and verified again, once) if
//!    that leaves [`SCORE_FRAMES`] frames; otherwise it is left out, with the reason.
//! 5. **Kept:** at most [`SND_SCORES_MAX`], the longest first, within [`SCORES_BYTES_MAX`] bytes
//!    in all (the longest cut to fit when it alone does not), none whose stretch plays with
//!    another's (method B plays one score at a time).

use crate::entry::{ear_edges, encode_list_scores, score_code, score_fingerprint, BuiltinRoles, LearnedEntry, ScoreEvent, ScoreRecord};
use crate::listen::{Inputs, LISTEN_FRAMES};
use crate::transcribe::{transcribe, Note};
use std::collections::HashMap;
use zx_sys::{
    zx_edge, Machine, SndEngines, SND_DELAY_FRAMES, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD, SND_SCORES_MAX, SND_SCORE_BUCKETS_MAX,
    SND_TONAL, ZX_FRAME_TSTATES,
};

/// The frames a routine's `OUT` must sound in to be recorded, and the shortest stretch.
pub const SCORE_FRAMES: u32 = 250;
/// Frames with no edge from the `OUT` before a trigger (or none since the run's start).
pub const TRIGGER_QUIET: u32 = 25;
/// Frames of silence from the `OUT` that end a stretch.
pub const END_SILENCE: u32 = 100;
/// A game's scores' bytes in all.
pub const SCORES_BYTES_MAX: usize = 64 * 1024;
/// The share of the frames a stretch sounds in that method B must give its score.
pub const CLAIM_SHARE: f64 = 0.9;
/// The least notes, and distinct pitches (a semitone apart), a score's transcription holds: a
/// held tone is not music.
pub const MUSIC_NOTES: usize = 8;
pub const MUSIC_PITCHES: usize = 3;
/// 64-T units a bucket (a frame).
const UNITS: u64 = ZX_FRAME_TSTATES as u64 / 64;

/// What the learner's run keeps for the scores ([`Tap::frame`] after each frame): each frame's
/// speaker edges (EAR changes only) and the level before them, and every trigger.
#[derive(Default)]
pub struct Tap {
    frames: Vec<(Vec<zx_edge>, u8)>,
    /// Per `OUT`: the last frame it made a speaker edge in.
    last: HashMap<u16, u32>,
    triggers: Vec<Trigger>,
}

/// A trigger (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Trigger {
    frame: u32,
    site: u16,
    /// The edge's T-state in its frame.
    t0: u32,
    /// The frames with no edge from `site` before it, at most [`TRIGGER_QUIET`].
    quiet: u16,
    /// [`score_code`] at the end of its frame.
    code: u32,
}

impl Tap {
    /// Takes the frame `m` just ran.
    pub fn frame(&mut self, m: &Machine) {
        let f = self.frames.len() as u32;
        let level = m.edges_start_level();
        let edges: Vec<zx_edge> = ear_edges(level, m.edges()).copied().collect();
        let mut seen: Vec<u16> = Vec::new();
        for e in &edges {
            if seen.contains(&e.pc) {
                continue;
            }
            seen.push(e.pc);
            let silent = self.last.get(&e.pc).map_or(Some(f), |&l| (f - l > TRIGGER_QUIET).then_some(f - l - 1));
            if let Some(q) = silent {
                let quiet = q.min(TRIGGER_QUIET) as u16;
                self.triggers.push(Trigger { frame: f, site: e.pc, t0: e.tstate, quiet, code: score_code(|a| m.peek(a), e.pc) });
            }
            self.last.insert(e.pc, f);
        }
        self.frames.push((edges, level));
    }
}

/// Why a stretch ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Inputs,
    Silence,
    Longest,
    Run,
}

impl End {
    fn text(self) -> &'static str {
        match self {
            End::Inputs => "the inputs change",
            End::Silence => "its OUT falls silent",
            End::Longest => "the longest a score may be",
            End::Run => "the run ends",
        }
    }
}

/// A stretch: its trigger, whole buckets from it, and why it ends.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    tr: Trigger,
    buckets: usize,
    end: End,
}

impl Stretch {
    /// The frames the score plays in: from the trigger's, the last holding the last bucket's end.
    fn span(&self) -> std::ops::Range<usize> {
        let j = self.tr.frame as usize;
        j..j + self.buckets + usize::from(self.tr.t0 != 0)
    }
}

/// The frames method B decides in a run of [`LISTEN_FRAMES`].
fn decided() -> usize {
    (LISTEN_FRAMES - SND_DELAY_FRAMES) as usize
}

/// The stretch from trigger `tr` (see the module docs).
fn stretch(tap: &Tap, inputs: &Inputs, tr: Trigger) -> Stretch {
    let j = tr.frame as usize;
    let n = decided().min(tap.frames.len());
    let lead = usize::from(tr.t0 != 0);
    // whole buckets in the frames j..k
    let whole = |k: usize| k.saturating_sub(j).saturating_sub(lead);
    let first = inputs(tr.frame);
    let mut last = (j, tr.t0);
    let mut out = (n, End::Run);
    for f in j..n {
        if f > j && inputs(f as u32) != first {
            out = (f, End::Inputs);
            break;
        }
        let (edges, _) = &tap.frames[f];
        if let Some(e) = edges.iter().rev().find(|e| e.pc == tr.site) {
            last = (f, e.tstate);
        } else if f - last.0 > END_SILENCE as usize {
            out = (f, End::Silence);
            break;
        }
    }
    let mut buckets = whole(out.0);
    let mut end = out.1;
    if end == End::Silence {
        let t = (last.0 - j) as u64 * u64::from(ZX_FRAME_TSTATES) + u64::from(last.1) - u64::from(tr.t0);
        buckets = buckets.min((t / u64::from(ZX_FRAME_TSTATES)) as usize + 1);
    }
    if buckets > SND_SCORE_BUCKETS_MAX {
        (buckets, end) = (SND_SCORE_BUCKETS_MAX, End::Longest);
    }
    Stretch { tr, buckets, end }
}

/// A score recorded for a routine: what the report says of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreSummary {
    /// The frame of its trigger.
    pub frame: u32,
    pub seconds: f64,
    pub voices: u8,
    pub notes: usize,
    /// Its record's bytes.
    pub bytes: usize,
    /// Shortened to the frames method B followed it in (or to fit the game's bytes).
    pub shortened: Option<String>,
}

/// A candidate: its block (the learner's index) and its `OUT`.
pub type Candidate = (usize, u16);

/// A candidate's outcome: its block and its score, or why it has none.
pub type Outcome = (usize, Result<ScoreSummary, String>);

/// A count and a noun, the noun plural unless the count is 1.
fn plural(n: usize, noun: &str) -> String {
    let es = if noun.ends_with("ch") { "es" } else { "s" };
    format!("{n} {noun}{}", if n == 1 { "" } else { es })
}

/// The record of a stretch, its notes counted.
fn build(tap: &Tap, st: &Stretch) -> Result<(ScoreRecord, usize), String> {
    let span = st.span();
    let frames = &tap.frames[span.clone()];
    let notes = transcribe(frames, st.tr.t0);
    let (events, voices, roles, count) = score_events(&notes, st.buckets);
    let mut pitches: Vec<i64> = notes.iter().map(|n| (12.0 * f64::from(n.period_t.max(1)).log2()).round() as i64).collect();
    pitches.sort_unstable();
    pitches.dedup();
    if count < MUSIC_NOTES || pitches.len() < MUSIC_PITCHES {
        return Err(format!(
            "not music: the transcription finds {} over {} in its stretch (at least {MUSIC_NOTES} notes over {MUSIC_PITCHES} pitches)",
            plural(count, "note"),
            plural(pitches.len(), "pitch")
        ));
    }
    let rec = ScoreRecord {
        site: st.tr.site,
        quiet: st.tr.quiet,
        code: st.tr.code,
        roles,
        voices,
        fingerprint: score_fingerprint(frames, 0, st.tr.t0, st.buckets),
        events,
    };
    Ok((rec, count))
}

/// A score's events from the transcriber's notes, within `buckets` from T0: the events, the
/// voices, their roles and the notes (see the module docs).
pub fn score_events(notes: &[Note], buckets: usize) -> (Vec<ScoreEvent>, u8, [u8; SND_TONAL], usize) {
    let limit = buckets as u64 * UNITS;
    let unit = |t: u64| t.saturating_add(32) / 64;
    // (start, end, period, level, voice) in 64-T units; the end at the fingerprint's end when past it
    let mut ns: Vec<(u64, u64, u32, u8, u8)> = notes
        .iter()
        .filter(|n| unit(n.start_t) < limit)
        .map(|n| (unit(n.start_t), unit(n.end_t).min(limit), n.period_t.clamp(64, 0xFF_FFFF), n.level.max(1), n.voice))
        .filter(|n| n.1 > n.0)
        .collect();
    ns.sort_by_key(|n| (n.0, n.4));
    // the voices: the most notes sounding at once
    let mut marks: Vec<(u64, i32)> = ns.iter().flat_map(|n| [(n.0, 1), (n.1, -1)]).collect();
    marks.sort_unstable();
    let (mut now, mut most) = (0i32, 0i32);
    for (_, d) in marks {
        now += d;
        most = most.max(now);
    }
    let voices = most.clamp(1, SND_TONAL as i32) as usize;
    // each note to a voice below that: its own if free, else the free one whose note ended last
    let mut free = vec![0u64; voices];
    for n in &mut ns {
        let own = usize::from(n.4);
        let v = if own < voices && free[own] <= n.0 {
            own
        } else {
            (0..voices).filter(|&v| free[v] <= n.0).max_by_key(|&v| (free[v], std::cmp::Reverse(v))).unwrap_or(0)
        };
        free[v] = n.1;
        n.4 = v as u8;
    }
    // events: (time, off before on, voice), on
    type Key = (u64, u8, u8);
    let mut ev: Vec<(Key, Option<(u32, u8)>)> = Vec::new();
    for v in 0..voices as u8 {
        let mine: Vec<&(u64, u64, u32, u8, u8)> = ns.iter().filter(|n| n.4 == v).collect();
        for (i, n) in mine.iter().enumerate() {
            ev.push(((n.0, 1, v), Some((n.2, n.3))));
            let legato = mine.get(i + 1).is_some_and(|next| next.0 == n.1);
            if n.1 < limit && !legato {
                ev.push(((n.1, 0, v), None));
            }
        }
    }
    ev.sort_by_key(|e| e.0);
    let mut prev = 0;
    let events = ev
        .iter()
        .map(|&((u, _, voice), on)| {
            let e = ScoreEvent { dt_t: (u - prev) * 64, voice, on };
            prev = u;
            e
        })
        .collect();
    // roles by each voice's mean pitch (log period, weighted by duration): high period is low
    let mut by: Vec<(f64, usize)> = (0..voices)
        .map(|v| {
            let (mut w, mut s) = (0f64, 0f64);
            for n in ns.iter().filter(|n| usize::from(n.4) == v) {
                let d = (n.1 - n.0) as f64;
                w += d;
                s += d * f64::from(n.2).log2();
            }
            (if w > 0.0 { s / w } else { 0.0 }, v)
        })
        .collect();
    by.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut roles = [0u8; SND_TONAL];
    match by.len() {
        1 => roles[by[0].1] = SND_ROLE_LEAD,
        k => {
            roles[by[0].1] = SND_ROLE_BASS;
            roles[by[k - 1].1] = SND_ROLE_LEAD;
            for &(_, v) in &by[1..k - 1] {
                roles[v] = SND_ROLE_PAD;
            }
        }
    }
    (events, voices as u8, roles, ns.len())
}

/// `r` cut to its first `buckets` buckets: its events from there on dropped (the player releases
/// the voices where the fingerprint ends); the notes left.
fn cut(r: &ScoreRecord, buckets: usize) -> (ScoreRecord, usize) {
    let limit = buckets as u64 * UNITS * 64;
    let mut t = 0;
    let events: Vec<ScoreEvent> = r
        .events
        .iter()
        .take_while(|e| {
            t += e.dt_t;
            t < limit
        })
        .copied()
        .collect();
    let notes = events.iter().filter(|e| e.on.is_some()).count();
    (ScoreRecord { fingerprint: r.fingerprint[..buckets.min(r.fingerprint.len())].to_vec(), events, ..r.clone() }, notes)
}

/// A record's bytes in the list.
fn bytes(r: &ScoreRecord) -> usize {
    encode_list_scores(&[], &[], std::slice::from_ref(r)).len() - 4
}

/// Method B's decision about each frame it decides in a run of the game under `inputs` with
/// `list`: (confident, engine); with the learned entries alone if not `builtins`. None: the
/// decoder refuses the list.
fn replay(load: &dyn Fn() -> Machine, inputs: &Inputs, list: &[u8], builtins: bool) -> Option<(Vec<(bool, u8)>, usize)> {
    let mut b = if builtins { SndEngines::with_learned(list) } else { SndEngines::with_learned_only(list) }.ok()?;
    let table = b.entries().len();
    let mut m = load();
    let delay = SND_DELAY_FRAMES as usize;
    let mut dec = Vec::with_capacity(decided());
    for f in 0..LISTEN_FRAMES as usize {
        let (rows, kemp) = inputs(f as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let d = b.frame(&m, m.edges(), m.edges_start_level());
        if f >= delay {
            dec.push((d.confident != 0, d.engine));
        }
    }
    Some((dec, table))
}

/// How a score fared in a replay.
enum Fail {
    /// Method B followed it to this frame (the first not decided as music), not to its end.
    Held(usize, String),
    /// Followed to its end, but too few of its frames were the score's.
    Share(String),
}

/// Whether score `index` (of the list, method B's table `table` entries long) fared well in the
/// replay `dec` for its stretch `st` (see the module docs).
fn check(tap: &Tap, dec: &[(bool, u8)], table: usize, index: usize, st: &Stretch) -> Result<(), Fail> {
    let id = (table + 1 + index) as u8;
    let span = st.span();
    let j = span.start;
    if let Some(f) = span.clone().find(|&f| !dec.get(f).is_some_and(|d| d.0)) {
        let why = if f == j {
            format!("method B does not arm it at its trigger (frame {j})")
        } else {
            format!("method B stops following it at frame {f}, {} frames in", f - j)
        };
        return Err(Fail::Held(f, why));
    }
    if dec[j].1 != id && dec[j].1 > table as u8 {
        return Err(Fail::Held(j, format!("score {} plays at its trigger (frame {j})", usize::from(dec[j].1) - table)));
    }
    let sounding: Vec<usize> = span.filter(|&f| !tap.frames[f].0.is_empty()).collect();
    let mine = sounding.iter().filter(|&&f| dec[f].1 == id).count();
    if (mine as f64) < CLAIM_SHARE * sounding.len() as f64 {
        return Err(Fail::Share(format!(
            "method B gives it {mine} of the {} frames its stretch sounds in (at least {:.0}%)",
            sounding.len(),
            100.0 * CLAIM_SHARE
        )));
    }
    Ok(())
}

/// A score kept: its stretch, record and summary.
struct Kept {
    st: Stretch,
    rec: ScoreRecord,
    sum: ScoreSummary,
}

fn summary(st: &Stretch, rec: &ScoreRecord, notes: usize, shortened: Option<String>) -> ScoreSummary {
    ScoreSummary {
        frame: st.tr.frame,
        seconds: st.buckets as f64 * f64::from(ZX_FRAME_TSTATES) / crate::spectrum::CLOCK_HZ,
        voices: rec.voices,
        notes,
        bytes: bytes(rec),
        shortened,
    }
}

/// Records the scores of the candidate routines `cands` (in the learner's order) from the
/// learner's run `tap` (under `inputs`), the game's list holding `entries` and `roles`: the
/// records (in the list's order) and each candidate's outcome (see the module docs).
pub fn record(
    load: &dyn Fn() -> Machine,
    inputs: &Inputs,
    tap: &Tap,
    cands: &[Candidate],
    entries: &[LearnedEntry],
    roles: &[BuiltinRoles],
    builtins: bool,
) -> (Vec<ScoreRecord>, Vec<Outcome>) {
    let mut outcomes: Vec<Outcome> = Vec::new();
    if cands.is_empty() {
        return (Vec::new(), outcomes);
    }
    // the frames method B's table (the built-in and kept entries) claims without scores
    let Some((table, _)) = replay(load, inputs, &encode_list_scores(entries, roles, &[]), builtins) else {
        return (Vec::new(), cands.iter().map(|&(block, _)| (block, Err("method B's decoder refuses the game's list".into()))).collect());
    };
    // each routine's longest stretch, built
    let mut built: Vec<(usize, Stretch, ScoreRecord, usize)> = Vec::new();
    for &(block, site) in cands {
        let best = tap.triggers.iter().filter(|t| t.site == site).map(|&t| stretch(tap, inputs, t)).max_by_key(|s| (s.buckets, std::cmp::Reverse(s.tr.frame)));
        let Some(st) = best.filter(|s| s.buckets >= SCORE_FRAMES as usize) else {
            let why = match best {
                Some(s) => format!("its longest stretch is {} frames (at least {SCORE_FRAMES}), from frame {}, until {}", s.buckets, s.tr.frame, s.end.text()),
                None => "no trigger in the frames method B decides".into(),
            };
            outcomes.push((block, Err(why)));
            continue;
        };
        let sounding: Vec<usize> = st.span().filter(|&f| !tap.frames[f].0.is_empty()).collect();
        let claimed = sounding.iter().filter(|&&f| table.get(f).is_some_and(|d| d.0)).count();
        if claimed as f64 > (1.0 - CLAIM_SHARE) * sounding.len() as f64 {
            outcomes.push((block, Err(format!("method B's table claims {claimed} of the {} frames its stretch sounds in", sounding.len()))));
            continue;
        }
        match build(tap, &st) {
            Ok((rec, notes)) => built.push((block, st, rec, notes)),
            Err(why) => outcomes.push((block, Err(why))),
        }
    }
    built.sort_by_key(|b| std::cmp::Reverse(b.1.buckets));

    let mut kept: Vec<Kept> = Vec::new();
    let list = |k: &[&ScoreRecord]| encode_list_scores(entries, roles, &k.iter().map(|&r| r.clone()).collect::<Vec<_>>());
    for (block, st, rec, notes) in built {
        let got = keep_one(load, inputs, tap, &kept, st, rec, notes, &list, builtins);
        match got {
            Ok(k) => {
                outcomes.push((block, Ok(k.sum.clone())));
                kept.push(k);
            }
            Err(why) => outcomes.push((block, Err(why))),
        }
    }
    (kept.into_iter().map(|k| k.rec).collect(), outcomes)
}

/// One score added to those `kept`, verified (see the module docs): it, or why not.
#[allow(clippy::too_many_arguments)]
fn keep_one(
    load: &dyn Fn() -> Machine,
    inputs: &Inputs,
    tap: &Tap,
    kept: &[Kept],
    mut st: Stretch,
    mut rec: ScoreRecord,
    mut notes: usize,
    list: &dyn Fn(&[&ScoreRecord]) -> Vec<u8>,
    builtins: bool,
) -> Result<Kept, String> {
    if kept.len() >= SND_SCORES_MAX {
        return Err(format!("only {SND_SCORES_MAX} scores a game: the longest first"));
    }
    let span = st.span();
    if let Some(k) = kept.iter().find(|k| k.st.span().start < span.end && span.start < k.st.span().end) {
        return Err(format!("its stretch plays with the score of the OUT at {:04X}h", k.rec.site));
    }
    let mut shortened = None;
    let used: usize = kept.iter().map(|k| k.sum.bytes).sum();
    if used + bytes(&rec) > SCORES_BYTES_MAX {
        if !kept.is_empty() {
            return Err(format!("the game's scores would take over {} KB: the longest first", SCORES_BYTES_MAX / 1024));
        }
        // the longest alone: cut to the most buckets that fit
        let (mut lo, mut hi) = (0usize, st.buckets);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if bytes(&cut(&rec, mid).0) <= SCORES_BYTES_MAX {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo < SCORE_FRAMES as usize {
            return Err(format!("its record would take over {} KB in {SCORE_FRAMES} frames", SCORES_BYTES_MAX / 1024));
        }
        (rec, notes) = cut(&rec, lo);
        st.buckets = lo;
        shortened = Some(format!("to fit {} KB", SCORES_BYTES_MAX / 1024));
    }
    let mut retried = false;
    loop {
        let mut all: Vec<&ScoreRecord> = kept.iter().map(|k| &k.rec).collect();
        all.push(&rec);
        let l = list(&all);
        let Some((dec, table)) = replay(load, inputs, &l, builtins) else {
            return Err("method B's decoder refuses its record".into());
        };
        // the scores kept before still play
        if let Some(k) = kept.iter().enumerate().find(|(i, k)| check(tap, &dec, table, *i, &k.st).is_err()).map(|(_, k)| k) {
            return Err(format!("it keeps the score of the OUT at {:04X}h from playing", k.rec.site));
        }
        match check(tap, &dec, table, kept.len(), &st) {
            Ok(()) => {
                let sum = summary(&st, &rec, notes, shortened);
                return Ok(Kept { st, rec, sum });
            }
            Err(Fail::Share(why)) => return Err(why),
            Err(Fail::Held(f, why)) => {
                let lead = usize::from(st.tr.t0 != 0);
                let held = (f - st.span().start).saturating_sub(lead);
                if retried || held < SCORE_FRAMES as usize {
                    return Err(why);
                }
                retried = true;
                (rec, notes) = cut(&rec, held);
                st.buckets = held;
                shortened = Some(format!("to the {held} frames method B followed it in"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(voice: u8, start: u64, end: u64, period: u32) -> Note {
        Note { voice, start_t: start * 64, end_t: end * 64, period_t: period, level: 100 }
    }

    /// An event at its time from T0 in 64-T units: (time, voice, a note on's period and level).
    type Timed = (u64, u8, Option<(u32, u8)>);

    /// The events as (time in 64-T units, voice, on).
    fn timed(ev: &[ScoreEvent]) -> Vec<Timed> {
        let mut t = 0;
        ev.iter()
            .map(|e| {
                t += e.dt_t;
                (t / 64, e.voice, e.on)
            })
            .collect()
    }

    #[test]
    fn events_from_notes() {
        // a high line with a legato change and a rest, a low note under it; the transcriber's
        // voice 2 moves to voice 1 (two notes at most sound at once)
        let notes = [note(0, 0, 100, 2000), note(2, 10, 300, 8000), note(0, 100, 200, 1800), note(0, 250, 400, 1600)];
        let (ev, voices, roles, n) = score_events(&notes, 1);
        assert_eq!((voices, n), (2, 4));
        assert_eq!(roles, [SND_ROLE_LEAD, SND_ROLE_BASS, 0]);
        assert_eq!(
            timed(&ev),
            [
                (0, 0, Some((2000, 100))),
                (10, 1, Some((8000, 100))),
                (100, 0, Some((1800, 100))),
                (200, 0, None),
                (250, 0, Some((1600, 100))),
                (300, 1, None),
                (400, 0, None),
            ]
        );
        // cut at the fingerprint's end: a note ending there or past it has no off, one starting
        // there is left out
        let (ev, _, _, n) = score_events(&[note(0, 0, 1092, 2000), note(1, 500, 2000, 4000), note(1, 1092, 1500, 3000)], 1);
        assert_eq!(n, 2);
        assert_eq!(timed(&ev), [(0, 0, Some((2000, 100))), (500, 1, Some((4000, 100)))]);
        // three voices: low bass, middle pad, high lead; a lone voice the lead
        let (_, voices, roles, _) = score_events(&[note(0, 0, 50, 1000), note(1, 0, 50, 9000), note(2, 0, 50, 3000)], 1);
        assert_eq!((voices, roles), (3, [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_PAD]));
        assert_eq!(score_events(&[note(1, 5, 50, 1000)], 1).2, [SND_ROLE_LEAD, 0, 0]);
    }

    #[test]
    fn a_cut_score_keeps_its_first_buckets() {
        let (events, voices, roles, _) = score_events(&[note(0, 0, 1000, 2000), note(0, 1500, 2500, 1000)], 3);
        let r = ScoreRecord { site: 0x8000, quiet: 25, code: 1, roles, voices, fingerprint: vec![9, 8, 7], events };
        let (c, n) = cut(&r, 2);
        assert_eq!((c.fingerprint.as_slice(), n), (&[9u8, 8][..], 2));
        assert_eq!(timed(&c.events), [(0, 0, Some((2000, 100))), (1000, 0, None), (1500, 0, Some((1000, 100)))]);
        assert!(bytes(&c) < bytes(&r) && bytes(&c).is_multiple_of(4));
    }
}
