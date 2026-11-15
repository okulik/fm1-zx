// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The gates a routine passes before it gets a learned entry ([`judge`]), and the replay that
//! runs the game again through method B with the candidate entry ([`replay`]).
//!
//! - **Built-in entries win:** a routine the built-in table matches gets no entry
//!   (an anchor or site of the routine inside a built-in's matching signature, [`builtin_at`];
//!   or the replay's built-in claims). [`builtin_at`] also serves a site the code reader refuses.
//! - **Heard:** the fit shows at least 85% inliers, 3 distinct notes, a span of half a
//!   semitone and nothing above 5 kHz; the replay claims at least 90% of the frames the
//!   routine's sites sound in (80% when the fit has at least 95% inliers: the formula is whole,
//!   and the frames left are those B's check cannot judge, such as a burst too short); and the music gate passes over the claimed frames: at least 60% of
//!   the note time in runs of 4 frames or more of one note, some note in two separate runs, and
//!   never 6 changes in a row (one every frame) all one way.
//! - **Code-only** (never heard): the notes come from a table walked by a pointer, the note's
//!   timing is known, and every formula fits a program.
//! - **Drums:** a heard site of another routine whose output does not fit (noise) and that sounds
//!   only between the claimed notes of an entry (outside its routine) becomes that entry's drum site ([`drum_ok`],
//!   [`add_drum`]).

use crate::entry::{encode_list, LearnedEntry};
use crate::fit::{check_fit_played, digest, period_ok, timing_prog, FitResult};
use crate::listen::{Inputs, SiteHeard};
use crate::model::EngineModel;
use crate::prog::Op;
use std::collections::HashMap;
use zx_sys::{Machine, SndEngines, SndFrame, SND_DELAY_FRAMES, SND_LEARNED_LEN_MAX, SND_LEARNED_SITES_MAX, SND_SITE_DRUM, SND_TONAL};

/// The refusal of a routine a built-in entry matches.
pub const BUILTIN: &str = "built-in entry";
/// The refusal of a heard routine judged without a replay, its other gates passed: replay it.
pub const NOT_REPLAYED: &str = "heard, but not replayed";
/// Fit: inlier share, distinct notes, pitch span (semitones), highest frequency.
const INLIERS: f64 = 0.85;
const NOTES: u32 = 3;
const SPAN: f64 = 0.5;
const MAX_HZ: f64 = 5000.0;
/// Replay: the share of the routine's sounding frames method B must claim, in tenths.
const CLAIM_TENTHS: u32 = 9;
/// The claim share for a routine whose fit is this strong (inliers): its formula is not
/// partial, and the frames B leaves out are those its runtime check cannot judge (a short burst
/// of a note each frame, too few flips to check).
const STRONG_FIT: f64 = 0.95;
const STRONG_CLAIM_TENTHS: u32 = 8;
/// Music gate: a held note's frames, the share of note time held (tenths), the changes in a row
/// all one way that make a sweep.
const RUN: usize = 4;
const HELD_TENTHS: usize = 6;
const MONOTONIC: usize = 6;
/// A learned drum's strength.
const DRUM: u8 = 200;
/// Frames between a drum frame and the claimed notes either side of it, at most.
const DRUM_GAP: usize = 6;

/// What the replay of a candidate entry showed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayStats {
    /// Frames with an edge from one of the entry's sites.
    pub routine_frames: u32,
    /// Of those, the frames method B (built-ins plus the entry) gave to the entry.
    pub claimed: u32,
    pub music_gate: bool,
    /// Of the routine's frames, those the built-in table alone claims.
    pub builtin_claimed: u32,
}

/// What a replay saw of the other speaker `OUT`s, for drums (see [`replay_with_others`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtherSite {
    pub pc: u16,
    /// Frames with an edge from it.
    pub frames: u32,
    /// Of those, the frames between claimed notes of the entry: a claimed frame at most
    /// 6 frames before and one at most 6 frames after.
    pub between: u32,
}

