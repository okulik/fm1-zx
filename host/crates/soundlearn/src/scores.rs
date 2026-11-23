// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recorded scores: the music of a game's routines that nothing else plays, recorded from the
//! learner's run (and from extra runs, one key each) as kind-2 records ([`ScoreRecord`]) that
//! method B plays in step with the beeper.
//!
//! 1. **Which music** (chosen by the learner, [`crate::learn`]): each heard routine (the
//!    learner's block, from its speaker `OUT` heard in the most frames) that sounded in at least
//!    [`SCORE_FRAMES`] frames, that no kept learned entry gives and no built-in entry matches,
//!    and whose `OUT` lies in no routine before it.
//! 2. **The stretches.** A *trigger* is the first speaker edge from the routine's `OUT` after at
//!    least [`TRIGGER_QUIET`] frames with none from it (or with none since the run's start). A
//!    stretch runs from it while the inputs stay as they were in its frame (a key press may
//!    change the music: it ends with the frame before they change), until the `OUT` has been
//!    silent for [`END_SILENCE`] frames (it ends with the bucket of the `OUT`'s last edge),
//!    [`SND_SCORE_BUCKETS_MAX`] buckets, or the end of the frames method B decides in the run.
//!    Every stretch of at least [`SCORE_FRAMES`] frames is taken (a run of several tunes gives
//!    several); none is recorded more than a tenth of whose sounding frames method B's table
//!    (the built-in and kept entries) claims: that music is played already. Times are T-states
//!    from the trigger edge (T0); a bucket is a frame long from T0, and a stretch is whole
//!    buckets.
//! 3. **The extra runs**, when the learner's run gives a score (step 7): each key of 1-9, 0,
//!    ENTER and SPACE but the one the learner's run presses once
//!    ([`key_runs`](crate::listen::key_runs)), held 5 frames at frame 500, then nothing (as
//!    [`choose_run`](crate::listen::choose_run)'s runs press their start keys): a menu of tunes
//!    plays another behind each key. Each is replayed once through method B with the table, a few
//!    at a time, and its stretches taken as in step 2, from the candidates' `OUT`s and from the
//!    `OUT`s it heard in at least [`SCORE_FRAMES`] frames that are none of theirs and lie in no
//!    learned routine nor a candidate's (another tune's own player).
//! 4. **The same music.** Two stretches are the same music when their fingerprints agree bucket
//!    by bucket as method B compares the live edges with a recorded fingerprint (|a - b| <=
//!    max(2, b / 8)) over their first [`SAME_BUCKETS`] buckets (or the shorter's length): the
//!    longer is kept (of two as long, the earlier run's, then the earlier). A run that sounds no
//!    different from those before (the same tunes, or none) costs its replay alone.
//! 5. **The loop** ([`find_loop`]): most tunes loop, so a stretch's music is looked at for a
//!    loop, on its speaker edges (the engines are deterministic, so a loop's edges repeat): the
//!    shortest `loop_t`, and for it the earliest `intro_t`, such that from the edge at T0 +
//!    `intro_t` to the stretch's end every edge comes again `loop_t` later (the same `OUT`
//!    setting the same level, the gap to the next the same within contention's jitter), with a
//!    whole repeat seen (`intro_t + 2 loop_t` within the stretch) and `loop_t` at least
//!    [`LOOP_FRAMES`] frames. The edges decide: the notes transcribed from two passes may differ
//!    a little. A stretch whose music does not repeat has no loop: its intro is the whole
//!    stretch.
//! 6. **The record.** An intro (`intro_t` from T0) and, with a loop, one pass of it (`loop_t`
//!    from T0 + `intro_t`), which the device follows round for as long as the beeper agrees.
//!    Each section's fingerprint is the speaker edges of each of its whole buckets, on its own
//!    grid from its start ([`score_fingerprint`]); its code is the hash of the bytes around the
//!    `OUT` at the trigger's frame ([`score_code`]), its `quiet` the frames with no edge from the
//!    `OUT` before the trigger, at most [`TRIGGER_QUIET`]: the device arms at every edge the
//!    learner would call a trigger (and, for a trigger with fewer quiet frames since the run's
//!    start, at the same edge: the device counts a game's first frames as quiet). The notes come
//!    from the transcriber ([`crate::transcribe`]) over the intro and the loop's first pass (and
//!    a second more), at least [`MUSIC_NOTES`] over [`MUSIC_PITCHES`] pitches (a held tone is not
//!    music): the intro's events those before `intro_t` (times since T0), the loop's those of its
//!    first pass (times since its start), each voice sounding at the loop's start restated there
//!    by a note on at 0; a note on with its period and level at each note's start, a note off at
//!    its end unless the voice's next note starts there (a legato change of pitch); a note
//!    sounding at its section's end is cut there with no note off, left to the player (which
//!    releases or restates the voices at a pass's start, and releases them at the end of an
//!    intro with no loop). Its voices are the most notes sounding at once (the notes moved to
//!    voices below that where needed), its roles by pitch: the voice lowest on average the bass,
//!    the highest the lead, a third the pad, a lone voice the lead.
//! 7. **Kept** ([`record`]): the learner's run's stretches are chosen first; when that keeps a
//!    score, the extra runs are taken and the choice is made again over all the runs. A choice
//!    takes the stretches the longest first: at most [`SND_SCORES_MAX`] scores, within
//!    [`SCORES_BYTES_MAX`] bytes in all (the longest with no loop cut to fit when it alone does
//!    not), none whose stretch plays with a kept one's in the same run (method B plays one score
//!    at a time), each built only then.
//! 8. **Verified by replay:** the game is run again under the score's run's inputs through
//!    method B with the game's whole list (its entries, roles and every score kept so far, it
//!    last), and the score must arm at its trigger, follow its fingerprint over its whole
//!    stretch (round its loop as many times as the stretch repeats it; the frames within the
//!    delay of the stretch's end aside, where the music may change after it and stop the loop)
//!    and get at least [`CLAIM_SHARE`] of the frames its stretch sounds in, while each score kept
//!    before still does in its own run (replayed again where the new score's `OUT` sounds): with
//!    several scores sharing a trigger (the same `OUT` and code), the one whose fingerprint the
//!    beeper follows plays. A score whose loop fails is recorded with none instead (the intro the
//!    whole stretch) and verified again. One with no loop that stops early is shortened to the
//!    frames it held (and verified again, once) if that leaves [`SCORE_FRAMES`] frames; otherwise
//!    it is left out, with the reason.

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
pub const SCORES_BYTES_MAX: usize = 128 * 1024;
/// The buckets two fingerprints are compared over for the same music (or the shorter's length).
pub const SAME_BUCKETS: usize = 100;
/// The share of the frames a stretch sounds in that method B must give its score.
pub const CLAIM_SHARE: f64 = 0.9;
/// The least notes, and distinct pitches (a semitone apart), a score's transcription holds: a
/// held tone is not music.
pub const MUSIC_NOTES: usize = 8;
pub const MUSIC_PITCHES: usize = 3;
/// Extra runs replayed at once (each holds its tap).
const EXTRA_AT_ONCE: usize = 4;
/// A record's fixed part, before its pitches.
const RECORD_HEAD: usize = 32;
/// Frames transcribed past a score's last section, so that its last notes are read whole.
const TRANSCRIBE_MORE: usize = 50;
/// 64-T units a bucket (a frame).
const UNITS: u64 = ZX_FRAME_TSTATES as u64 / 64;

