// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A score's music read exactly from its game's music engine, on the score's own timeline
//! (`fmzx-pack engine-notes`, for `tools/arrange-music engine`).
//!
//! The score's run is the one that recorded it ([`ScoreRuns`](crate::personal::ScoreRuns)), as
//! `music-preview` plays it, so the timelines agree: the game's music comes from the workspace's
//! sound cache ([`learn_music`]), the score's frame from the learner's details, and its T0 is the
//! first speaker edge from the score's `OUT` in that frame (the trigger edge). The run is searched
//! for a known engine ([`chipmusic::engine::locate_in_run`]) up to the score's end whose code lies
//! within [`NEAR`] bytes of the score's `OUT` (another engine the game holds is not the score's). Then the run
//! plays again with the engine's tap on from [`BEFORE`] frames before the score
//! ([`chipmusic::engine::read_run`]), and the notes and drum hits are put on the score's timeline:
//! T since T0, cut to `[0, intro_t + loop_t)` (a note sounding at T0 starts at 0), those under 64 T
//! left out. The same run feeds method B with the game's learned list: the share of the score's
//! sounding frames (frames with speaker edges) a built-in entry of the firmware plays says
//! whether a shadow score already plays exactly, live ([`LIVE_SHARE`]).
//!
//! # The notes file
//!
//! `game`, `key`, `shadow`, `clock_hz`, `intro_t` and `loop_t` (the score's), `engine` (its name),
//! `anchor` (where its signature matched, hex), `read` (what the tap read, in words), `pass_t` (T,
//! the sound loop's pass), `channels` (its tone channels), `rows` (rows read in the run),
//! `first_frame` and `t0` (the score's frame and its trigger edge's T in the run),
//! `builtin_share` (above) and `live` (a sentence when a built-in entry plays the shadow score
//! exactly, else ""). Then each `[[note]]` by start and channel: `channel` (0-based, the
//! engine's), `start_t` and `end_t` (T since T0), `period_t` (the full period in T, rounded),
//! `midi` (the pitch, 2 decimals), `level` (1-255) and `duty` (0-255 of a period high); and each
//! `[[drum]]` by start: `start_t`, `end_t` and `kind` (`kick`, `snare`, `hat`, `tom`, `tone`,
//! `noise`).

use crate::arrange::{Of, CLOCK_HZ};
use crate::personal::{cache_path, learn_music, machine};
use crate::workspace::{game_files, score_frame};
use chipmusic::engine::{locate_in_run, read_run, Hit, Reading, ENGINES};
use soundlearn::entry::ear_edges;
use soundlearn::{decode_scores, score_key, ScoreRecord};
use std::path::Path;
use zx_sys::{Machine, SndEngines, SND_DELAY_FRAMES, ZX_FRAME_TSTATES};

/// Bytes between the score's `OUT` and the engine's code within which it is the score's engine.
pub const NEAR: u16 = 0x200;
/// Frames before the score's the tap starts (a note sounding at T0 began before it).
pub const BEFORE: usize = 250;
/// The share of a shadow score's sounding frames a built-in entry must play for it to play
/// exactly already.
pub const LIVE_SHARE: f64 = 0.9;
/// Notes shorter than this on the timeline are left out (T).
const SHORTEST: u64 = 64;
const FT: u64 = ZX_FRAME_TSTATES as u64;

/// A note on the score's timeline (T since T0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedNote {
    pub channel: u8,
    pub start_t: u64,
    pub end_t: u64,
    pub period_t: f64,
    pub midi: f64,
    pub level: u8,
    pub duty: u8,
}

/// A drum hit on the score's timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedDrum {
    pub start_t: u64,
    pub end_t: u64,
    pub kind: &'static str,
}

/// A score's engine, read.
#[derive(Clone, Debug)]
pub struct Found {
    pub reading: Reading,
    pub notes: Vec<TimedNote>,
    pub drums: Vec<TimedDrum>,
    pub first_frame: usize,
    pub t0: u64,
    /// The share of its sounding frames a built-in entry plays (method B).
    pub builtin_share: f64,
    /// Why no take is needed: a built-in entry plays this shadow score exactly; None otherwise.
    pub live: Option<String>,
}

impl Found {
    /// "ntropic, 2 channels + drums".
    pub fn shape(&self) -> String {
        let drums = if self.drums.is_empty() { "" } else { " + drums" };
        format!("{}, {} channels{drums}", self.reading.engine, self.reading.channels)
    }
}

/// A score and what its engine gave.
#[derive(Clone, Debug)]
pub struct ScoreEngine {
    pub key: String,
    pub shadow: bool,
    pub intro_t: u32,
    pub loop_t: u32,
    /// Err: why no engine was read for it.
    pub found: Result<Found, String>,
}

