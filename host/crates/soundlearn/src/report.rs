// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the learner says about a game ([`learn`](crate::learn::learn)): the one-line summary ([`line`](fn@line)) and the
//! details, one block per routine ([`block`]).
//!
//! The summary is one of:
//!
//! - `music learned (1 routine, heard, 2 channels)`: the routines learned; heard, code-only or
//!   both; the most channels of any of them;
//! - `no music learned (<reason>)`: the first routine's refusal, heard routines first (the one
//!   heard in the most frames first), then those found in the code only that the reader could
//!   read;
//! - `no music learned (no speaker routine found)`: nothing heard, and no routine in the code the
//!   reader could read.
//!
//! A block gives the speaker `OUT` the routine was read from (heard, or found in the code only),
//! the routine's span, family and channels, each channel's half period as a formula of its
//! inputs, the gates' figures (fit and replay), any drum sites, and the outcome: learned, or the
//! reason it was left out.

use crate::fit::FitResult;
use crate::gate::ReplayStats;
use crate::model::{ChannelModel, EngineModel, Family, Input, Refusal, Term};

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
    /// Ok: learned; Err: why not.
    pub outcome: Result<(), String>,
}

/// A count and a noun, the noun plural unless the count is 1.
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The summary line: `learned` holds each learned routine's (heard, channels), `refused` the
/// first refusal's text ([`reason`]).
pub fn line(learned: &[(bool, usize)], refused: Option<&str>) -> String {
    if learned.is_empty() {
        return format!("no music learned ({})", refused.unwrap_or("no speaker routine found"));
    }
    let heard = learned.iter().filter(|r| r.0).count();
    let how = match heard {
        0 => "code-only",
        h if h == learned.len() => "heard",
        _ => "both",
    };
    let channels = learned.iter().map(|r| r.1).max().unwrap_or(0);
    format!("music learned ({}, {how}, {})", count(learned.len(), "routine"), count(channels, "channel"))
}

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
fn input(x: &Input, anchor: u16) -> String {
    match *x {
        Input::Rel(o) => format!("({:04X}h)", anchor.wrapping_add(o)),
        Input::Rel16(o) => format!("word ({:04X}h)", anchor.wrapping_add(o)),
        Input::Abs(a) => format!("({a:04X}h)"),
        Input::Abs16(a) => format!("word ({a:04X}h)"),
        Input::Indexed(r, d) if d < 0 => format!("({}-{})", reg_name(r), i16::from(d).unsigned_abs()),
        Input::Indexed(r, d) => format!("({}+{d})", reg_name(r)),
        Input::Reg(r) => reg_name(r).into(),
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

/// The routine's block of the details: indented lines, each ending in a line break.
pub fn block(r: &Routine) -> String {
    let mut s = match r.heard {
        Some((frames, edges)) => format!("  OUT at {:04X}h, heard in {} ({})\n", r.site, count(frames as usize, "frame"), count(edges as usize, "edge")),
        None if r.model.is_none() => {
            // a site found in the code that the reader could not read: one line
            let why = r.outcome.as_ref().err().map_or("", String::as_str);
            return format!("  OUT at {:04X}h, code only: {why}\n", r.site);
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
            let silent = ch.silent_if_zero.as_ref().map_or(String::new(), |x| format!(", silent when {} is 0", input(x, m.anchor)));
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
        let gate = if p.music_gate { "passed" } else { "failed" };
        s += &format!("    replay: method B claims {} of the {} frames it sounds in; music gate {gate}\n", p.claimed, p.routine_frames);
    }
    for d in &r.drums {
        s += &format!("    drum: OUT at {d:04X}h\n");
    }
    s += &match &r.outcome {
        Ok(()) => "    learned\n".to_string(),
        Err(why) => format!("    left out: {why}\n"),
    };
    s
}