/// What a run keeps for the scores ([`Tap::frame`] after each frame): each frame's
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

/// The shortest loop a score may have, in buckets.
pub const LOOP_FRAMES: usize = 50;
/// The share of a stretch's last lag of buckets that may disagree with the lag before it for
/// that lag to be tried as a loop (see [`find_loop`]).
const LOOP_COARSE: f64 = 0.15;
/// How far two edges a loop apart may stray from the loop's length (the 48K's contention and
/// interrupts slow the music by an amount that depends on where in the frame it is).
const LOOP_WOBBLE: u64 = ZX_FRAME_TSTATES as u64 / 8;

/// A speaker edge of a stretch: its time since T0, its `OUT` and the level it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edge {
    t: u64,
    pc: u16,
    level: u8,
}

/// The speaker edges ([`ear_edges`] of `frames`, each frame's edges and the level before them)
/// in the first `len_t` T-states from T0, T-state `t0` of the first frame.
fn edge_times(frames: &[(Vec<zx_edge>, u8)], t0: u32, len_t: u64) -> Vec<Edge> {
    let mut out = Vec::new();
    for (f, (edges, level)) in frames.iter().enumerate() {
        for e in ear_edges(*level, edges) {
            let Some(t) = (f as u64 * u64::from(ZX_FRAME_TSTATES) + u64::from(e.tstate)).checked_sub(u64::from(t0)) else { continue };
            if t < len_t {
                out.push(Edge { t, pc: e.pc, level: e.level & 2 });
            }
        }
    }
    out
}

/// A score's loop: the music from T0 + `intro_t` repeats every `loop_t` T-states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loop {
    pub intro_t: u32,
    pub loop_t: u32,
}

/// The loop of a stretch's music (`frames` from the trigger's, T0 T-state `t0` of the first, the
/// stretch `buckets` long), if it has one (see the module docs).
fn stretch_loop(frames: &[(Vec<zx_edge>, u8)], t0: u32, buckets: usize) -> Option<Loop> {
    let len_t = buckets as u64 * u64::from(ZX_FRAME_TSTATES);
    find_loop(&edge_times(frames, t0, len_t), len_t)
}

/// The loop of the speaker edges `e` (times since T0) of a stretch `len_t` T-states long: the
/// shortest `loop_t`, and for it the earliest `intro_t`, such that from the edge at `intro_t` on
/// every edge is followed `loop_t` later by one from the same `OUT` setting the same level, the
/// gaps between them the same, to the stretch's end, with a whole repeat at least (`intro_t + 2
/// loop_t` within the stretch). The music engines are deterministic, but the 48K's contention
/// (and interrupts) slow them by an amount that depends on where in the frame they are: a gap
/// may differ by 1/16 of it (32 T at least), an edge stray up to [`LOOP_WOBBLE`] from where the
/// loop's length puts it, and `loop_t` is the mean distance of the edges paired.
///
/// The lags are found coarsely first, on the edges counted per bucket: a lag of `p` buckets is
/// tried when the last `p` buckets agree with the `p` before them (each within the two buckets
/// the lag falls between, or 1/8 off) for all but [`LOOP_COARSE`] of their edges; then each
/// edge of the stretch's last edge's kind that lies a lag back is tried as its pair, the shortest
/// lag first.
fn find_loop(e: &[Edge], len_t: u64) -> Option<Loop> {
    let frame = u64::from(ZX_FRAME_TSTATES);
    let n = (len_t / frame) as usize;
    if e.len() < 2 || n <= 2 * LOOP_FRAMES {
        return None;
    }
    let mut f = vec![0u32; n];
    for x in e {
        if let Some(c) = f.get_mut((x.t / frame) as usize) {
            *c += 1;
        }
    }
    let mut lags: Vec<usize> = Vec::new();
    for p in LOOP_FRAMES..=(n - 1) / 2 {
        let (mut off, mut all) = (0u64, 0u64);
        for b in n - p..n {
            let (a, c) = (f[b - p], f[b - p - 1]);
            let (lo, hi) = (a.min(c), a.max(c));
            let v = f[b];
            let slack = (v / 8).max(2);
            off += u64::from(lo.saturating_sub(v).saturating_sub(slack) + v.saturating_sub(hi).saturating_sub(slack));
            all += u64::from(v);
        }
        if all > 0 && (off as f64) <= LOOP_COARSE * all as f64 {
            lags.push(p);
        }
    }
    let r = e.len() - 1;
    let last = e[r];
    let gap = |i: usize| e[i + 1].t - e[i].t;
    let close = |a: u64, b: u64| a.abs_diff(b) <= (a.max(b) / 16).max(32);
    let mut tried = 0..0;
    for p in lags {
        // the pairs a lag of p buckets (and the wobble) back, not tried before: the shortest first
        let from = last.t.saturating_sub((p as u64 + 1) * frame + LOOP_WOBBLE);
        let to = last.t.saturating_sub((p as u64 * frame).saturating_sub(LOOP_WOBBLE));
        let lo = e.partition_point(|x| x.t < from);
        let hi = e.partition_point(|x| x.t <= to).min(r);
        for c in (lo..hi).rev().filter(|c| !tried.contains(c)) {
            if (e[c].pc, e[c].level) != (last.pc, last.level) {
                continue;
            }
            let (k, l0) = (r - c, last.t - e[c].t);
            if l0 < LOOP_FRAMES as u64 * frame {
                continue;
            }
            // back from the last pair while the pairs hold
            let mut j = c;
            let mut sum = u128::from(l0);
            while j > 0 {
                let i = j - 1;
                let (a, b) = (e[i], e[i + k]);
                if (a.pc, a.level) != (b.pc, b.level) || !close(gap(i), gap(i + k)) || (b.t - a.t).abs_diff(l0) > LOOP_WOBBLE {
                    break;
                }
                sum += u128::from(b.t - a.t);
                j = i;
            }
            let loop_t = (sum / (c - j + 1) as u128) as u64;
            let intro_t = e[j].t;
            if intro_t + 2 * loop_t <= len_t {
                return Some(Loop { intro_t: intro_t as u32, loop_t: loop_t as u32 });
            }
        }
        // the windows move back as the lag grows: joined while they overlap
        tried = if tried.is_empty() || hi < tried.start { lo..hi } else { lo.min(tried.start)..hi.max(tried.end) };
    }
    None
}

