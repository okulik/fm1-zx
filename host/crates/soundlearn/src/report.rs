// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the learner says about a game ([`learn`](crate::learn::learn)): the one-line summary ([`line`](fn@line)) and the
//! details, one block per routine ([`block`]).
//!
//! The summary is one of:
//!
//! - `music learned (1 routine, heard, 2 channels)`: the routines learned; heard, code-only or
//!   both; `observed` when they were all learned by observation (`1 observed` when some were);
//!   the most channels of any of them; `; 1 score` after them when scores were recorded too
//!   ([`crate::scores`]);
//! - `music recorded (1 score, 3 voices)`: no routine learned, the scores recorded; the most
//!   voices of any of them;
//! - `no music learned (<reason>)`: the first routine's refusal, heard routines first (the one
//!   heard in the most frames first), then those found in the code only that the reader could
//!   read;
//! - `no music learned (no speaker routine found)`: nothing heard, and no routine in the code the
//!   reader could read.
//!
//! Each ends with `; 2 shadow scores` inside its brackets when shadow scores were recorded.
//!
//! A block gives the speaker `OUT` the routine was read from (heard, or found in the code only),
//! the routine's span, family and channels, each channel's half period as a formula of its
//! inputs, the gates' figures (fit and replay), any drum sites, and the outcome: learned, or the
//! reason it was left out. A learned routine's block, and the block of a built-in routine's `OUT`
//! heard in the most frames (one refused as the built-in entry first), then give each channel's stored role and where it came from
//! ([`ChannelRole`]): `roles: channel 0 bass (analysis), channel 1 lead (override)`.
//!
//! A heard routine its code did not give, observed instead ([`Observation`]), says so after its
//! code's figures: its code's refusal (`left out: ...`, or `from its code: ...` when the
//! observation learned it), the routine observed with its `OUT`s and pass, each channel's input,
//! mapping and score (and any channel dropped), the replay of its entry, and then
//! `learned by observation: 3 channels` or `observation: <reason>`.
//!
//! A routine a score may be recorded for ends its block with its scores
//! (`recorded score: 169.5 s from frame 506 (intro 4.2 s, loop 41.6 s), 3 voices, 1074 notes,
//! 4.1 KB (score 8951-1a2b3c4d)`: its stretch's length, its trigger's frame, its intro and loop
//! when its music loops, its voices, the notes it stores and its bytes, `, shortened ...` when it
//! was cut, and its key ([`score_key`](crate::entry::score_key)), which names its folder of
//! arranged music in the personal pack), its shadow scores (`shadow score: ...` and, after the
//! key, `; packed only with a chosen take`) and its
//! stretches left out (`no score: <reason>`, `no shadow score: <reason>`), in the order of the runs and then of the frames,
//! each from a run not the learner's naming it (`recorded score (key 3 once): ...`), the same
//! reasons once (`... (3 stretches)`); or, with none, `no score: <reason>`. An `OUT` heard in
//! the extra runs only ([`crate::scores`]) gives its scores and its stretches left out (but
//! those method B's table claims, or that play with a score kept) in its block if it has one,
//! else in one of its own after the others ([`heard_only_block`]: `OUT at 9A12h, heard only in
//! other runs`).

use crate::fit::FitResult;
use crate::gate::ReplayStats;
use crate::model::{ChannelModel, EngineModel, Family, Input, Refusal, Term};
use crate::observe::{Mapping, ObservedChannel, ObservedModel};
use crate::scores::ScoreOutcome;
use zx_sys::{SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD};

/// One routine the learner looked at.
#[derive(Clone, Debug)]
pub struct Routine {
    /// The speaker `OUT` it was read from.
    pub site: u16,
    /// Heard: frames with its edges, and its edges; None: found in the code only.
    pub heard: Option<(u32, u32)>,
    pub model: Option<EngineModel>,
    pub fit: Option<Result<FitResult, String>>,
    pub replay: Option<ReplayStats>,
    /// Drum `OUT`s added to its entry.
    pub drums: Vec<u16>,
    /// Ok: learned (from its code, or by observation); Err: why not (the code path's reason when
    /// an observation was tried too: that one's is in `observation`).
    pub outcome: Result<(), String>,
    /// Per channel, its stored role (empty: a routine with none: neither learned nor built-in).
    pub roles: Vec<ChannelRole>,
    /// The observation tried when its code did not give the routine.
    pub observation: Option<Observation>,
    /// Its recorded scores ([`crate::scores`]) and its stretches left out, or why it has none;
    /// empty: not a routine for one.
    pub scores: Vec<ScoreOutcome>,
}