/// Reads each score of game `game` of the workspace `all` (only `key` when given) from its
/// engine (see the module docs). Err: the workspace, the game or the score is not there.
pub fn read_scores(all: &Path, game: &str, key: Option<&str>) -> Result<Vec<ScoreEngine>, String> {
    if !all.is_dir() {
        return Err(format!("{}: no such folder", all.display()));
    }
    let g = game_files(all, game)?;
    let cache = all.join("sound-cache");
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let script = std::fs::read_to_string(&g.keys).ok();
    if !cache_path(&g.data, script.as_deref(), &g.instruments, &cache).is_file() {
        eprintln!("{game}: not in the sound cache: learning its music (this takes a while)");
    }
    let s = learn_music(&g.file, &g.data, &g.keys, &g.instruments, &cache);
    let scores = s.list.as_deref().map(decode_scores).transpose().map_err(|e| format!("{game}: its learned list: {e}"))?.unwrap_or_default();
    let all_scores: Vec<(Of, &ScoreRecord)> =
        scores.iter().enumerate().map(|(i, r)| (Of::Score(i), r)).chain(s.shadows.iter().enumerate().map(|(i, r)| (Of::Shadow(i), r))).collect();
    if let Some(k) = key {
        if !all_scores.iter().any(|(_, r)| score_key(r) == k) {
            let keys: Vec<String> = all_scores.iter().map(|(_, r)| score_key(r)).collect();
            let has = if keys.is_empty() { "it has none".to_string() } else { format!("its scores: {}", keys.join(", ")) };
            return Err(format!("game {game:?} has no score {k} ({has})"));
        }
    }
    let load = || machine(&g.file, &g.data).unwrap_or_else(|_| Machine::new());
    let mut out = Vec::new();
    for (of, rec) in all_scores {
        let k = score_key(rec);
        if key.is_some_and(|want| want != k) {
            continue;
        }
        let found = (|| {
            let inputs = s.runs.as_ref().and_then(|r| r.inputs(of)).ok_or("the run that recorded it is not known")?;
            let first = score_frame(&s.details, &k).ok_or("the learner's details do not say where it starts")?;
            read_one(&load, &*inputs, s.list.as_deref(), rec, first)
        })();
        out.push(ScoreEngine { key: k, shadow: matches!(of, Of::Shadow(_)), intro_t: rec.intro_t, loop_t: rec.loop_t, found });
    }
    Ok(out)
}

/// One score's engine read from its run (see the module docs).
fn read_one(load: &dyn Fn() -> Machine, inputs: &dyn Fn(u32) -> ([u8; 8], u8), list: Option<&[u8]>, rec: &ScoreRecord, first: usize) -> Result<Found, String> {
    let end_t = u64::from(rec.intro_t) + u64::from(rec.loop_t);
    let frames = first + end_t.div_ceil(FT) as usize + SND_DELAY_FRAMES as usize + 2;
    let names: Vec<&str> = ENGINES.iter().map(|e| e.name()).collect();
    let (engine, sites, _) = locate_in_run(load, inputs, frames, &|s| s.near(rec.site, NEAR)).map_err(|other| match other {
        Some((e, s)) => format!("its OUT at {:04X}h is not that of the {} engine found at {:04X}h, nor of another known engine", rec.site, e.name(), s.anchor),
        None => format!("no known engine in its run (looked for {})", names.join(", ")),
    })?;
    let builtins = SndEngines::new().entries().len();
    let mut method_b = list.and_then(|l| SndEngines::with_learned(l).ok()).unwrap_or_default();
    let delay = SND_DELAY_FRAMES as usize;
    let span = first..first + end_t.div_ceil(FT) as usize;
    let (mut sounding, mut claimed, mut sounds) = (0usize, 0usize, vec![false; frames]);
    let mut t0 = None;
    let reading = read_run(load, inputs, frames, engine, &sites, first.saturating_sub(BEFORE), &mut |k, m| {
        let level = m.edges_start_level();
        let edges = m.edges();
        sounds[k] = !edges.is_empty();
        if k == first {
            t0 = ear_edges(level, edges).find(|e| e.pc == rec.site).map(|e| k as u64 * FT + u64::from(e.tstate));
        }
        let fb = method_b.frame(m, edges, level);
        if k.checked_sub(delay).is_some_and(|d| span.contains(&d) && sounds[d]) {
            sounding += 1;
            if fb.confident != 0 && (1..=builtins).contains(&usize::from(fb.engine)) {
                claimed += 1;
            }
        }
    });
    let t0 = t0.ok_or_else(|| format!("no speaker edge from its OUT at {:04X}h in frame {first} of its run", rec.site))?;
    let notes: Vec<TimedNote> = reading
        .notes
        .iter()
        .filter_map(|n| {
            let (s, e) = on_timeline(n.start_t, n.end_t, t0, end_t)?;
            Some(TimedNote { channel: n.channel, start_t: s, end_t: e, period_t: n.period_t, midi: n.midi(), level: n.level, duty: n.duty })
        })
        .collect();
    let drums: Vec<TimedDrum> = reading
        .drums
        .iter()
        .filter_map(|h: &Hit| {
            // a hit's burst may be one OUT: it lasts at least its shortest
            let (s, e) = on_timeline(h.t, h.end_t.max(h.t + SHORTEST), t0, end_t)?;
            (h.t >= t0).then_some(TimedDrum { start_t: s, end_t: e, kind: h.kind })
        })
        .collect();
    if notes.is_empty() && drums.is_empty() {
        return Err(format!("the {} engine at {:04X}h plays nothing during the score", engine.name(), sites.anchor));
    }
    let builtin_share = if sounding == 0 { 0.0 } else { claimed as f64 / sounding as f64 };
    let live = (builtin_share >= LIVE_SHARE)
        .then(|| format!("a built-in entry of the firmware reads this engine and plays it live in {:.0}% of the score's frames", builtin_share * 100.0));
    Ok(Found { reading, notes, drums, first_frame: first, t0, builtin_share, live })
}