/// A score recorded for a routine: what the report says of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreSummary {
    /// The frame of its trigger.
    pub frame: u32,
    pub seconds: f64,
    /// Its loop: the intro's seconds and a pass's.
    pub looped: Option<(f64, f64)>,
    pub voices: u8,
    /// The note ons stored.
    pub notes: usize,
    /// Its record's bytes.
    pub bytes: usize,
    /// Shortened to the frames method B followed it in (or to fit the game's bytes).
    pub shortened: Option<String>,
}

/// A candidate: its block (the learner's index) and its `OUT`.
pub type Candidate = (usize, u16);

/// A score's outcome for the report: the run it is from (None: the learner's; else its short
/// name, `key 3 once`) and the score, or why it has none.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreOutcome {
    pub run: Option<String>,
    pub got: Result<ScoreSummary, String>,
}

/// A candidate's outcome: its block and a score's outcome (a candidate has one or more).
pub type Outcome = (usize, ScoreOutcome);

/// A count and a noun, the noun plural unless the count is 1.
fn plural(n: usize, noun: &str) -> String {
    let es = if noun.ends_with("ch") { "es" } else { "s" };
    format!("{n} {noun}{}", if n == 1 { "" } else { es })
}

/// The record of a stretch with its loop `lp` (None: none, the intro the whole stretch), its
/// notes counted.
fn build(tap: &Tap, st: &Stretch, lp: Option<Loop>) -> Result<(ScoreRecord, usize), String> {
    let span = st.span();
    let frames = &tap.frames[span.clone()];
    let len_t = st.buckets as u64 * u64::from(ZX_FRAME_TSTATES);
    let (intro_t, loop_t) = lp.map_or((len_t, 0), |l| (u64::from(l.intro_t), u64::from(l.loop_t)));
    // the frames the notes are read from: the intro and the loop's first pass, and a second more
    let needed = (intro_t + loop_t).div_ceil(u64::from(ZX_FRAME_TSTATES)) as usize + 1 + TRANSCRIBE_MORE;
    let notes = transcribe(&frames[..needed.min(frames.len())], st.tr.t0);
    let ev = score_events(&notes, intro_t, loop_t);
    let semitone = |n: &Note| (12.0 * f64::from(n.period_t.max(1)).log2()).round() as i64;
    let mut pitches: Vec<i64> = notes.iter().filter(|n| n.start_t < intro_t + loop_t).map(semitone).collect();
    pitches.sort_unstable();
    pitches.dedup();
    if ev.notes < MUSIC_NOTES || pitches.len() < MUSIC_PITCHES {
        return Err(format!(
            "not music: the transcription finds {} over {} in its stretch (at least {MUSIC_NOTES} notes over {MUSIC_PITCHES} pitches)",
            plural(ev.notes, "note"),
            plural(pitches.len(), "pitch")
        ));
    }
    let rec = ScoreRecord {
        site: st.tr.site,
        quiet: st.tr.quiet,
        code: st.tr.code,
        roles: ev.roles,
        voices: ev.voices,
        intro_t: intro_t as u32,
        loop_t: loop_t as u32,
        intro_fp: score_fingerprint(frames, st.tr.t0, 0, intro_t),
        loop_fp: score_fingerprint(frames, st.tr.t0, intro_t, loop_t),
        intro_events: ev.intro,
        loop_events: ev.looped,
    };
    Ok((rec, ev.notes))
}

/// A score's events ([`score_events`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreEvents {
    /// The intro's events (times since T0).
    pub intro: Vec<ScoreEvent>,
    /// The loop's first pass's events (times since its start).
    pub looped: Vec<ScoreEvent>,
    /// The voices (1-3) and their roles.
    pub voices: u8,
    pub roles: [u8; SND_TONAL],
    /// The note ons stored.
    pub notes: usize,
}

/// A score's events from the transcriber's notes: the intro's, `intro_t` from T0, and the loop's
/// first pass's, `loop_t` from there (0: no loop), each section's notes cut at its ends, the
/// voices and their roles (see the module docs).
pub fn score_events(notes: &[Note], intro_t: u64, loop_t: u64) -> ScoreEvents {
    let unit = |t: u64| t.saturating_add(32) / 64;
    // each section's start (T from T0) and its units: an event's unit u holds 64 u < the length
    let secs = [(0, intro_t.div_ceil(64)), (intro_t, loop_t.div_ceil(64))];
    // a time in a section as one order: the section above the units
    let at = |sec: usize, u: u64| (sec as u64) << 40 | u;
    // the pieces of the notes in each section: (start, end, period, level, voice, cut at the
    // section's end) in that order; a note sounding at the loop's start restated there
    let mut ns: Vec<(u64, u64, u32, u8, u8, bool)> = Vec::new();
    for (sec, &(from, lim)) in secs.iter().enumerate() {
        for n in notes {
            let to = from + lim * 64;
            if n.end_t <= from || n.start_t >= to {
                continue;
            }
            let start = unit(n.start_t.max(from) - from);
            let end = unit(n.end_t.min(to) - from);
            if start >= lim {
                continue;
            }
            let cut = end >= lim;
            if !cut && end <= start {
                continue;
            }
            ns.push((at(sec, start), at(sec, end.min(lim)), n.period_t.clamp(64, 0xFF_FFFF), n.level.max(1), n.voice, cut));
        }
    }
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
    // events: (time, off before on, voice), on; no off where a note is cut or the voice's next
    // note starts (a legato change of pitch)
    type Key = (u64, u8, u8);
    let mut ev: Vec<(Key, Option<(u32, u8)>)> = Vec::new();
    for v in 0..voices as u8 {
        let mine: Vec<&(u64, u64, u32, u8, u8, bool)> = ns.iter().filter(|n| n.4 == v).collect();
        for (i, n) in mine.iter().enumerate() {
            ev.push(((n.0, 1, v), Some((n.2, n.3))));
            let legato = mine.get(i + 1).is_some_and(|next| next.0 == n.1);
            if !n.5 && !legato {
                ev.push(((n.1, 0, v), None));
            }
        }
    }
    ev.sort_by_key(|e| e.0);
    let (mut intro, mut looped) = (Vec::new(), Vec::new());
    for &((key, _, voice), on) in &ev {
        let e = ScoreEvent { t: ((key & ((1 << 40) - 1)) * 64) as u32, voice, on };
        if key >> 40 == 0 { intro.push(e) } else { looped.push(e) }
    }
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
    ScoreEvents { intro, looped, voices: voices as u8, roles, notes: ns.len() }
}