/// The verdict on a routine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It gets an entry; `heard`: through the heard gates, else the code-only ones.
    Learned { heard: bool },
    Refused(String),
}

/// The anchor of a built-in entry whose signature matches in `m` with `pc` inside it (from its
/// anchor to its last byte), if any.
pub fn builtin_at(m: &Machine, pc: u16) -> Option<u16> {
    let b = SndEngines::new();
    b.entries().iter().enumerate().find_map(|(i, e)| {
        (0..e.len).map(|back| pc.wrapping_sub(back)).find(|&anchor| b.digest(m, i, anchor) == e.digest)
    })
}

/// Runs the game again from `load()` under `inputs` for `frames` frames through method B with
/// the built-in table and `entry` (and the built-in table alone, alongside): the frames the
/// entry's sites sound in, those B gives the entry, the music gate over those, and the built-ins'
/// claims.
pub fn replay(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, entry: &LearnedEntry) -> ReplayStats {
    replay_with_others(load, inputs, frames, entry).0
}

/// [`replay`], and the other speaker `OUT`s heard while the entry's music played.
pub fn replay_with_others(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, entry: &LearnedEntry) -> (ReplayStats, Vec<OtherSite>) {
    replay_table(load, inputs, frames, entry, true)
}

/// [`replay_with_others`]; without the built-in table if not `builtins` (the entry alone, and no
/// built-in claims: [`crate::learn::LearnOptions::ignore_builtins`]).
pub fn replay_table(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, entry: &LearnedEntry, builtins: bool) -> (ReplayStats, Vec<OtherSite>) {
    let mut m = load();
    let list = encode_list(std::slice::from_ref(entry));
    let (mine, mut with) = if builtins {
        (SndEngines::new().entries().len() as u8 + 1, SndEngines::with_learned(&list).ok())
    } else {
        (1, SndEngines::with_learned_only(&list).ok())
    };
    let mut alone = builtins.then(SndEngines::new);
    let delay = SND_DELAY_FRAMES as usize;
    // per frame: the entry's sites sounded, the other OUTs that did; and from its decisions
    // (frame k's come with frame k + delay): the entry claimed it (with the note at its end, None
    // if silent), the built-ins alone claimed it
    let n = (frames as usize).saturating_sub(delay);
    let mut sounded = Vec::with_capacity(frames as usize);
    let mut others: Vec<Vec<u16>> = Vec::with_capacity(frames as usize);
    let (mut claimed, mut builtin) = (vec![false; n], vec![false; n]);
    let mut notes: Vec<Option<[u32; SND_TONAL]>> = vec![None; n];
    for f in 0..frames as usize {
        let (rows, kemp) = inputs(f as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let (edges, level) = (m.edges(), m.edges_start_level());
        let mut pcs: Vec<u16> = edges.iter().map(|e| e.pc).collect();
        pcs.sort_unstable();
        pcs.dedup();
        let (here, foreign): (Vec<u16>, Vec<u16>) = pcs.into_iter().partition(|&pc| is_site(entry, pc, &m));
        sounded.push(!here.is_empty());
        others.push(foreign);
        let d = with.as_mut().map(|b| b.frame(&m, edges, level));
        let b = alone.as_mut().map(|a| a.frame(&m, edges, level));
        let Some(k) = f.checked_sub(delay).filter(|&k| k < n) else { continue };
        if let Some(d) = d.filter(|d| d.confident != 0 && d.engine == mine) {
            claimed[k] = true;
            notes[k] = Some(sounded_notes(&d)).filter(|v| v.iter().any(|&p| p != 0));
        }
        builtin[k] = b.is_some_and(|b| b.confident != 0 && b.engine != 0);
    }
    let mut st = ReplayStats { routine_frames: 0, claimed: 0, music_gate: music_gate(&notes), builtin_claimed: 0 };
    for k in (0..n).filter(|&k| sounded[k]) {
        st.routine_frames += 1;
        st.claimed += u32::from(claimed[k]);
        st.builtin_claimed += u32::from(builtin[k]);
    }
    // the other OUTs: their frames, and those between claimed notes
    let at: Vec<usize> = (0..n).filter(|&k| claimed[k]).collect();
    let between = |k: usize| {
        let i = at.partition_point(|&c| c < k);
        let prev = i.checked_sub(1).map(|i| at[i]);
        let next = at.get(at.partition_point(|&c| c <= k)).copied();
        prev.is_some_and(|p| k - p <= DRUM_GAP) && next.is_some_and(|x| x - k <= DRUM_GAP)
    };
    let mut per: HashMap<u16, OtherSite> = HashMap::new();
    for (k, pcs) in others.iter().enumerate().take(n) {
        for &pc in pcs {
            let o = per.entry(pc).or_insert(OtherSite { pc, frames: 0, between: 0 });
            o.frames += 1;
            o.between += u32::from(between(k));
        }
    }
    let mut others: Vec<OtherSite> = per.into_values().collect();
    others.sort_unstable_by_key(|o| o.pc);
    (st, others)
}

/// Whether `pc` is one of the entry's sites in `m`: the entry's signature matches at the
/// anchor that puts one of its sites there.
fn is_site(entry: &LearnedEntry, pc: u16, m: &Machine) -> bool {
    let peek = |a: u16| m.peek(a);
    entry.sites.iter().any(|s| digest(&peek, pc.wrapping_sub(s.0), entry.len, &entry.wild, &entry.reloc) == entry.digest)
}

/// The notes a decision sounds: per voice, the last pitch it sounded in the frame (its start's,
/// then its events'; a release keeps the pitch it ends). A routine that plays a short burst of its
/// note each frame is released after each burst, so its voices at the frame's end are silent.
fn sounded_notes(d: &SndFrame) -> [u32; SND_TONAL] {
    let mut v = d.tone.map(|t| if t.level != 0 { t.period_q8 } else { 0 });
    for e in d.events().iter().filter(|e| e.period_q8 != 0 && e.level != 0) {
        if let Some(p) = v.get_mut(e.voice as usize) {
            *p = e.period_q8;
        }
    }
    v
}

/// The music gate over each frame's note (None: not claimed, or silent).
fn music_gate(notes: &[Option<[u32; SND_TONAL]>]) -> bool {
    let total = notes.iter().flatten().count();
    if total == 0 {
        return false;
    }
    // runs of one note
    let mut held = 0;
    let mut runs: HashMap<[u32; SND_TONAL], u32> = HashMap::new();
    let mut i = 0;
    while i < notes.len() {
        let Some(note) = notes[i] else {
            i += 1;
            continue;
        };
        let len = notes[i..].iter().take_while(|n| **n == Some(note)).count();
        if len >= RUN {
            held += len;
        }
        *runs.entry(note).or_default() += 1;
        i += len;
    }
    let recurs = runs.values().any(|&r| r >= 2);
    // a change every frame, all one way (by the highest voice)
    let top = |v: &[u32; SND_TONAL]| v.iter().copied().filter(|&p| p != 0).min().unwrap_or(0);
    let (mut streak, mut way) = (0usize, 0i8);
    let mut sweep = false;
    for w in notes.windows(2) {
        let dir = match (w[0], w[1]) {
            (Some(a), Some(b)) if a != b => top(&b).cmp(&top(&a)) as i8,
            _ => 0,
        };
        streak = if dir != 0 && dir == way { streak + 1 } else { usize::from(dir != 0) };
        way = dir;
        sweep |= streak >= MONOTONIC;
    }
    held * 10 >= total * HELD_TENTHS && recurs && !sweep
}

/// The verdict on `model` (see the module docs), read from `m`'s memory: `heard` the site it was
/// heard at (None: never heard, the code-only gates), `replay` and `fit` what they showed. A
/// routine with its anchor or a site inside a built-in entry's matching signature in `m`
/// ([`builtin_at`]), or whose replay the built-in table claims, is refused [`BUILTIN`] on either
/// path.
pub fn judge(model: &EngineModel, m: &Machine, heard: Option<&SiteHeard>, replay: Option<&ReplayStats>, fit: Option<&FitResult>) -> Verdict {
    judge_table(model, m, heard, replay, fit, true)
}

/// [`judge`]; the built-in entries not consulted if not `builtins`
/// ([`crate::learn::LearnOptions::ignore_builtins`]).
pub fn judge_table(model: &EngineModel, m: &Machine, heard: Option<&SiteHeard>, replay: Option<&ReplayStats>, fit: Option<&FitResult>, builtins: bool) -> Verdict {
    let no = |why: String| Verdict::Refused(why);
    let mut pcs = model.channels.iter().flat_map(|c| c.sites.iter().copied()).chain([model.anchor]);
    if builtins && (replay.is_some_and(|r| r.builtin_claimed > 0) || pcs.any(|pc| builtin_at(m, pc).is_some())) {
        return no(BUILTIN.into());
    }
    if let Some(c) = model.channels.iter().position(|ch| !period_ok(ch)) {
        return no(format!("channel {c}'s formula does not fit a program"));
    }
    if heard.is_none() {
        if model.stack {
            return no("code only: the sound loop uses the stack, which may lie in contended RAM".into());
        }
        if !model.table_walk {
            return no("code only: the notes are not read from a table".into());
        }
        return match &model.duration {
            None => no("code only: the note's timing is not known".into()),
            Some(d) if timing_prog(d).is_none() => no("code only: the note's timing does not fit a program".into()),
            Some(_) => Verdict::Learned { heard: false },
        };
    }
    // a stack in contended RAM slows the pushes and pops: the periods as played must fit too
    if let Some(h) = heard.filter(|_| model.stack) {
        match check_fit_played(model, h) {
            Ok(p) if p.inliers >= INLIERS => {}
            Ok(p) => return no(format!("fit as played (the stack's contention): {:.0}% inliers (at least {:.0}%)", 100.0 * p.inliers, 100.0 * INLIERS)),
            Err(e) => return no(format!("fit as played: {e}")),
        }
    }
    let Some(f) = fit else { return no("heard, but its periods do not fit the formula".into()) };
    if f.inliers < INLIERS {
        return no(format!("fit: {:.0}% inliers (at least {:.0}%)", 100.0 * f.inliers, 100.0 * INLIERS));
    }
    if f.distinct_notes < NOTES {
        return no(format!("fit: {} distinct notes (at least {NOTES})", f.distinct_notes));
    }
    if f.span_semitones < SPAN {
        return no(format!("fit: the notes span {:.2} semitones (at least {SPAN})", f.span_semitones));
    }
    if f.max_hz > MAX_HZ {
        return no(format!("fit: a note at {:.0} Hz (at most {MAX_HZ:.0})", f.max_hz));
    }
    let Some(r) = replay else { return no(NOT_REPLAYED.into()) };
    let tenths = if f.inliers >= STRONG_FIT { STRONG_CLAIM_TENTHS } else { CLAIM_TENTHS };
    if r.routine_frames == 0 || r.claimed * 10 < r.routine_frames * tenths {
        return no(format!("replay: method B claims {} of the {} frames it sounds in (at least {}0%)", r.claimed, r.routine_frames, tenths));
    }
    if !r.music_gate {
        return no("replay: not music (held notes, recurring pitches, no sweeps)".into());
    }
    Verdict::Learned { heard: true }
}

/// Whether the other routine's site `other` (its fit `fit`: none, or failing the fit gates, is
/// noise) is a drum of the entry the replay played: it sounds only between the entry's claimed
/// notes.
pub fn drum_ok(other: &OtherSite, fit: Option<&FitResult>) -> bool {
    let noise = fit.is_none_or(|f| f.inliers < INLIERS || f.distinct_notes < NOTES);
    noise && other.frames > 0 && other.between == other.frames
}

/// `entry` with a drum site at `pc` (another routine's speaker `OUT`), its strength 200: the
/// signature grows to cover it (from an anchor moved back to it, if it lies before), so the
/// whole span between must stay as it is now in `peek`. None if the span would exceed
/// `SND_LEARNED_LEN_MAX` or the sites `SND_LEARNED_SITES_MAX`, or if the drum's `OUT` lies in
/// the entry's own routine (a drum is another routine's). A grown signature holds code the
/// reader never read: the caller replays the entry with the drum before keeping it.
pub fn add_drum(entry: &LearnedEntry, anchor: u16, pc: u16, peek: impl Fn(u16) -> u8) -> Option<LearnedEntry> {
    let inside = |a: u16| a.wrapping_sub(anchor) < entry.len;
    if entry.sites.len() >= SND_LEARNED_SITES_MAX || inside(pc) || inside(pc.wrapping_add(1)) {
        return None;
    }
    // the new anchor and how far the old offsets move
    let (new_anchor, shift) = if pc.wrapping_sub(anchor) < 0x8000 { (anchor, 0) } else { (pc, anchor.wrapping_sub(pc)) };
    let end = (pc.wrapping_sub(new_anchor) + 2).max(entry.len + shift); // an OUT is 2 bytes
    if end > SND_LEARNED_LEN_MAX {
        return None;
    }
    let mut e = entry.clone();
    let mv = |o: u16| o + shift;
    e.len = end;
    e.wild = entry.wild.iter().map(|&o| mv(o)).collect();
    e.reloc = entry.reloc.iter().map(|&o| mv(o)).collect();
    e.sites = entry.sites.iter().map(|&(o, r, c)| (mv(o), r, c)).collect();
    e.sites.push((pc.wrapping_sub(new_anchor), SND_SITE_DRUM, 0));
    e.sites.sort_unstable();
    e.stale = entry.stale.map(|(from, len)| (from.wrapping_add(shift), len));
    let rel = |ops: &[Op]| -> Vec<Op> {
        ops.iter()
            .map(|op| match *op {
                Op::PeekRel(o) => Op::PeekRel(o.wrapping_add(shift)),
                Op::Peek16Rel(o) => Op::Peek16Rel(o.wrapping_add(shift)),
                other => other,
            })
            .collect()
    };
    e.progs = entry.progs.iter().map(|p| rel(p)).collect();
    e.timing = entry.timing.as_deref().map(rel);
    e.drum = DRUM;
    e.digest = digest(&peek, new_anchor, e.len, &e.wild, &e.reloc);
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::music_gate;

    /// Frames of notes: (period, frames) runs, 0 for unclaimed frames.
    fn frames(runs: &[(u32, usize)]) -> Vec<Option<[u32; 3]>> {
        runs.iter().flat_map(|&(p, n)| std::iter::repeat_n((p != 0).then_some([p, 0, 0]), n)).collect()
    }

    #[test]
    fn held_recurring_notes_are_music() {
        // a rising scale of held notes, twice (the tune): changes all one way, but not every frame
        let scale: Vec<(u32, usize)> = (0..8).map(|i| (6000 - 400 * i, 25)).collect();
        assert!(music_gate(&frames(&[scale.clone(), scale].concat())));
    }

    #[test]
    fn what_is_not_music() {
        assert!(!music_gate(&[]));
        assert!(!music_gate(&frames(&[(0, 50)])), "nothing claimed");
        // no note comes back
        assert!(!music_gate(&frames(&[(5000, 20), (4000, 20), (3000, 20)])));
        // short notes: under 60% of the time in runs of 4 frames or more
        let short: Vec<(u32, usize)> = (0..40).map(|i| (3000 + 100 * (i % 3), if i % 2 == 0 { 3 } else { 4 })).collect();
        assert!(!music_gate(&frames(&short)));
        // a sweep: a change every frame, 6 in a row one way, inside otherwise held notes
        let mut sweep = vec![(5000, 30), (4000, 30), (5000, 30), (4000, 30)];
        sweep.extend((1..=6).map(|i| (4000 - 100 * i, 1)));
        assert!(!music_gate(&frames(&sweep)));
        // 5 such changes are not a sweep yet
        sweep.truncate(4 + 5);
        assert!(music_gate(&frames(&sweep)));
    }
}