/// A heard routine observed ([`crate::observe`]) because its code did not give it.
#[derive(Clone, Debug)]
pub struct Observation {
    /// The code path's refusal.
    pub code: String,
    /// What the observation found (None: it found no routine, or was not run).
    pub model: Option<ObservedModel>,
    /// Channels found but left out: their input does not read the same at the routine's `OUT`s
    /// (the share of the frames, of those two or more of them sounded in, in which it did).
    pub dropped: Vec<(ObservedChannel, f64)>,
    /// The replay through method B with its entry.
    pub replay: Option<ReplayStats>,
    /// Ok: learned; Err: why not.
    pub outcome: Result<(), String>,
}

/// A channel's stored role (`SND_ROLE_*`) and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelRole {
    /// The analysis of the run ranked it ([`crate::roles`]).
    Analysis(u8),
    /// The owner's override (`[game.instruments]`).
    Override(u8),
    /// None: the channel never sounded in the routine's frames.
    NotHeard,
    /// None: too little music heard to rank the channels.
    TooLittle,
    /// None: the game's list already holds 8 entries (a built-in routine's roles take one).
    NoRoom,
}

impl ChannelRole {
    /// The role stored (0: none).
    pub fn role(self) -> u8 {
        match self {
            ChannelRole::Analysis(r) | ChannelRole::Override(r) => r,
            _ => 0,
        }
    }
}

/// The roles a channel can be given, by name (the report's and `[game.instruments]`' names).
pub const ROLES: [(&str, u8); 3] = [("bass", SND_ROLE_BASS), ("lead", SND_ROLE_LEAD), ("pad", SND_ROLE_PAD)];

/// A role's name ([`ROLES`]; "none" for any other value).
pub fn role_name(role: u8) -> &'static str {
    ROLES.iter().find(|r| r.1 == role).map_or("none", |r| r.0)
}

/// The roles line of a block (none for no roles).
fn roles_line(roles: &[ChannelRole]) -> String {
    if roles.is_empty() {
        return String::new();
    }
    let each: Vec<String> = roles
        .iter()
        .enumerate()
        .map(|(c, r)| match *r {
            ChannelRole::Analysis(x) => format!("channel {c} {} (analysis)", role_name(x)),
            ChannelRole::Override(x) => format!("channel {c} {} (override)", role_name(x)),
            ChannelRole::NotHeard => format!("channel {c} by the live rule (not heard)"),
            ChannelRole::TooLittle => format!("channel {c} by the live rule (too little music heard to rank)"),
            ChannelRole::NoRoom => format!("channel {c} by the live rule (no room left in the game's list)"),
        })
        .collect();
    format!("    roles: {}\n", each.join(", "))
}

/// A count and a noun, the noun plural unless the count is 1.
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The summary line: `learned` holds each learned routine's (heard, observed, channels),
/// `scores` each recorded score's voices, `shadows` the shadow scores recorded, `refused` the
/// first refusal's text ([`reason`]).
pub fn line(learned: &[(bool, bool, usize)], scores: &[u8], shadows: usize, refused: Option<&str>) -> String {
    let voices = scores.iter().copied().max().unwrap_or(0);
    let shadowed = if shadows == 0 { String::new() } else { format!("; {}", count(shadows, "shadow score")) };
    if learned.is_empty() && !scores.is_empty() {
        return format!("music recorded ({}, {}{shadowed})", count(scores.len(), "score"), count(usize::from(voices), "voice"));
    }
    if learned.is_empty() {
        return format!("no music learned ({}{shadowed})", refused.unwrap_or("no speaker routine found"));
    }
    let heard = learned.iter().filter(|r| r.0).count();
    let how = match heard {
        0 => "code-only",
        h if h == learned.len() => "heard",
        _ => "both",
    };
    let observed = match learned.iter().filter(|r| r.1).count() {
        0 => String::new(),
        o if o == learned.len() => ", observed".into(),
        o => format!(", {o} observed"),
    };
    let channels = learned.iter().map(|r| r.2).max().unwrap_or(0);
    let recorded = if scores.is_empty() { String::new() } else { format!("; {}", count(scores.len(), "score")) };
    format!("music learned ({}, {how}{observed}, {}{recorded}{shadowed})", count(learned.len(), "routine"), count(channels, "channel"))
}