/// `r` cut to its first `buckets` buckets: its events from there on dropped (the player releases
/// the voices where the fingerprint ends); the notes left.
fn cut(r: &ScoreRecord, buckets: usize) -> (ScoreRecord, usize) {
    let limit = buckets as u64 * UNITS * 64;
    let events: Vec<ScoreEvent> = r.intro_events.iter().take_while(|e| u64::from(e.t) < limit).copied().collect();
    let notes = events.iter().filter(|e| e.on.is_some()).count();
    let intro_fp = r.intro_fp[..buckets.min(r.intro_fp.len())].to_vec();
    (ScoreRecord { intro_t: limit as u32, intro_fp, intro_events: events, ..r.clone() }, notes)
}

/// A record's bytes in the list.
fn bytes(r: &ScoreRecord) -> usize {
    encode_list_scores(&[], &[], std::slice::from_ref(r)).len() - 4
}

/// Method B's decision about each frame it decides in a run of the game under `inputs` with
/// `list`: (confident, engine); with the learned entries alone if not `builtins`. None: the
/// decoder refuses the list.
fn replay(load: &dyn Fn() -> Machine, inputs: &Inputs, list: &[u8], builtins: bool) -> Option<(Vec<(bool, u8)>, usize)> {
    replay_tap(load, inputs, list, builtins, None)
}