/// A span of the run (T) on the score's timeline: from T0, cut to `[0, end_t)`; None when
/// nothing of it (at least [`SHORTEST`]) is left.
fn on_timeline(start: u64, end: u64, t0: u64, end_t: u64) -> Option<(u64, u64)> {
    let s = start.saturating_sub(t0);
    let e = end.saturating_sub(t0).min(end_t);
    (end > t0 && s < end_t && e >= s + SHORTEST).then_some((s, e))
}

/// A TOML basic string.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// The notes file of a score's engine (see the module docs).
pub fn notes_toml(game: &str, se: &ScoreEngine, f: &Found) -> String {
    let r = &f.reading;
    let mut s = format!(
        "# A score's music read exactly from the game's music engine, for tools/arrange-music (written by fmzx-pack engine-notes).\n\
         game = {}\nkey = {}\nshadow = {}\nclock_hz = {CLOCK_HZ}\nintro_t = {}\nloop_t = {}\nengine = {}\nanchor = \"{:04X}\"\nread = {}\n\
         pass_t = {:.3}\nchannels = {}\nrows = {}\nfirst_frame = {}\nt0 = {}\nbuiltin_share = {:.3}\nlive = {}\n",
        quoted(game),
        quoted(&se.key),
        se.shadow,
        se.intro_t,
        se.loop_t,
        quoted(r.engine),
        r.sites.anchor,
        quoted(&r.what),
        r.pass_t,
        r.channels,
        r.rows.len(),
        f.first_frame,
        f.t0,
        f.builtin_share,
        quoted(f.live.as_deref().unwrap_or(""))
    );
    for n in &f.notes {
        s += &format!(
            "\n[[note]]\nchannel = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nmidi = {:.2}\nlevel = {}\nduty = {}\n",
            n.channel,
            n.start_t,
            n.end_t,
            n.period_t.round() as u64,
            n.midi,
            n.level,
            n.duty
        );
    }
    for d in &f.drums {
        s += &format!("\n[[drum]]\nstart_t = {}\nend_t = {}\nkind = {}\n", d.start_t, d.end_t, quoted(d.kind));
    }
    s
}

/// A score's line for `--info` and the command's summary: "ntropic at 9C98h, 2 channels + drums
/// (kick, noise): 419 notes, 289 drum hits", with "; already exact: ..." for one a built-in entry
/// plays, or why no engine was read.
pub fn summary(se: &ScoreEngine) -> String {
    match &se.found {
        Err(why) => why.clone(),
        Ok(f) => {
            let mut kinds: Vec<&str> = f.drums.iter().map(|d| d.kind).collect();
            kinds.sort_unstable();
            kinds.dedup();
            let kinds = if kinds.is_empty() { String::new() } else { format!(" ({})", kinds.join(", ")) };
            let mut s = format!(
                "{} at {:04X}h, {}{kinds}: {} notes, {} drum hits",
                f.reading.engine,
                f.reading.sites.anchor,
                f.shape().split_once(", ").map_or("", |x| x.1),
                f.notes.len(),
                f.drums.len()
            );
            if let Some(l) = &f.live {
                s += &format!("; already exact: {l}");
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_on_the_timeline() {
        assert_eq!(on_timeline(100, 1000, 0, 10_000), Some((100, 1000)));
        assert_eq!(on_timeline(100, 1000, 500, 10_000), Some((0, 500)), "sounding at T0: from 0");
        assert_eq!(on_timeline(100, 1000, 2000, 10_000), None, "over before T0");
        assert_eq!(on_timeline(9_000, 12_000, 0, 10_000), Some((9_000, 10_000)), "cut at the end");
        assert_eq!(on_timeline(9_990, 12_000, 0, 10_000), None, "under 64 T left");
    }
}