/// A score's line of a block.
fn score_line(score: &ScoreOutcome) -> String {
    let run = score.run.as_ref().map_or(String::new(), |r| format!(" ({r})"));
    let what = if score.shadow { "shadow score" } else { "score" };
    match &score.got {
        Ok(s) => {
            let shortened = s.shortened.as_ref().map_or(String::new(), |w| format!(", shortened {w}"));
            let looped = s.looped.map_or(String::new(), |(intro, pass)| format!(" (intro {intro:.1} s, loop {pass:.1} s)"));
            let (head, tail) = if s.shadow { ("shadow score", SHADOW_TAIL) } else { ("recorded score", "") };
            format!(
                "    {head}{run}: {:.1} s from frame {}{looped}, {}, {}, {:.1} KB{shortened} (score {}{tail})\n",
                s.seconds,
                s.frame,
                count(usize::from(s.voices), "voice"),
                count(s.notes, "note"),
                s.bytes as f64 / 1024.0,
                s.key
            )
        }
        Err(why) => format!("    no {what}{run}: {why}\n"),
    }
}

/// What a shadow score's line says after its key.
pub const SHADOW_TAIL: &str = "; packed only with a chosen take";

/// A refusal of the code reader.
pub fn refusal(r: &Refusal) -> String {
    match *r {
        Refusal::Undecoded(a) => format!("an instruction not decoded at {a:04X}h"),
        Refusal::ComputedJump(a) => format!("a computed jump at {a:04X}h"),
        Refusal::SelfModUnmodelled(a) => format!("code that rewrites itself at {a:04X}h"),
        Refusal::Unmodelled(w, a) => format!("{} at {a:04X}h", w.text()),
        Refusal::Ambiguous(a) => format!("one channel or two in unison, not settled by the code at {a:04X}h"),
        Refusal::NotSpeaker(a) => format!("not a speaker OUT in the memory read at {a:04X}h"),
    }
}

/// A routine's refusal for the summary line: a gate's reason names the routine's start (a code
/// reader's refusal names its own address); None if it was learned.
pub fn reason(r: &Routine) -> Option<String> {
    let why = r.outcome.as_ref().err()?;
    Some(match &r.model {
        Some(m) => format!("{why}, routine at {:04X}h", m.anchor),
        None => why.clone(),
    })
}

const REGS: [&str; 18] = ["A", "F", "B", "C", "D", "E", "H", "L", "BC", "DE", "HL", "IX", "IY", "SP", "A'", "BC'", "DE'", "HL'"];

fn reg_name(r: u8) -> &'static str {
    REGS.get(usize::from(r)).copied().unwrap_or("?")
}

/// An input as the formula shows it: memory as `(8034h)` (`word (8034h)` for 16 bits) or
/// `(IX+2)`, a register by name; `anchor` places the routine-relative ones.
pub(crate) fn input(x: &Input, anchor: u16) -> String {
    match *x {
        Input::Rel(o) => format!("({:04X}h)", anchor.wrapping_add(o)),
        Input::Rel16(o) => format!("word ({:04X}h)", anchor.wrapping_add(o)),
        Input::Abs(a) => format!("({a:04X}h)"),
        Input::Abs16(a) => format!("word ({a:04X}h)"),
        Input::Indexed(r, d) if d < 0 => format!("({}-{})", reg_name(r), i16::from(d).unsigned_abs()),
        Input::Indexed(r, d) => format!("({}+{d})", reg_name(r)),
        Input::Reg(r) => reg_name(r).into(),
        Input::RegHalf(r, hi) => format!("{}{}", reg_name(r), if hi { 'h' } else { 'l' }),
    }
}