/// [`replay`], each frame given to `tap` too.
fn replay_tap(load: &dyn Fn() -> Machine, inputs: &Inputs, list: &[u8], builtins: bool, mut tap: Option<&mut Tap>) -> Option<(Vec<(bool, u8)>, usize)> {
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
        if let Some(t) = tap.as_deref_mut() {
            t.frame(&m);
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

/// Whether score `index` (of the list, method B's table `table` entries long; `looped`: it has a
/// loop) fared well in the replay `dec` for its stretch `st` (see the module docs).
fn check(tap: &Tap, dec: &[(bool, u8)], table: usize, index: usize, st: &Stretch, looped: bool) -> Result<(), Fail> {
    let id = (table + 1 + index) as u8;
    let span = st.span();
    let j = span.start;
    // a loop is followed past the stretch's end: where the music then changes, the frames within
    // the delay of the third bucket that disagrees are not its
    let held = if looped { j..(span.end + 2).saturating_sub(SND_DELAY_FRAMES as usize).max(j + 1) } else { span.clone() };
    if let Some(f) = held.clone().find(|&f| !dec.get(f).is_some_and(|d| d.0)) {
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

/// A run the scores are recorded from: its name in the report (None: the learner's run), its
/// inputs and its tap.
struct Run<'a> {
    name: Option<String>,
    inputs: &'a Inputs,
    tap: Tap,
}

impl Run<'_> {
    /// ` (key 3 once)` after a score's words, for a run not the learner's.
    fn suffix(&self) -> String {
        self.name.as_ref().map_or(String::new(), |n| format!(" ({n})"))
    }
}

/// A stretch taken from a run (see the module docs): its run, its candidate's block (None: an
/// `OUT` heard in the extra runs only), the stretch and its fingerprint, and what became of it.
struct Take {
    run: usize,
    block: Option<usize>,
    st: Stretch,
    fp: Vec<u8>,
    /// The same music as this take's, which is longer (or as long and earlier).
    same: Option<usize>,
    /// Left out whatever the choice (method B's table claims it, or it is not music), and why.
    left: Option<String>,
    /// Left out because method B's table claims it.
    claimed: bool,
    /// Left out because it plays with a score kept.
    overlap: bool,
    /// Its loop, once looked for (None inside: it has none).
    lp: Option<Option<Loop>>,
    /// Its record and notes, once built (with its loop, if it has one).
    built: Option<(ScoreRecord, usize)>,
    /// Its record with no loop, once built (when the one with its loop is not kept), or why not.
    plain: Option<Result<(ScoreRecord, usize), String>>,
    /// The choice's outcome.
    got: Option<Result<ScoreSummary, String>>,
}

impl Take {
    /// Not the same music as another's, nor left out before the choice.
    fn open(&self) -> bool {
        self.same.is_none() && self.left.is_none()
    }
}

/// Whether two fingerprints are the same music: they agree bucket by bucket as method B
/// compares the live edges with a recorded fingerprint `rec` (|live - rec| <= max(2, rec / 8)),
/// over their first [`SAME_BUCKETS`] buckets (or the shorter's length).
fn same(live: &[u8], rec: &[u8]) -> bool {
    live.iter().zip(rec).take(SAME_BUCKETS).all(|(&l, &r)| l.abs_diff(r) <= (r / 8).max(2))
}

/// Adds `t` to the takes: the same music as a take as long or longer, it is that one's; else
/// the takes it is the same music as become its.
fn add(takes: &mut Vec<Take>, mut t: Take) {
    if let Some(k) = takes.iter().position(|k| k.same.is_none() && k.st.buckets >= t.st.buckets && same(&t.fp, &k.fp)) {
        t.same = Some(k);
    } else {
        let i = takes.len();
        for k in takes.iter_mut().filter(|k| k.same.is_none() && same(&k.fp, &t.fp)) {
            k.same = Some(i);
        }
    }
    takes.push(t);
}

/// The stretches of at least [`SCORE_FRAMES`] frames in run `r` of each `OUT` of `outs` (its
/// candidate's block, or None for one heard in the extra runs only), added to the takes; and
/// each candidate with none, with why.
fn gather(run: &Run, r: usize, outs: &[(Option<usize>, u16)], takes: &mut Vec<Take>) -> Vec<(usize, String)> {
    let mut none = Vec::new();
    for &(block, site) in outs {
        let all: Vec<Stretch> = run.tap.triggers.iter().filter(|t| t.site == site).map(|&t| stretch(&run.tap, run.inputs, t)).collect();
        let longest = all.iter().max_by_key(|s| (s.buckets, std::cmp::Reverse(s.tr.frame)));
        if longest.is_none_or(|s| s.buckets < SCORE_FRAMES as usize) {
            let why = match longest {
                Some(s) => format!("its longest stretch is {} frames (at least {SCORE_FRAMES}), from frame {}, until {}", s.buckets, s.tr.frame, s.end.text()),
                None => "no trigger in the frames method B decides".into(),
            };
            none.extend(block.map(|b| (b, why)));
            continue;
        }
        for st in all.into_iter().filter(|s| s.buckets >= SCORE_FRAMES as usize) {
            let fp = score_fingerprint(&run.tap.frames[st.span()], st.tr.t0, 0, st.buckets as u64 * UNITS * 64);
            add(takes, Take { run: r, block, st, fp, same: None, left: None, claimed: false, overlap: false, lp: None, built: None, plain: None, got: None });
        }
    }
    none
}

/// The `OUT`s of an extra run's tap heard in at least [`SCORE_FRAMES`] frames, the most first,
/// that are no candidate's and that `covered` does not name.
fn heard_only(tap: &Tap, cands: &[Candidate], covered: &dyn Fn(u16) -> bool) -> Vec<u16> {
    let mut frames: HashMap<u16, u32> = HashMap::new();
    for (edges, _) in &tap.frames {
        let mut seen: Vec<u16> = Vec::new();
        for e in edges {
            if !seen.contains(&e.pc) {
                seen.push(e.pc);
                *frames.entry(e.pc).or_default() += 1;
            }
        }
    }
    let mut outs: Vec<(u16, u32)> = frames.into_iter().filter(|&(pc, n)| n >= SCORE_FRAMES && !cands.iter().any(|c| c.1 == pc) && !covered(pc)).collect();
    outs.sort_by_key(|&(pc, n)| (std::cmp::Reverse(n), pc));
    outs.into_iter().map(|o| o.0).collect()
}

/// Leaves out each open take of run `r` more than a tenth of whose sounding frames method B's
/// table (the built-in and kept entries) claims: `table`, its decisions in the run.
fn claims(run: &Run, r: usize, takes: &mut [Take], table: &[(bool, u8)]) {
    for t in takes.iter_mut().filter(|t| t.run == r && t.open()) {
        let sounding: Vec<usize> = t.st.span().filter(|&f| !run.tap.frames[f].0.is_empty()).collect();
        let claimed = sounding.iter().filter(|&&f| table.get(f).is_some_and(|d| d.0)).count();
        if claimed as f64 > (1.0 - CLAIM_SHARE) * sounding.len() as f64 {
            t.left = Some(format!("method B's table claims {claimed} of the {} frames its stretch sounds in", sounding.len()));
            t.claimed = true;
        }
    }
}

/// A score kept: its run, stretch, record and summary.
struct Kept {
    run: usize,
    st: Stretch,
    rec: ScoreRecord,
    sum: ScoreSummary,
}

fn summary(st: &Stretch, rec: &ScoreRecord, notes: usize, shortened: Option<String>) -> ScoreSummary {
    ScoreSummary {
        frame: st.tr.frame,
        seconds: st.buckets as f64 * f64::from(ZX_FRAME_TSTATES) / crate::spectrum::CLOCK_HZ,
        looped: (rec.loop_t > 0).then(|| (f64::from(rec.intro_t) / crate::spectrum::CLOCK_HZ, f64::from(rec.loop_t) / crate::spectrum::CLOCK_HZ)),
        voices: rec.voices,
        notes,
        bytes: bytes(rec),
        shortened,
    }
}

/// What the choice needs besides the runs and the takes.
struct Game<'a> {
    load: &'a (dyn Fn() -> Machine + Sync),
    entries: &'a [LearnedEntry],
    roles: &'a [BuiltinRoles],
    builtins: bool,
}

/// The choice over the open takes (of the learner's run alone if `first`), the longest first
/// (see the module docs): each one's outcome, and the scores kept.
fn choose(g: &Game, runs: &[Run], takes: &mut [Take], first: bool) -> Vec<Kept> {
    let mut order: Vec<usize> = (0..takes.len()).filter(|&i| takes[i].open() && (!first || takes[i].run == 0)).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(takes[i].st.buckets), takes[i].run, takes[i].st.tr.frame, takes[i].st.tr.site));
    let mut kept: Vec<Kept> = Vec::new();
    for i in order {
        let t = &mut takes[i];
        t.got = None;
        if let Err((why, overlap)) = room(runs, &kept, t.run, &t.st) {
            t.got = Some(Err(why));
            t.overlap = overlap;
            continue;
        }
        let tap = &runs[t.run].tap;
        let lp = *t.lp.get_or_insert_with(|| stretch_loop(&tap.frames[t.st.span()], t.st.tr.t0, t.st.buckets));
        // a record holds a nibble at least for each bucket of its sections: one that cannot fit
        // is not built
        let used: usize = kept.iter().map(|k| k.sum.bytes).sum();
        let stored = lp.map_or(t.st.buckets, |l| ((u64::from(l.intro_t) + u64::from(l.loop_t)) / u64::from(ZX_FRAME_TSTATES)) as usize);
        if !kept.is_empty() && used + RECORD_HEAD + stored / 2 > SCORES_BYTES_MAX {
            t.got = Some(Err(over_bytes()));
            continue;
        }
        if t.built.is_none() {
            // with its loop; one whose loop is not music, with none
            match build(tap, &t.st, lp).or_else(|why| if lp.is_some() { plain(tap, t) } else { Err(why) }) {
                Ok(b) => t.built = Some(b),
                Err(why) => {
                    t.left = Some(why);
                    continue;
                }
            }
        }
        let Some((rec, notes)) = t.built.clone() else { continue };
        let looped = rec.loop_t > 0;
        let mut got = keep_one(g, runs, &kept, t.run, t.st, rec, notes);
        if got.is_err() && looped {
            // its loop not followed: the record with none
            got = plain(tap, t).and_then(|(rec, notes)| keep_one(g, runs, &kept, t.run, t.st, rec, notes));
        }
        t.got = Some(got.map(|k| {
            let sum = k.sum.clone();
            kept.push(k);
            sum
        }));
    }
    kept
}