/// A channel's half period in T-states, as a sum of its terms (an input in a product counts a
/// loop: 0 counts 256, 65,536 for 16 bits).
pub fn formula(ch: &ChannelModel, anchor: u16) -> String {
    let mut s = String::new();
    for t in &ch.half_period {
        let (neg, text) = match t {
            Term::Const(c) => (*c < 0, c.unsigned_abs().to_string()),
            Term::Lin { coef_t, x } => (*coef_t < 0, format!("{} x {}", coef_t.unsigned_abs(), input(x, anchor))),
            Term::Prod { coef_t, x, y } => (*coef_t < 0, format!("{} x {} x {}", coef_t.unsigned_abs(), input(x, anchor), input(y, anchor))),
            Term::Recip { num_t, x } => (*num_t < 0, format!("{} / {}", num_t.unsigned_abs(), input(x, anchor))),
        };
        s += match (s.is_empty(), neg) {
            (true, false) => "",
            (true, true) => "-",
            (false, false) => " + ",
            (false, true) => " - ",
        };
        s += &text;
    }
    if s.is_empty() {
        s.push('0');
    }
    s
}

fn family(f: Family) -> &'static str {
    match f {
        Family::Square => "square wave",
        Family::Flips => "flips (channels XORed)",
        Family::Slots => "time slots",
    }
}

/// A mapping as the report shows it: `counter x1`, `16-bit accumulator`.
pub(crate) fn mapping(m: Mapping) -> String {
    match m {
        Mapping::Counter { mult } => format!("counter x{mult}"),
        Mapping::Accumulator { bits } => format!("{bits}-bit accumulator"),
    }
}

/// A channel the observation found: its input, mapping and score.
fn observed_channel(c: &ObservedChannel, anchor: u16) -> String {
    format!("{}, {}, score {:.2}", input(&c.input, anchor), mapping(c.mapping), c.score)
}

/// The replay's line.
fn replay_line(p: &ReplayStats) -> String {
    let gate = if p.music_gate { "passed" } else { "failed" };
    format!("    replay: method B claims {} of the {} frames it sounds in; music gate {gate}\n", p.claimed, p.routine_frames)
}

/// The observation's lines, after the code's outcome.
fn observation_lines(o: &Observation) -> String {
    let mut s = String::new();
    if let Some(m) = &o.model {
        let outs: Vec<String> = m.sites.iter().map(|&off| format!("{:04X}h", m.anchor.wrapping_add(off))).collect();
        s += &format!(
            "    observed routine {:04X}h-{:04X}h: OUT {}, pass {:.1} T\n",
            m.anchor,
            m.anchor.wrapping_add(m.len).wrapping_sub(1),
            outs.join(" "),
            m.pass_t
        );
        let kept = m.channels.iter().filter(|c| o.dropped.iter().all(|d| d.0.input != c.input));
        for (c, ch) in kept.enumerate() {
            s += &format!("    channel {c}: {}\n", observed_channel(ch, m.anchor));
        }
        for (ch, share) in &o.dropped {
            s += &format!(
                "    channel dropped: {}: reads the same at the routine's OUTs in {:.0}% of the frames two of them sound in (at least 95%)\n",
                observed_channel(ch, m.anchor),
                100.0 * share
            );
        }
    }
    if let Some(p) = &o.replay {
        s += &replay_line(p);
    }
    s + &match &o.outcome {
        Ok(()) => {
            let n = o.model.as_ref().map_or(0, |m| m.channels.len() - o.dropped.len());
            format!("    learned by observation: {}\n", count(n, "channel"))
        }
        Err(why) => format!("    observation: {why}\n"),
    }
}