/// Take `t`'s record with no loop (built once).
fn plain(tap: &Tap, t: &mut Take) -> Result<(ScoreRecord, usize), String> {
    t.plain.get_or_insert_with(|| build(tap, &t.st, None)).clone()
}

/// The outcomes of `takes` for the report, in the order of the runs and then of the frames, the
/// same reasons together (`... (3 stretches)`); with `all`, those that play with a score kept
/// too, and of those method B's table claims, the longest of each run.
fn lines(runs: &[Run], takes: &[&Take], all: bool) -> Vec<ScoreOutcome> {
    let longest = |t: &Take| !takes.iter().any(|k| k.claimed && k.same.is_none() && k.run == t.run && (k.st.buckets, std::cmp::Reverse(k.st.tr.frame)) > (t.st.buckets, std::cmp::Reverse(t.st.tr.frame)));
    let mut mine: Vec<(usize, u32, ScoreOutcome)> = takes
        .iter()
        .filter(|t| t.same.is_none() && (all || !t.claimed && !t.overlap) && (!t.claimed || longest(t)))
        .filter_map(|t| t.left.clone().map(Err).or_else(|| t.got.clone()).map(|got| (t.run, t.st.tr.frame, ScoreOutcome { run: runs[t.run].name.clone(), got })))
        .collect();
    mine.sort_by_key(|m| (m.0, m.1));
    let mut out: Vec<(ScoreOutcome, usize)> = Vec::new();
    for (_, _, o) in mine {
        match out.iter_mut().find(|l| l.0.got.is_err() && l.0 == o) {
            Some(l) => l.1 += 1,
            None => out.push((o, 1)),
        }
    }
    out.into_iter()
        .map(|(mut o, n)| {
            if n > 1 {
                o.got = o.got.map_err(|why| format!("{why} ({n} stretches)"));
            }
            o
        })
        .collect()
}

/// Records the scores of the candidate routines `cands` (in the learner's order) from the
/// learner's run `tap` (under `inputs`) and, when it gives one, from the `extra` runs too (each
/// named for the report, with its inputs): theirs, and those of the `OUT`s heard in them only
/// that `covered` does not name; the game's list holding `entries` and `roles`. Returns the
/// records (in the list's order), the candidates' outcomes (each candidate's in the order of the
/// runs and then of the frames) and the outcomes of the `OUT`s heard in the extra runs only that
/// have any but the same music as another's or one method B's table claims (see the module
/// docs).
#[allow(clippy::too_many_arguments)]
pub fn record(
    load: &(dyn Fn() -> Machine + Sync),
    inputs: &Inputs,
    tap: Tap,
    extra: &[(String, Inputs)],
    cands: &[Candidate],
    covered: &dyn Fn(u16) -> bool,
    entries: &[LearnedEntry],
    roles: &[BuiltinRoles],
    builtins: bool,
) -> Recorded {
    if cands.is_empty() {
        return Recorded::default();
    }
    let refused = || Recorded {
        outcomes: cands.iter().map(|&(block, _)| (block, ScoreOutcome { run: None, got: Err("method B's decoder refuses the game's list".into()) })).collect(),
        ..Recorded::default()
    };
    let g = Game { load, entries, roles, builtins };
    let base = encode_list_scores(entries, roles, &[]);
    let mut runs = vec![Run { name: None, inputs, tap }];
    let mut takes: Vec<Take> = Vec::new();
    let outs: Vec<(Option<usize>, u16)> = cands.iter().map(|&(b, s)| (Some(b), s)).collect();
    let none = gather(&runs[0], 0, &outs, &mut takes);
    let Some((table, _)) = replay(load, inputs, &base, builtins) else { return refused() };
    claims(&runs[0], 0, &mut takes, &table);
    let mut kept = choose(&g, &runs, &mut takes, true);
    // the extra runs, when the learner's run gives a score; then the choice over all the runs
    if !kept.is_empty() && !extra.is_empty() {
        // a few at a time, each tapped with its table's decisions in one replay
        for chunk in extra.chunks(EXTRA_AT_ONCE) {
            let tapped = par(chunk.len(), &|i| {
                let mut tap = Tap::default();
                replay_tap(load, &chunk[i].1, &base, builtins, Some(&mut tap)).map(|(table, _)| (tap, table))
            });
            for ((name, inputs), got) in chunk.iter().zip(tapped) {
                let Some((tap, table)) = got else { return refused() };
                let r = runs.len();
                runs.push(Run { name: Some(name.clone()), inputs, tap });
                let mut outs = outs.clone();
                outs.extend(heard_only(&runs[r].tap, cands, covered).into_iter().map(|s| (None, s)));
                gather(&runs[r], r, &outs, &mut takes);
                claims(&runs[r], r, &mut takes, &table);
                if !takes.iter().any(|t| t.run == r && t.open()) {
                    // it sounds no different: its tap is not needed
                    runs[r].tap = Tap::default();
                }
            }
        }
        for t in &mut takes {
            t.got = None;
            t.overlap = false;
        }
        kept = choose(&g, &runs, &mut takes, false);
    }

    // the outcomes: each candidate's takes, else why it has none
    let mut outcomes: Vec<Outcome> = Vec::new();
    for &(block, _) in cands {
        let mine: Vec<&Take> = takes.iter().filter(|t| t.block == Some(block)).collect();
        let got = lines(&runs, &mine, true);
        if !got.is_empty() {
            outcomes.extend(got.into_iter().map(|o| (block, o)));
            continue;
        }
        let why = if let Some(t) = mine.iter().find(|t| t.same.is_some()) {
            // the same music as another take's (followed to the one that stands)
            let mut k: &Take = t;
            while let Some(j) = k.same {
                k = &takes[j];
            }
            format!("the same music as the stretch of the OUT at {:04X}h from frame {}{}", k.st.tr.site, k.st.tr.frame, runs[k.run].suffix())
        } else {
            none.iter().find(|n| n.0 == block).map_or_else(|| "no stretch".into(), |n| n.1.clone())
        };
        outcomes.push((block, ScoreOutcome { run: None, got: Err(why) }));
    }
    // the OUTs heard in the extra runs only, in the order first taken
    let mut heard: Vec<(u16, Vec<ScoreOutcome>)> = Vec::new();
    for t in takes.iter().filter(|t| t.block.is_none()) {
        let site = t.st.tr.site;
        if heard.iter().any(|h| h.0 == site) {
            continue;
        }
        let mine: Vec<&Take> = takes.iter().filter(|k| k.block.is_none() && k.st.tr.site == site).collect();
        let got = lines(&runs, &mine, false);
        if !got.is_empty() {
            heard.push((site, got));
        }
    }
    Recorded { scores: kept.into_iter().map(|k| k.rec).collect(), outcomes, heard }
}