/// The routine's block of the details: indented lines, each ending in a line break.
pub fn block(r: &Routine) -> String {
    let mut s = match r.heard {
        Some((frames, edges)) => format!("  OUT at {:04X}h, heard in {} ({})\n", r.site, count(frames as usize, "frame"), count(edges as usize, "edge")),
        None if r.model.is_none() && r.observation.is_none() => {
            // a site found in the code that the reader could not read: one line
            let why = r.outcome.as_ref().err().map_or("", String::as_str);
            return format!("  OUT at {:04X}h, code only: {why}\n", r.site) + &r.scores.iter().map(score_line).collect::<String>();
        }
        None => format!("  OUT at {:04X}h, code only\n", r.site),
    };
    if let Some(m) = &r.model {
        let timing = if m.duration.is_some() { "the note's timing known" } else { "the note's timing not known" };
        let table = if m.table_walk { ", notes from a table" } else { "" };
        s += &format!(
            "    routine {:04X}h-{:04X}h: {}, {}, {timing}{table}\n",
            m.anchor,
            m.code_end.wrapping_sub(1),
            family(m.family),
            count(m.channels.len(), "channel")
        );
        for (c, ch) in m.channels.iter().enumerate() {
            let sites: Vec<String> = ch.sites.iter().map(|p| format!("{p:04X}h")).collect();
            let mut silent = ch.silent_if_zero.as_ref().map_or(String::new(), |x| format!(", silent when {} is 0", input(x, m.anchor)));
            if let Some((x, mask, set)) = &ch.silent_if_bit {
                silent += &format!(", silent when {} & {mask:02X}h is {}", input(x, m.anchor), if *set { "set" } else { "clear" });
            }
            s += &format!("    channel {c}, OUT {}: half period {} T{silent}\n", sites.join(" "), formula(ch, m.anchor));
        }
    }
    match &r.fit {
        Some(Ok(f)) => {
            s += &format!(
                "    fit: {:.1}% inliers, {} over {:.1} semitones, highest {:.0} Hz\n",
                100.0 * f.inliers,
                count(f.distinct_notes as usize, "note"),
                f.span_semitones,
                f.max_hz
            )
        }
        Some(Err(e)) => s += &format!("    fit: {e}\n"),
        None => {}
    }
    if let Some(p) = &r.replay {
        s += &replay_line(p);
    }
    for d in &r.drums {
        s += &format!("    drum: OUT at {d:04X}h\n");
    }
    s += &match (&r.observation, &r.outcome) {
        (Some(o), _) if o.outcome.is_ok() => format!("    from its code: {}\n{}", o.code, observation_lines(o)),
        (Some(o), _) => format!("    left out: {}\n{}", o.code, observation_lines(o)),
        (None, Ok(())) => "    learned\n".to_string(),
        (None, Err(why)) => format!("    left out: {why}\n"),
    };
    s + &roles_line(&r.roles) + &r.scores.iter().map(score_line).collect::<String>()
}

/// The block of an `OUT` heard in the scores' extra runs only: its scores and its stretches left
/// out.
pub fn heard_only_block(site: u16, scores: &[ScoreOutcome]) -> String {
    format!("  OUT at {site:04X}h, heard only in other runs\n") + &scores.iter().map(score_line).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::line;

    #[test]
    fn the_line_says_observed() {
        assert_eq!(line(&[(true, false, 1)], &[], 0, None), "music learned (1 routine, heard, 1 channel)");
        assert_eq!(line(&[(true, true, 3)], &[], 0, None), "music learned (1 routine, heard, observed, 3 channels)");
        assert_eq!(line(&[(true, true, 2), (false, false, 1)], &[], 0, None), "music learned (2 routines, both, 1 observed, 2 channels)");
        assert_eq!(line(&[], &[], 0, Some("a sweep, not music")), "no music learned (a sweep, not music)");
    }

    #[test]
    fn the_line_counts_scores() {
        assert_eq!(line(&[(true, true, 3)], &[2], 0, None), "music learned (1 routine, heard, observed, 3 channels; 1 score)");
        assert_eq!(line(&[], &[3], 0, Some("a sweep, not music")), "music recorded (1 score, 3 voices)");
        assert_eq!(line(&[], &[1, 2], 0, None), "music recorded (2 scores, 2 voices)");
        // the shadow scores counted after the rest
        assert_eq!(line(&[(true, false, 1)], &[], 1, None), "music learned (1 routine, heard, 1 channel; 1 shadow score)");
        assert_eq!(line(&[(true, false, 1)], &[2], 2, None), "music learned (1 routine, heard, 1 channel; 1 score; 2 shadow scores)");
        assert_eq!(line(&[], &[2], 1, None), "music recorded (1 score, 2 voices; 1 shadow score)");
        assert_eq!(line(&[], &[], 1, Some("built-in entry at 03B5h")), "no music learned (built-in entry at 03B5h; 1 shadow score)");
    }
}