/// What [`record`] gives.
#[derive(Debug, Default)]
pub struct Recorded {
    /// The records, in the list's order.
    pub scores: Vec<ScoreRecord>,
    /// The candidates' outcomes.
    pub outcomes: Vec<Outcome>,
    /// The `OUT`s heard in the extra runs only, each with its outcomes.
    pub heard: Vec<(u16, Vec<ScoreOutcome>)>,
}

/// Whether a score from run `r` with stretch `st` has room beside those `kept`: at most
/// [`SND_SCORES_MAX`], and none whose stretch plays with another's in the same run. Err: why
/// not, and whether it plays with one.
fn room(runs: &[Run], kept: &[Kept], r: usize, st: &Stretch) -> Result<(), (String, bool)> {
    let span = st.span();
    if let Some(k) = kept.iter().find(|k| k.run == r && k.st.span().start < span.end && span.start < k.st.span().end) {
        return Err((format!("its stretch plays with {}", whose(runs, k)), true));
    }
    if kept.len() >= SND_SCORES_MAX {
        return Err((format!("only {SND_SCORES_MAX} scores a game: the longest first"), false));
    }
    Ok(())
}

/// `f(0)` to `f(n - 1)`, each in a thread of its own.
fn par<T: Send>(n: usize, f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    std::thread::scope(|s| {
        let threads: Vec<_> = (0..n).map(|i| s.spawn(move || f(i))).collect();
        threads.into_iter().map(|t| t.join().unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
    })
}

fn over_bytes() -> String {
    format!("the game's scores would take over {} KB: the longest first", SCORES_BYTES_MAX / 1024)
}

/// `the score of the OUT at 8951h (key 3 once)`.
fn whose(runs: &[Run], k: &Kept) -> String {
    format!("the score of the OUT at {:04X}h{}", k.rec.site, runs[k.run].suffix())
}

/// One score from run `r` added to those `kept` (it has [`room`]), verified (see the module
/// docs): it, or why not.
#[allow(clippy::too_many_arguments)]
fn keep_one(g: &Game, runs: &[Run], kept: &[Kept], r: usize, mut st: Stretch, mut rec: ScoreRecord, mut notes: usize) -> Result<Kept, String> {
    let mut shortened = None;
    let used: usize = kept.iter().map(|k| k.sum.bytes).sum();
    if used + bytes(&rec) > SCORES_BYTES_MAX {
        if !kept.is_empty() || rec.loop_t > 0 {
            return Err(over_bytes());
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
    // its run first, then the runs of the scores kept before in which its OUT sounds (where it
    // never sounds, it never arms: those runs play as they did)
    let mut order = vec![r];
    for k in kept {
        let sounds = || runs[k.run].tap.frames.iter().any(|(edges, _)| edges.iter().any(|e| e.pc == rec.site));
        if !order.contains(&k.run) && sounds() {
            order.push(k.run);
        }
    }
    let mut retried = false;
    loop {
        let mut all: Vec<ScoreRecord> = kept.iter().map(|k| k.rec.clone()).collect();
        all.push(rec.clone());
        let l = encode_list_scores(g.entries, g.roles, &all);
        // its run, then the others at once
        let mine = replay(g.load, runs[r].inputs, &l, g.builtins);
        let Some((dec, table)) = &mine else {
            return Err("method B's decoder refuses its record".into());
        };
        match check(&runs[r].tap, dec, *table, kept.len(), &st, rec.loop_t > 0) {
            Ok(()) => {}
            Err(Fail::Share(why)) => return Err(why),
            Err(Fail::Held(f, why)) => {
                let lead = usize::from(st.tr.t0 != 0);
                let held = (f - st.span().start).saturating_sub(lead);
                if retried || held < SCORE_FRAMES as usize || rec.loop_t > 0 {
                    return Err(why);
                }
                retried = true;
                (rec, notes) = cut(&rec, held);
                st.buckets = held;
                shortened = Some(format!("to the {held} frames method B followed it in"));
                continue;
            }
        }
        let others = par(order.len() - 1, &|i| replay(g.load, runs[order[i + 1]].inputs, &l, g.builtins));
        // the scores kept before still play in their runs
        for (&q, got) in order.iter().zip(std::iter::once(mine).chain(others)) {
            let Some((dec, table)) = got else {
                return Err("method B's decoder refuses its record".into());
            };
            if let Some(k) = kept.iter().enumerate().find(|(i, k)| k.run == q && check(&runs[q].tap, &dec, table, *i, &k.st, k.rec.loop_t > 0).is_err()).map(|(_, k)| k) {
                return Err(format!("it keeps {} from playing", whose(runs, k)));
            }
        }
        let sum = summary(&st, &rec, notes, shortened);
        return Ok(Kept { run: r, st, rec, sum });
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
        ev.iter().map(|e| (u64::from(e.t) / 64, e.voice, e.on)).collect()
    }

    /// [`score_events`] with no loop, `buckets` long: (events, voices, roles, notes).
    fn plain(notes: &[Note], buckets: u64) -> (Vec<ScoreEvent>, u8, [u8; SND_TONAL], usize) {
        let ev = score_events(notes, buckets * UNITS * 64, 0);
        assert!(ev.looped.is_empty());
        (ev.intro, ev.voices, ev.roles, ev.notes)
    }

    #[test]
    fn events_from_notes() {
        // a high line with a legato change and a rest, a low note under it; the transcriber's
        // voice 2 moves to voice 1 (two notes at most sound at once)
        let notes = [note(0, 0, 100, 2000), note(2, 10, 300, 8000), note(0, 100, 200, 1800), note(0, 250, 400, 1600)];
        let (ev, voices, roles, n) = plain(&notes, 1);
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
        let (ev, _, _, n) = plain(&[note(0, 0, 1092, 2000), note(1, 500, 2000, 4000), note(1, 1092, 1500, 3000)], 1);
        assert_eq!(n, 2);
        assert_eq!(timed(&ev), [(0, 0, Some((2000, 100))), (500, 1, Some((4000, 100)))]);
        // three voices: low bass, middle pad, high lead; a lone voice the lead
        let (_, voices, roles, _) = plain(&[note(0, 0, 50, 1000), note(1, 0, 50, 9000), note(2, 0, 50, 3000)], 1);
        assert_eq!((voices, roles), (3, [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_PAD]));
        assert_eq!(plain(&[note(1, 5, 50, 1000)], 1).2, [SND_ROLE_LEAD, 0, 0]);
    }

    #[test]
    fn events_of_an_intro_and_a_loop() {
        // the intro 1000 units, the loop 2000 from there: a note across the intro's end is cut
        // there and restated at the loop's start; one across the loop's end is cut there; one
        // after the loop's first pass is left out; times from each section's start
        let notes = [note(0, 0, 400, 2000), note(1, 900, 1300, 4000), note(0, 1000, 1500, 1800), note(0, 2800, 3200, 1600), note(1, 3000, 3100, 3000)];
        let ev = score_events(&notes, 1000 * 64, 2000 * 64);
        assert_eq!((ev.voices, ev.notes), (2, 5));
        assert_eq!(timed(&ev.intro), [(0, 0, Some((2000, 100))), (400, 0, None), (900, 1, Some((4000, 100)))]);
        assert_eq!(timed(&ev.looped), [(0, 0, Some((1800, 100))), (0, 1, Some((4000, 100))), (300, 1, None), (500, 0, None), (1800, 0, Some((1600, 100)))]);
        // a section's length not a whole unit: the last unit inside it holds an event, none past
        let ev = score_events(&[note(0, 0, 2000, 2000), note(1, 999, 2000, 3000)], 999 * 64 + 10, 500 * 64);
        assert_eq!(timed(&ev.intro), [(0, 0, Some((2000, 100))), (999, 1, Some((3000, 100)))]);
        assert_eq!(timed(&ev.looped), [(0, 0, Some((2000, 100))), (0, 1, Some((3000, 100)))]);
    }

    /// Speaker edges of a tune at `out`: each note (half period in T, its length in T) a square
    /// wave from its start, the notes one after another from `from`; `jitter` adds (edge index x
    /// 7) mod `jitter` T to each edge, as contention does.
    fn square(notes: &[(u64, u64)], from: u64, jitter: u64) -> Vec<Edge> {
        let mut out = Vec::new();
        let mut t = from;
        for &(half, len) in notes {
            let mut level = 0;
            let mut x = t;
            while x < t + len {
                level ^= 2;
                let j = if jitter > 0 { (out.len() as u64 * 7) % jitter } else { 0 };
                out.push(Edge { t: x + j, pc: 0x8000, level });
                x += half;
            }
            t += len;
        }
        out
    }

    const F: u64 = ZX_FRAME_TSTATES as u64;

    #[test]
    fn finds_the_loop_of_a_tune() {
        // an intro of three notes, then a phrase of five, four times over, cut mid-pass
        let intro = [(900, 20 * F + 123), (1200, 15 * F + 7), (1500, 30 * F)];
        let phrase = [(600, 25 * F), (700, 10 * F + 500), (800, 40 * F), (650, 12 * F + 1), (1000, 33 * F)];
        let intro_t: u64 = intro.iter().map(|n| n.1).sum();
        let loop_t: u64 = phrase.iter().map(|n| n.1).sum();
        let mut notes = intro.to_vec();
        for _ in 0..4 {
            notes.extend(phrase);
        }
        for jitter in [0, 5] {
            let e: Vec<Edge> = square(&notes, 0, jitter).into_iter().filter(|e| e.t < intro_t + 3 * loop_t + 50 * F).collect();
            let len_t = intro_t + 3 * loop_t + 60 * F;
            let lp = find_loop(&e, len_t).expect("a loop");
            assert_eq!(u64::from(lp.intro_t), intro_t + if jitter > 0 { (e.iter().position(|x| x.t >= intro_t).unwrap() as u64 * 7) % jitter } else { 0 });
            assert!(u64::from(lp.loop_t).abs_diff(loop_t) <= jitter, "{lp:?} {loop_t}");
            // under two whole passes: no loop
            assert_eq!(find_loop(&e, intro_t + 2 * loop_t - F), None);
        }
        // a tune that never repeats
        let e = square(&intro.iter().chain(&phrase).copied().cycle().take(40).enumerate().map(|(i, n)| (n.0 + i as u64, n.1)).collect::<Vec<_>>(), 0, 0);
        assert_eq!(find_loop(&e, e.last().unwrap().t + F), None);
    }

    #[test]
    fn a_cut_score_keeps_its_first_buckets() {
        let (events, voices, roles, _) = plain(&[note(0, 0, 1000, 2000), note(0, 1500, 2500, 1000)], 3);
        let r = ScoreRecord {
            site: 0x8000,
            quiet: 25,
            code: 1,
            roles,
            voices,
            intro_t: 3 * UNITS as u32 * 64,
            loop_t: 0,
            intro_fp: vec![9, 8, 7],
            loop_fp: vec![],
            intro_events: events,
            loop_events: vec![],
        };
        let (c, n) = cut(&r, 2);
        assert_eq!((c.intro_fp.as_slice(), n), (&[9u8, 8][..], 2));
        assert_eq!(timed(&c.intro_events), [(0, 0, Some((2000, 100))), (1000, 0, None), (1500, 0, Some((1000, 100)))]);
        assert!(bytes(&c) < bytes(&r) && bytes(&c).is_multiple_of(4));
    }
}
