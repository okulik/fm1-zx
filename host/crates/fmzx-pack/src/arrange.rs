// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Arranged music: the personal pack's recorded scores written out for `tools/arrange-music`,
//! and the takes the owner chose put into the pack in place of the scores' notes.
//!
//! # The scores, written out
//!
//! Each recorded score (kind-2 record) of each packed game, and each of its shadow scores (music a
//! learned or built-in entry plays, [`soundlearn::GameSound::shadows`]), is written to
//! `<out>/arrange/<game-id>/<key>/score.toml` (`<key>`: [`score_key`]): the game's id and name,
//! the key, `clock_hz`, the record's `intro_t` and `loop_t`, its tonal `voices` and their
//! `roles` by name, `shadow = true` for a shadow score, and its notes on one timeline, the intro and then one pass of the loop (a
//! loop note at `intro_t` plus its time in the loop), each `[[note]]` with its `voice` (0-2),
//! `start_t` and `end_t` (T-states since T0), `period_t` (the full period in T-states) and
//! `level` (1-255), by start and then voice. A note runs from its note on to the voice's next
//! note on or note off, or to its section's end (where the device releases or restates it). A
//! file is written only when its text changes; the folder's other files (the takes, the renders)
//! are left alone, and the folder of a score the pack no longer holds stays as it is.
//!
//! # The takes chosen
//!
//! `<out>/arrange/choices.toml` holds a `[choices]` table, `"<game-id>/<key>" = <take>`: a number
//! for an AI take (`take-<n>`, 0 the original), `"engine-<n>"` for an engine take (the game's own
//! music read exactly from its engine), `"chip-<n>"` for a C64 take or `"ay-<n>"` for a
//! Spectrum 128 take ([`Choice`]; a value that is none of these, or a file that does not read,
//! fails the run, naming the file and the key). A chosen AI take is read from `<key>/take-<n>/arranged.toml` (the header of `score.toml`
//! and `take`, `generator`, and notes on 4 voices: 0 the lead, 1 the bass, 2 the pad, 3 the drum
//! with `period_t = 0`, a hit from `start_t` to `end_t`) and checked ([`arrange`]): its key and
//! timeline the score's, every note within the timeline, on voices 0-3, with no two on one voice
//! overlapping, a tonal note's period 64 to 16,777,215 T, a drum's 0, levels 1-255. Its notes
//! become the record's events as the learner builds a score's
//! ([`soundlearn::scores::score_events`]): times rounded to 64 T, the notes cut at the intro's end
//! and the loop's, each tonal voice sounding at the loop's start restated there, a note off at a
//! note's end unless the voice's next note starts there (or the note reaches its section's end);
//! voices 0-2 tonal with the roles lead, bass and pad, voice 3 the drum's hits and offs, a drum
//! off at least 64 T before the next hit (shortened where needed; a drum note is never restated:
//! the device strikes the drum anew at each pass). A frame (a bucket of 69,888 T on its section's
//! grid) holds at most [`SND_EVENTS`] events: past that, drum notes are dropped first, then pad
//! notes (the latest in the frame first), and the take says how many; a frame over it with the
//! lead and bass alone refuses the take. The record keeps the score's trigger, code, quiet and
//! fingerprints, so it plays in step as before.
//!
//! A chosen engine take (`<key>/engine-<n>/arranged.toml`, written by `tools/arrange-music
//! engine`) is on the score's timeline as an AI take is, `take` its number or its folder's name,
//! and becomes the record's events the same way; its `source` (the engine and what was read)
//! names it in the report.
//!
//! A chosen C64 take (`<key>/chip-<n>/arranged.toml`, written by `tools/arrange-music chip`) or
//! Spectrum 128 take (`<key>/ay-<n>/arranged.toml`, written by `tools/arrange-music ay`) has
//! the same header and notes, `take` its number or its folder's name, and its own timeline:
//! `timeline = "own"`, `own_intro_t` and `own_loop_t` (T; 0: no loop, the notes end at
//! `own_intro_t`), and `source` (where the tune comes from). It is checked the same way against
//! its own timeline (the key and the score's header still the score's; `own_loop_t` 0 or at least
//! a frame; the two together 1 T to a score's longest, [`SND_SCORE_BUCKETS_MAX`] frames, about 4
//! minutes), and becomes a
//! replacement record ([`ScoreRecord::replacement`], docs/pack-format.md): the score's trigger,
//! code, quiet and fingerprints, which method B follows as before, precedence set, and its events
//! built as above on its own sections ([`ScoreRecord::event_lengths`]): its intro from T0 and its
//! loop, played round from T0 + `own_intro_t` for as long as the score is followed. Its frames
//! are counted on its own sections' grids.
//!
//! Then ([`apply`]) the game's scores must stay within [`SCORES_BYTES_MAX`], method B's decoder
//! must take the list, and the replay must verify the score ([`soundlearn::scores::verify`]).
//! A take that fails any of these leaves the original, with the reason. A shadow score is packed
//! only with a take chosen for it (an AI take, a C64 take or a Spectrum 128 take): the take's record, its precedence
//! set (method B gives it the frames the table claims too), is added after the game's scores,
//! which must then number at most [`SND_SCORES_MAX`](zx_sys::SND_SCORES_MAX) and stay within
//! [`SCORES_BYTES_MAX`], and is verified the same way; else the routine plays the music live, as
//! before.

use serde::Deserialize;
use soundlearn::scores::{record_bytes, SCORES_BYTES_MAX};
use soundlearn::{append_score, decode_scores, replace_scores, score_key, ScoreEvent, ScoreRecord, SCORE_DRUM};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use zx_sys::{SndEngines, SND_EVENTS, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD, SND_SCORE_BUCKETS_MAX, SND_TONAL, ZX_FRAME_TSTATES};

/// The arranged music's folder, in the output folder.
pub const ARRANGE: &str = "arrange";
/// The owner's choices, in [`ARRANGE`].
pub const CHOICES: &str = "choices.toml";
/// A score written out, in its folder.
pub const SCORE_FILE: &str = "score.toml";
/// A take, in its `take-<n>` folder.
pub const TAKE_FILE: &str = "arranged.toml";
/// The Spectrum's clock: the T-states of a second.
pub const CLOCK_HZ: u32 = 3_500_000;
/// A take's tonal voices' roles: 0 the lead, 1 the bass, 2 the pad.
const TAKE_ROLES: [u8; SND_TONAL] = [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_PAD];
/// 64-T units a frame (69,888 T).
const FRAME_UNITS: u64 = ZX_FRAME_TSTATES as u64 / 64;
/// The longest period a note may have (the record's 24 bits).
const PERIOD_MAX: i64 = 0xFF_FFFF;
/// A replacement's event sections together, at most (T): the decoder's limit.
const OWN_MAX: i64 = SND_SCORE_BUCKETS_MAX as i64 * ZX_FRAME_TSTATES as i64;
/// A section's most events (the record's 16-bit counts).
const EVENTS_MAX: usize = u16::MAX as usize;

/// A note of a score's timeline: its voice (0-2 tonal, 3 the drum), its start and end in T
/// since T0, its full period in T (0 for the drum) and its level (1-255).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub voice: u8,
    pub start_t: u64,
    pub end_t: u64,
    pub period_t: u32,
    pub level: u8,
}

/// A stored role's name in the files: `lead`, `bass`, `pad`, or `""` for none.
pub fn role_name(role: u8) -> &'static str {
    match role {
        SND_ROLE_LEAD => "lead",
        SND_ROLE_BASS => "bass",
        SND_ROLE_PAD => "pad",
        _ => "",
    }
}

/// The tonal notes of a record on one timeline (see the module docs; a replacement's on its
/// events' own), by start and then voice.
pub fn timeline(rec: &ScoreRecord) -> Vec<Note> {
    let mut out = Vec::new();
    let (intro_t, loop_t) = rec.event_lengths();
    let intro_t = u64::from(intro_t);
    for (events, from, len) in [(&rec.intro_events, 0, intro_t), (&rec.loop_events, intro_t, u64::from(loop_t))] {
        let mut open: [Option<(u64, u32, u8)>; SND_TONAL] = [None; SND_TONAL];
        let close = |out: &mut Vec<Note>, v: usize, o: Option<(u64, u32, u8)>, end: u64| {
            if let Some((start, period_t, level)) = o.filter(|o| end > o.0) {
                out.push(Note { voice: v as u8, start_t: start, end_t: end, period_t, level });
            }
        };
        for e in events.iter().filter(|e| usize::from(e.voice) < SND_TONAL) {
            let (v, t) = (usize::from(e.voice), from + u64::from(e.t));
            close(&mut out, v, open[v].take(), t);
            open[v] = e.on.map(|(p, l)| (t, p, l));
        }
        for (v, o) in open.into_iter().enumerate() {
            close(&mut out, v, o, from + len);
        }
    }
    out.sort_by_key(|n| (n.start_t, n.voice));
    out
}

/// A TOML basic string.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// The text of a score's `score.toml` (see the module docs): `game` its game's id, `name` the
/// game's name; a shadow score's (precedence set) says `shadow = true`.
pub fn score_toml(game: &str, name: &str, rec: &ScoreRecord) -> String {
    let roles: Vec<String> = rec.roles[..usize::from(rec.voices).min(SND_TONAL)].iter().map(|&r| quoted(role_name(r))).collect();
    let mut s = format!(
        "# A recorded score's notes, for tools/arrange-music (written by fmzx-pack personal).\n\
         game = {}\nname = {}\nkey = {}\nclock_hz = {CLOCK_HZ}\nintro_t = {}\nloop_t = {}\nvoices = {}\nroles = [{}]\n",
        quoted(game),
        quoted(name),
        quoted(&score_key(rec)),
        rec.intro_t,
        rec.loop_t,
        rec.voices,
        roles.join(", ")
    );
    if rec.precedence {
        s += "# A shadow score: music a learned or built-in routine plays live. It is packed only with a\n\
              # take chosen for it, which then plays in place of the live version.\n\
              shadow = true\n";
    }
    for n in timeline(rec) {
        s += &format!(
            "\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = {}\n",
            n.voice, n.start_t, n.end_t, n.period_t, n.level
        );
    }
    s
}

/// Writes each of a game's `scores` to `<dir>/<game>/<key>/score.toml` (`dir`: the arranged
/// music's folder), each only when its text changes.
pub fn export(dir: &Path, game: &str, name: &str, scores: &[ScoreRecord]) -> Result<(), String> {
    for rec in scores {
        let folder = dir.join(game).join(score_key(rec));
        let path = folder.join(SCORE_FILE);
        let text = score_toml(game, name, rec);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// A TOML error as one line: `line 3: <what>`.
fn toml_error(text: &str, e: &toml::de::Error) -> String {
    let at = e.span().map(|s| text[..s.start.min(text.len())].lines().count().max(1));
    let what = e.message().trim();
    at.map_or_else(|| what.to_string(), |l| format!("line {l}: {what}"))
}

/// A choice of `choices.toml`: an AI take (`take-<n>`; `Take(0)` the original), an engine take
/// (`engine-<n>`: the game's own music read from its engine, on the score's timeline), a C64
/// take (`chip-<n>`) or a Spectrum 128 take (`ay-<n>`). The last two are a tune from another
/// version of the game, each on its own timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Choice {
    Take(u32),
    Chip(u32),
    Ay(u32),
    Engine(u32),
}

impl Choice {
    /// The choice a `choices.toml` value names: a number (0 the original), `"engine-<n>"`,
    /// `"chip-<n>"` or `"ay-<n>"` (n 1 or more). None: none of these.
    pub fn parse(v: &toml::Value) -> Option<Choice> {
        match v {
            toml::Value::Integer(n) => u32::try_from(*n).ok().map(Choice::Take),
            toml::Value::String(s) => Choice::named(s),
            _ => None,
        }
    }

    /// The take a folder name names besides `take-<n>`: `engine-<n>`, `chip-<n>` or `ay-<n>` (n 1
    /// or more, plain digits). None: none of these.
    pub fn named(s: &str) -> Option<Choice> {
        let number = |n: &str| Some(n).filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))?.parse().ok().filter(|&n| n > 0);
        match s.strip_prefix("engine-") {
            Some(n) => number(n).map(Choice::Engine),
            None => Choice::own_named(s),
        }
    }

    /// The take on its own timeline a folder name names: `chip-<n>` or `ay-<n>` (n 1 or more,
    /// plain digits). None: neither.
    pub fn own_named(s: &str) -> Option<Choice> {
        let number = |n: &str| Some(n).filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))?.parse().ok().filter(|&n| n > 0);
        if let Some(n) = s.strip_prefix("chip-") {
            number(n).map(Choice::Chip)
        } else {
            number(s.strip_prefix("ay-")?).map(Choice::Ay)
        }
    }

    /// Whether it is a tune on its own timeline (a C64 or Spectrum 128 take).
    pub fn is_own(self) -> bool {
        matches!(self, Choice::Chip(_) | Choice::Ay(_))
    }

    /// Its number.
    pub fn number(self) -> u32 {
        match self {
            Choice::Take(n) | Choice::Chip(n) | Choice::Ay(n) | Choice::Engine(n) => n,
        }
    }

    /// Its folder in the score's folder: `take-<n>`, `engine-<n>`, `chip-<n>` or `ay-<n>`.
    pub fn dir(self) -> String {
        match self {
            Choice::Take(n) => format!("take-{n}"),
            Choice::Chip(n) => format!("chip-{n}"),
            Choice::Ay(n) => format!("ay-{n}"),
            Choice::Engine(n) => format!("engine-{n}"),
        }
    }

    /// The kind of take: `AI`, `engine`, `C64` or `Spectrum 128`.
    pub fn kind(self) -> &'static str {
        match self {
            Choice::Take(_) => "AI",
            Choice::Chip(_) => "C64",
            Choice::Ay(_) => "Spectrum 128",
            Choice::Engine(_) => "engine",
        }
    }
}

impl fmt::Display for Choice {
    /// `take 2`, `engine take engine-1`, `C64 take chip-1` or `Spectrum 128 take ay-1`.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Choice::Take(n) => write!(f, "take {n}"),
            _ => write!(f, "{} take {}", self.kind(), self.dir()),
        }
    }
}

#[derive(Deserialize)]
struct ChoicesFile {
    #[serde(default)]
    choices: BTreeMap<String, toml::Value>,
}

/// The owner's choices, from `<dir>/choices.toml` (`dir`: the arranged music's folder): each
/// `"<game-id>/<key>"` and its [`Choice`]; none when the file is not there. Err: a file that
/// does not read, or a value that names no take, naming the file (and the key).
pub fn read_choices(dir: &Path) -> Result<BTreeMap<String, Choice>, String> {
    let path = dir.join(CHOICES);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let bad = |why: String| format!("{}: {why} (fix it, or choose again with tools/arrange-music pick)", path.display());
    let f: ChoicesFile = toml::from_str(&text).map_err(|e| bad(toml_error(&text, &e)))?;
    f.choices
        .into_iter()
        .map(|(k, v)| match Choice::parse(&v) {
            Some(c) => Ok((k, c)),
            None => Err(bad(format!(
                "{k:?} = {v}: a choice is a take's number (1 or more, 0 the original), \"engine-<n>\" (an engine take), \"chip-<n>\" (a C64 take) or \"ay-<n>\" (a Spectrum 128 take)"
            ))),
        })
        .collect()
}

#[derive(Deserialize)]
struct TakeFile {
    key: String,
    intro_t: i64,
    loop_t: i64,
    take: Option<toml::Value>,
    generator: Option<String>,
    timeline: Option<String>,
    own_intro_t: Option<i64>,
    own_loop_t: Option<i64>,
    source: Option<String>,
    #[serde(default)]
    note: Vec<TakeNote>,
}

#[derive(Clone, Copy, Deserialize)]
struct TakeNote {
    voice: i64,
    start_t: i64,
    end_t: i64,
    period_t: i64,
    level: i64,
}

/// A take made into a record ([`arrange`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Arranged {
    pub rec: ScoreRecord,
    /// What made it: an AI take's `generator`, a C64 or Spectrum 128 take's `source`.
    pub generator: String,
    /// The drum and pad notes dropped for frames over [`SND_EVENTS`] events.
    pub dropped: (usize, usize),
}

/// A piece of a note in a section: its section (0 the intro, 1 the loop), its start and end in
/// 64-T units from the section's start, its period, level and voice, and whether it reaches the
/// section's end (no note off: the device releases or restates it there).
#[derive(Clone, Copy, Debug)]
struct Piece {
    sec: usize,
    start: u64,
    end: u64,
    period: u32,
    level: u8,
    voice: u8,
    cut: bool,
}

/// Take `choice` of score `rec`, from the text of its `arranged.toml`, made into a record (see
/// the module docs): an AI take's on the score's timeline, a C64 or Spectrum 128 take's a
/// replacement on its own.
/// Err: why it does not fit.
pub fn arrange(rec: &ScoreRecord, choice: Choice, text: &str) -> Result<Arranged, String> {
    let f: TakeFile = toml::from_str(text).map_err(|e| format!("{TAKE_FILE} does not read: {}", toml_error(text, &e)))?;
    let key = score_key(rec);
    if f.key != key {
        return Err(format!("its key is {}, not the score's {key}", f.key));
    }
    if (f.intro_t, f.loop_t) != (i64::from(rec.intro_t), i64::from(rec.loop_t)) {
        return Err(format!("its timeline (intro_t {}, loop_t {}) is not the score's ({}, {})", f.intro_t, f.loop_t, rec.intro_t, rec.loop_t));
    }
    let named = match (&f.take, choice) {
        (None, _) => true,
        (Some(toml::Value::Integer(t)), c) => *t == i64::from(c.number()),
        (Some(toml::Value::String(t)), c) => !matches!(c, Choice::Take(_)) && Choice::named(t) == Some(c),
        _ => false,
    };
    if !named {
        let t = f.take.as_ref().map(ToString::to_string).unwrap_or_default();
        return Err(format!("it says take {t}, in {}", choice.dir()));
    }
    // a C64 or Spectrum 128 take's own timeline: its intro and its loop (0: none)
    let own = match (choice.is_own(), f.timeline.as_deref()) {
        (false, None | Some("score")) => None,
        (false, Some(t)) => {
            return Err(format!("its timeline is {t:?}: {} is on the score's (only a C64 or Spectrum 128 take, chip-<n> or ay-<n>, has its own)", choice.dir()))
        }
        (true, Some("own")) => {
            let (Some(oi), Some(ol)) = (f.own_intro_t, f.own_loop_t) else {
                return Err("it has no own_intro_t and own_loop_t (its own timeline)".into());
            };
            let why = if oi < 0 || ol < 0 {
                "below 0"
            } else if oi + ol == 0 {
                "no length"
            } else if oi + ol > OWN_MAX {
                "longer than a score may be"
            } else if ol != 0 && ol < i64::from(ZX_FRAME_TSTATES) {
                "a loop shorter than a frame"
            } else {
                ""
            };
            if !why.is_empty() {
                return Err(format!("its own timeline (own_intro_t {oi}, own_loop_t {ol}): {why}"));
            }
            Some((oi as u32, ol as u32))
        }
        (true, t) => {
            return Err(format!("its timeline is {}: a {} take's is \"own\"", t.map_or("not given".into(), |t| format!("{t:?}")), choice.kind()))
        }
    };
    if f.note.is_empty() {
        return Err("it has no notes".into());
    }
    // the record as it will be: a replacement plays with precedence; its events' sections
    let base = match own {
        Some(o) => ScoreRecord { replacement: Some(o), precedence: true, ..rec.clone() },
        None => ScoreRecord { replacement: None, ..rec.clone() },
    };
    let (intro_t, loop_t) = base.event_lengths();
    let (intro_t, loop_t) = (u64::from(intro_t), u64::from(loop_t));
    let end_t = (intro_t + loop_t) as i64;
    let whose = if own.is_some() { "take's own" } else { "score's" };
    let drum = i64::from(SCORE_DRUM);
    for (k, x) in f.note.iter().enumerate() {
        let k = k + 1;
        let why = if !(0..=drum).contains(&x.voice) {
            format!("voice {} (0-3)", x.voice)
        } else if x.start_t < 0 || x.end_t <= x.start_t {
            format!("it starts at {} T and ends at {} T", x.start_t, x.end_t)
        } else if x.end_t > end_t {
            format!("it ends at {} T, past the {whose} end at {end_t} T", x.end_t)
        } else if x.voice < drum && !(64..=PERIOD_MAX).contains(&x.period_t) {
            format!("period_t {} (64-{PERIOD_MAX} for a note)", x.period_t)
        } else if x.voice == drum && x.period_t != 0 {
            format!("period_t {} (0 for a drum)", x.period_t)
        } else if !(1..=255).contains(&x.level) {
            format!("level {} (1-255)", x.level)
        } else {
            continue;
        };
        return Err(format!("note {k}: {why}"));
    }
    // the file's checks done: every number fits its type
    let notes: Vec<Note> = f
        .note
        .iter()
        .map(|x| Note { voice: x.voice as u8, start_t: x.start_t as u64, end_t: x.end_t as u64, period_t: x.period_t as u32, level: x.level as u8 })
        .collect();
    for v in 0..=SCORE_DRUM {
        let mut mine: Vec<(usize, &Note)> = notes.iter().enumerate().filter(|(_, x)| x.voice == v).collect();
        mine.sort_by_key(|(_, x)| x.start_t);
        if let Some(w) = mine.windows(2).find(|w| w[1].1.start_t < w[0].1.end_t) {
            let (a, b) = (w[0].0.min(w[1].0) + 1, w[0].0.max(w[1].0) + 1);
            return Err(format!("notes {a} and {b} overlap on voice {v}"));
        }
    }
    // a score with no loop is followed only to its end: an own-timeline take's events past it never play,
    // so its own timeline stops there (the notes past it left out, one across it cut)
    let mut base = base;
    let (intro_t, loop_t) = match base.replacement {
        Some(_) if rec.loop_t == 0 && intro_t >= u64::from(rec.intro_t) => {
            base.replacement = Some((rec.intro_t, 0));
            (u64::from(rec.intro_t), 0)
        }
        _ => (intro_t, loop_t),
    };
    let mut pieces = pieces(&notes, intro_t, loop_t);
    let dropped = fit_frames(&mut pieces)?;
    let [intro_events, loop_events] = events(&pieces);
    for (ev, name) in [(&intro_events, "intro"), (&loop_events, "loop")] {
        if ev.len() > EVENTS_MAX {
            return Err(format!("its {name} has {} events, over {EVENTS_MAX}", ev.len()));
        }
    }
    let voices = notes.iter().filter(|x| x.voice < SCORE_DRUM).map(|x| x.voice + 1).max().unwrap_or(1);
    let mut roles = [0u8; SND_TONAL];
    roles[..usize::from(voices)].copy_from_slice(&TAKE_ROLES[..usize::from(voices)]);
    let generator = match (own, choice) {
        (Some(_), _) => f.source.unwrap_or_else(|| "no source named".into()),
        (None, Choice::Engine(_)) => f.source.or(f.generator).unwrap_or_else(|| "no source named".into()),
        (None, _) => f.generator.unwrap_or_else(|| "no generator named".into()),
    };
    Ok(Arranged { rec: ScoreRecord { voices, roles, intro_events, loop_events, ..base }, generator, dropped })
}

/// The pieces of `notes` in the intro (`intro_t` from T0) and the loop's pass (`loop_t` from
/// there), in order of section, start and voice (see the module docs).
fn pieces(notes: &[Note], intro_t: u64, loop_t: u64) -> Vec<Piece> {
    let unit = |t: u64| t.saturating_add(32) / 64;
    let mut out: Vec<Piece> = Vec::new();
    for (sec, (from, len)) in [(0, intro_t), (intro_t, loop_t)].into_iter().enumerate() {
        let (to, lim) = (from + len, len.div_ceil(64));
        for n in notes {
            if n.end_t <= from || n.start_t >= to || (sec == 1 && n.voice == SCORE_DRUM && n.start_t < from) {
                continue;
            }
            let start = unit(n.start_t.max(from) - from);
            let end = unit(n.end_t.min(to) - from);
            // reaching the section's end (or rounded to it): no note off
            let cut = n.end_t >= to || end >= lim;
            if start >= lim || (!cut && end <= start) {
                continue;
            }
            out.push(Piece { sec, start, end: end.min(lim), period: n.period_t, level: n.level, voice: n.voice, cut });
        }
    }
    out.sort_by_key(|p| (p.sec, p.start, p.voice));
    // a drum's off at least a unit before its next hit: shortened, or the hit left out when
    // that leaves it no length
    let mut k = 0;
    while k < out.len() {
        let p = out[k];
        if p.voice == SCORE_DRUM && !p.cut {
            if let Some(next) = out[k + 1..].iter().find(|x| x.voice == SCORE_DRUM && x.sec == p.sec).map(|x| x.start) {
                if next <= p.end {
                    if next <= p.start + 1 {
                        out.remove(k);
                        continue;
                    }
                    out[k].end = next - 1;
                }
            }
        }
        k += 1;
    }
    out
}

/// Whether piece `k` ends with a note off: not cut, and (a tonal voice) its voice's next piece
/// does not start where it ends.
fn has_off(pieces: &[Piece], k: usize) -> bool {
    let p = &pieces[k];
    // the pieces are in order of section and start
    let at = pieces.partition_point(|x| (x.sec, x.start) < (p.sec, p.end));
    !p.cut && (p.voice == SCORE_DRUM || !pieces[at..].iter().take_while(|x| (x.sec, x.start) == (p.sec, p.end)).any(|x| x.voice == p.voice))
}

/// The intro's and the loop's events of the pieces: a note on at each start, a note off at each
/// end that has one, in order of time (an off before an on, then by voice).
fn events(pieces: &[Piece]) -> [Vec<ScoreEvent>; 2] {
    type Key = (usize, u64, u8, u8);
    let mut ev: Vec<(Key, Option<(u32, u8)>)> = Vec::new();
    for (k, p) in pieces.iter().enumerate() {
        ev.push(((p.sec, p.start, 1, p.voice), Some((p.period, p.level))));
        if has_off(pieces, k) {
            ev.push(((p.sec, p.end, 0, p.voice), None));
        }
    }
    ev.sort_by_key(|e| e.0);
    let mut out = [Vec::new(), Vec::new()];
    for ((sec, u, _, voice), on) in ev {
        out[sec].push(ScoreEvent { t: (u * 64) as u32, voice, on });
    }
    out
}

/// Drops pieces until no frame (on its section's grid) holds more than [`SND_EVENTS`] events:
/// in the first frame over, the latest drum piece with an event there, else the latest pad
/// piece. Returns the drum and pad pieces dropped. Err: a frame over with neither.
fn fit_frames(pieces: &mut Vec<Piece>) -> Result<(usize, usize), String> {
    let mut dropped = (0, 0);
    loop {
        let [intro, looped] = events(pieces);
        let over = [intro, looped].iter().enumerate().find_map(|(sec, ev)| {
            let mut per: BTreeMap<u64, usize> = BTreeMap::new();
            for e in ev {
                *per.entry(u64::from(e.t) / 64 / FRAME_UNITS).or_default() += 1;
            }
            per.into_iter().find(|&(_, n)| n > SND_EVENTS).map(|(f, _)| (sec, f))
        });
        let Some((sec, f)) = over else { return Ok(dropped) };
        let there = |ps: &[Piece], k: usize| {
            let p = &ps[k];
            p.sec == sec && (p.start / FRAME_UNITS == f || (has_off(ps, k) && p.end / FRAME_UNITS == f))
        };
        let pick = |v: u8| (0..pieces.len()).rev().find(|&k| pieces[k].voice == v && there(pieces, k));
        match (pick(SCORE_DRUM), pick(2)) {
            (Some(k), _) => {
                pieces.remove(k);
                dropped.0 += 1;
            }
            (None, Some(k)) => {
                pieces.remove(k);
                dropped.1 += 1;
            }
            (None, None) => {
                let name = if sec == 0 { "intro" } else { "loop" };
                return Err(format!("frame {f} of the {name} has more than {SND_EVENTS} events on the lead and bass alone"));
            }
        }
    }
}

/// What plays for a score that has takes, or was given a choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Takes, none chosen.
    Original,
    /// Take `take` plays: its generator (a C64 or Spectrum 128 take's source), and the drum and
    /// pad notes dropped.
    Take { take: Choice, generator: String, dropped: (usize, usize) },
    /// Take `take` was chosen and does not fit: why.
    Refused { take: Choice, why: String },
}

impl Status {
    /// Its line in the report: `arranged: take 2 (<generator>)` (`arranged: C64 take chip-1
    /// (<source>)`, `arranged: Spectrum 128 take ay-1 (<source>)`), `arranged: original` or `arranged: take 2 refused: <why>` (a shadow score's:
    /// `arranged: the live version` when none is chosen).
    pub fn text(&self, shadow: bool) -> String {
        match self {
            Status::Original if shadow => "arranged: the live version (no take chosen)".into(),
            Status::Original => "arranged: original".into(),
            Status::Take { take, generator, dropped } => {
                let mut parts = Vec::new();
                for (n, what) in [(dropped.0, "drum"), (dropped.1, "pad")] {
                    if n > 0 {
                        parts.push(format!("{n} {what} note{}", if n == 1 { "" } else { "s" }));
                    }
                }
                let dropped = if parts.is_empty() {
                    String::new()
                } else {
                    format!(", {} dropped (over {SND_EVENTS} events in a frame)", parts.join(" and "))
                };
                format!("arranged: {take} ({generator}){dropped}")
            }
            Status::Refused { take, why } => format!("arranged: {take} refused: {why}"),
        }
    }
}

/// Whether a score's folder holds a take (a `take-<n>`, `engine-<n>`, `chip-<n>` or `ay-<n>`
/// folder with an `arranged.toml`).
pub fn has_takes(folder: &Path) -> bool {
    let named = |n: &str| n.strip_prefix("take-").is_some_and(|k| k.parse::<u32>().is_ok()) || Choice::named(n).is_some();
    std::fs::read_dir(folder).is_ok_and(|rd| rd.flatten().any(|e| e.file_name().to_str().is_some_and(named) && e.path().join(TAKE_FILE).is_file()))
}

/// Which score a take is of: score `k` of the game's list, or its shadow score `k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Of {
    Score(usize),
    Shadow(usize),
}

/// How [`apply`] verifies a take by replay: given the list with it, the score's index in the
/// list and which score it is; Err: why it does not play.
pub type Verify<'a> = &'a dyn Fn(&[u8], usize, Of) -> Result<(), String>;

/// A game's list with the chosen takes in it, and what plays for each score.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub list: Vec<u8>,
    /// Per score, in the list's order: None when it has no takes and no choice.
    pub status: Vec<Option<Status>>,
    /// Per shadow score, the same.
    pub shadow_status: Vec<Option<Status>>,
}

/// Puts the takes `choices` names for game `game` into its entry list `list` (its `scores`
/// decoded), each verified by `verify` (the list with it, the score's index) after the checks
/// (see the module docs); `dir` the arranged music's folder. The takes go in one after another,
/// each verified on the list with those before it. Then each shadow score of `shadows` with a
/// take chosen goes into the list after its scores, the take's record with precedence (method B
/// plays it in the frames the table claims too), within [`SND_SCORES_MAX`](zx_sys::SND_SCORES_MAX)
/// scores and [`SCORES_BYTES_MAX`] bytes with the others packed, verified the same way.
pub fn apply(
    dir: &Path,
    game: &str,
    list: &[u8],
    scores: &[ScoreRecord],
    shadows: &[ScoreRecord],
    choices: &BTreeMap<String, Choice>,
    verify: Verify,
) -> Applied {
    let mut cur = list.to_vec();
    let mut recs = scores.to_vec();
    let chosen = |rec: &ScoreRecord| {
        let key = score_key(rec);
        (dir.join(game).join(&key), choices.get(&format!("{game}/{key}")).copied().filter(|&c| c != Choice::Take(0)))
    };
    let mut status = Vec::new();
    for (i, rec) in scores.iter().enumerate() {
        let (folder, take) = chosen(rec);
        status.push(match take {
            Some(take) => Some(match one_take(&folder, take, &recs, i, &cur, verify) {
                Ok((a, l)) => {
                    cur = l;
                    recs[i] = a.rec;
                    Status::Take { take, generator: a.generator, dropped: a.dropped }
                }
                Err(why) => Status::Refused { take, why },
            }),
            None => has_takes(&folder).then_some(Status::Original),
        });
    }
    let mut shadow_status = Vec::new();
    for (i, rec) in shadows.iter().enumerate() {
        let (folder, take) = chosen(rec);
        shadow_status.push(match take {
            Some(take) => Some(match one_shadow(&folder, take, rec, i, &cur, verify) {
                Ok((a, l)) => {
                    cur = l;
                    Status::Take { take, generator: a.generator, dropped: a.dropped }
                }
                Err(why) => Status::Refused { take, why },
            }),
            None => has_takes(&folder).then_some(Status::Original),
        });
    }
    Applied { list: cur, status, shadow_status }
}

/// Take `take` of shadow score `i` (`rec`) in its `folder`: made into a record with precedence
/// and added to the list `list`, checked and verified. Err: why not.
fn one_shadow(folder: &Path, take: Choice, rec: &ScoreRecord, i: usize, list: &[u8], verify: Verify) -> Result<(Arranged, Vec<u8>), String> {
    let text = read_take(folder, take)?;
    let mut a = arrange(rec, take, &text)?;
    a.rec.precedence = true;
    let mut all = decode_scores(list)?;
    let index = all.len();
    all.push(a.rec.clone());
    fits(&all)?;
    let l = append_score(list, &a.rec)?;
    SndEngines::with_learned(&l).map_err(|e| format!("method B's decoder refuses it (error {e})"))?;
    verify(&l, index, Of::Shadow(i)).map_err(|why| format!("the replay: {why}"))?;
    Ok((a, l))
}

/// Take `take` of score `i` (of `recs`, the list `list`'s scores) in its `folder`: made into a
/// record and put into the list, checked and verified. Err: why not.
fn one_take(
    folder: &Path,
    take: Choice,
    recs: &[ScoreRecord],
    i: usize,
    list: &[u8],
    verify: Verify,
) -> Result<(Arranged, Vec<u8>), String> {
    let text = read_take(folder, take)?;
    let a = arrange(&recs[i], take, &text)?;
    let mut all = recs.to_vec();
    all[i] = a.rec.clone();
    fits(&all)?;
    let l = replace_scores(list, &all)?;
    SndEngines::with_learned(&l).map_err(|e| format!("method B's decoder refuses it (error {e})"))?;
    verify(&l, i, Of::Score(i)).map_err(|why| format!("the replay: {why}"))?;
    Ok((a, l))
}

/// The text of take `take`'s file in the score's `folder`. Err: why it does not read.
fn read_take(folder: &Path, take: Choice) -> Result<String, String> {
    let dir = take.dir();
    std::fs::read_to_string(folder.join(&dir).join(TAKE_FILE)).map_err(|e| format!("{dir}/{TAKE_FILE}: {e}"))
}

/// Whether a game's scores fit [`SCORES_BYTES_MAX`]. Err: how much they would take.
fn fits(scores: &[ScoreRecord]) -> Result<(), String> {
    let bytes: usize = scores.iter().map(record_bytes).sum();
    if bytes > SCORES_BYTES_MAX {
        return Err(format!("the game's scores would take {:.1} KB with it, over {} KB", bytes as f64 / 1024.0, SCORES_BYTES_MAX / 1024));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: u32 = ZX_FRAME_TSTATES;
    const D: u8 = SCORE_DRUM;

    fn ev(t: u32, voice: u8, on: Option<(u32, u8)>) -> ScoreEvent {
        ScoreEvent { t, voice, on }
    }

    /// A score: an intro of 10 frames and 10 T (not whole 64-T units), a loop of 20 frames.
    fn score() -> ScoreRecord {
        ScoreRecord {
            site: 0x8000,
            quiet: 25,
            code: 7,
            voices: 2,
            precedence: false,
            replacement: None,
            roles: [SND_ROLE_LEAD, SND_ROLE_BASS, 0],
            intro_t: 10 * F + 10,
            loop_t: 20 * F,
            intro_fp: vec![9; 10],
            loop_fp: vec![9; 20],
            intro_events: vec![
                ev(0, 0, Some((2000, 100))),
                ev(640, 0, Some((1800, 100))),
                ev(1280, 0, None),
                ev(1280, 1, Some((8000, 60))),
                ev(2000, D, Some((0, 40))),
            ],
            loop_events: vec![ev(0, 1, Some((8000, 60))), ev(3200, 1, None), ev(6400, 0, Some((1600, 80)))],
        }
    }

    fn n(voice: u8, start_t: u64, end_t: u64, period_t: u32, level: u8) -> Note {
        Note { voice, start_t, end_t, period_t, level }
    }

    #[test]
    fn a_score_on_one_timeline() {
        let s = score();
        let (i, end) = (u64::from(s.intro_t), u64::from(s.intro_t + s.loop_t));
        // a legato change ends a note, a note sounding at a section's end ends there, the loop's
        // notes at intro_t plus their time; the drum is not written
        assert_eq!(
            timeline(&s),
            [
                n(0, 0, 640, 2000, 100),
                n(0, 640, 1280, 1800, 100),
                n(1, 1280, i, 8000, 60),
                n(1, i, i + 3200, 8000, 60),
                n(0, i + 6400, end, 1600, 80),
            ]
        );
        let text = score_toml("a-game", "A \"game\"", &s);
        assert!(text.starts_with("# "));
        let body = &text[text.find('\n').unwrap() + 1..];
        assert!(
            body.starts_with(&format!(
                "game = \"a-game\"\nname = 'A \"game\"'\nkey = \"{}\"\nclock_hz = 3500000\nintro_t = {i}\nloop_t = {}\nvoices = 2\n\
                 roles = [\"lead\", \"bass\"]\n\n[[note]]\nvoice = 0\nstart_t = 0\nend_t = 640\nperiod_t = 2000\nlevel = 100\n\n[[note]]\n",
                score_key(&s),
                s.loop_t
            )),
            "{text}"
        );
        let t: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(t["note"].as_array().unwrap().len(), 5);
        assert_eq!(t["name"].as_str(), Some("A \"game\""));
    }

    /// The header of a take of `s` (number 1), then its notes.
    fn take(s: &ScoreRecord, notes: &[(i64, i64, i64, i64, i64)]) -> String {
        let mut t = format!("key = \"{}\"\nintro_t = {}\nloop_t = {}\ntake = 1\ngenerator = \"g 1\"\n", score_key(s), s.intro_t, s.loop_t);
        for (v, a, b, p, l) in notes {
            t += &format!("[[note]]\nvoice = {v}\nstart_t = {a}\nend_t = {b}\nperiod_t = {p}\nlevel = {l}\n");
        }
        t
    }

    /// An engine take is on the score's timeline: its notes become the record's events as an AI
    /// take's do, named by its source.
    #[test]
    fn an_engine_take_is_on_the_scores_timeline() {
        let s = score();
        let notes = [(0, 0, 640, 3000, 90), (1, 640, 1280, 9000, 50), (3, 0, 640, 0, 200)];
        let ai = arrange(&s, Choice::Take(1), &take(&s, &notes)).unwrap();
        let text = take(&s, &notes).replace("take = 1\ngenerator = \"g 1\"\n", "take = \"engine-1\"\ngenerator = \"engine ntropic\"\nsource = \"ntropic: its rows\"\n");
        let a = arrange(&s, Choice::Engine(1), &text).unwrap();
        assert_eq!((a.rec.intro_events, a.rec.loop_events, a.rec.replacement), (ai.rec.intro_events, ai.rec.loop_events, None));
        assert_eq!(a.generator, "ntropic: its rows");
        assert_eq!(Status::Take { take: Choice::Engine(1), generator: a.generator, dropped: (0, 0) }.text(false), "arranged: engine take engine-1 (ntropic: its rows)");
        // a number names it too; another kind's name or its own timeline does not fit
        assert!(arrange(&s, Choice::Engine(1), &take(&s, &notes)).is_ok());
        assert_eq!(arrange(&s, Choice::Chip(1), &text).unwrap_err(), "it says take \"engine-1\", in chip-1");
        assert_eq!(arrange(&s, Choice::Engine(2), &text).unwrap_err(), "it says take \"engine-1\", in engine-2");
        let own = text.replace("source =", "timeline = \"own\"\nown_intro_t = 640\nown_loop_t = 0\nsource =");
        assert!(arrange(&s, Choice::Engine(1), &own).unwrap_err().starts_with("its timeline is \"own\": engine-1 is on the score's"));
        assert_eq!((Choice::named("engine-3"), Choice::named("engine-0"), Choice::named("engine-x"), Choice::named("ay-2")), (Some(Choice::Engine(3)), None, None, Some(Choice::Ay(2))));
        assert_eq!((Choice::Engine(3).dir(), Choice::Engine(3).to_string(), Choice::Engine(3).is_own()), ("engine-3".into(), "engine take engine-3".into(), false));
        assert_eq!(Choice::parse(&toml::Value::String("engine-2".into())), Some(Choice::Engine(2)));
    }

    #[test]
    fn a_take_becomes_the_records_events() {
        let s = score();
        let (i, l) = (i64::from(s.intro_t), i64::from(s.loop_t));
        let a = arrange(
            &s,
            Choice::Take(1),
            &take(
                &s,
                &[
                    // a lead note across the intro's end (cut there, restated at the loop's start),
                    // one to the loop's end (no off), a bass note rounded to 64 T
                    (0, i - 640, i + 640, 3000, 90),
                    (0, i + l - 6400, i + l, 2500, 90),
                    (1, 100, 1000, 9000, 50),
                    // drums: one ending where the next starts (shortened), one hit a unit before
                    // the next (left out), one across the intro's end (not restated)
                    (3, 0, 640, 0, 200),
                    (3, 640, 1280, 0, 120),
                    (3, 1280, 1300, 0, 10),
                    (3, 1344, 2000, 0, 30),
                    (3, i - 64, i + 64, 0, 255),
                ],
            ),
        )
        .unwrap();
        assert_eq!((a.generator.as_str(), a.dropped), ("g 1", (0, 0)));
        assert_eq!((a.rec.voices, a.rec.roles), (2, [SND_ROLE_LEAD, SND_ROLE_BASS, 0]));
        // the trigger and fingerprints kept
        assert_eq!((a.rec.site, a.rec.code, a.rec.quiet, &a.rec.intro_fp), (s.site, s.code, s.quiet, &s.intro_fp));
        let u = |t: i64| ((t + 32) / 64 * 64) as u32;
        assert_eq!(
            a.rec.intro_events,
            [
                ev(0, D, Some((0, 200))),
                ev(u(100), 1, Some((9000, 50))),
                ev(576, D, None),
                ev(640, D, Some((0, 120))),
                ev(u(1000), 1, None),
                ev(1280, D, None),
                ev(1344, D, Some((0, 30))),
                ev(u(2000), D, None),
                ev(u(i - 640), 0, Some((3000, 90))),
                ev(u(i - 64), D, Some((0, 255))),
            ]
        );
        assert_eq!(a.rec.loop_events, [ev(0, 0, Some((3000, 90))), ev(640, 0, None), ev(u(l - 6400), 0, Some((2500, 90)))]);
        // no loop: the intro alone
        let plain = ScoreRecord { loop_t: 0, loop_fp: vec![], loop_events: vec![], ..s.clone() };
        let a = arrange(&plain, Choice::Take(1), &take(&plain, &[(0, 0, 640, 3000, 90), (3, 640, i, 0, 9)])).unwrap();
        assert_eq!(a.rec.intro_events, [ev(0, 0, Some((3000, 90))), ev(640, 0, None), ev(640, D, Some((0, 9)))]);
        assert!(a.rec.loop_events.is_empty());
        // only the lead and the drum: one voice
        assert_eq!((a.rec.voices, a.rec.roles), (1, [SND_ROLE_LEAD, 0, 0]));
    }

    #[test]
    fn takes_that_do_not_fit() {
        let s = score();
        let (i, l) = (i64::from(s.intro_t), i64::from(s.loop_t));
        let good = (0, 0, 640, 3000, 90);
        let why = |notes: &[(i64, i64, i64, i64, i64)]| arrange(&s, Choice::Take(1), &take(&s, notes)).unwrap_err();
        assert_eq!(why(&[good, (4, 0, 64, 0, 1)]), "note 2: voice 4 (0-3)");
        assert_eq!(why(&[(0, 640, 640, 3000, 90)]), "note 1: it starts at 640 T and ends at 640 T");
        assert_eq!(why(&[(0, -64, 640, 3000, 90)]), "note 1: it starts at -64 T and ends at 640 T");
        assert_eq!(why(&[(1, 0, i + l + 1, 3000, 90)]), format!("note 1: it ends at {} T, past the score's end at {} T", i + l + 1, i + l));
        assert_eq!(why(&[(2, 0, 64, 63, 90)]), "note 1: period_t 63 (64-16777215 for a note)");
        assert_eq!(why(&[(2, 0, 64, 1 << 24, 90)]), "note 1: period_t 16777216 (64-16777215 for a note)");
        assert_eq!(why(&[(3, 0, 64, 100, 90)]), "note 1: period_t 100 (0 for a drum)");
        assert_eq!(why(&[(1, 0, 64, 100, 0)]), "note 1: level 0 (1-255)");
        assert_eq!(why(&[(1, 0, 64, 100, 256)]), "note 1: level 256 (1-255)");
        assert_eq!(why(&[(1, 500, 900, 100, 9), good, (1, 0, 501, 100, 9)]), "notes 1 and 3 overlap on voice 1");
        assert_eq!(why(&[]), "it has no notes");
        let text = take(&s, &[good]);
        assert_eq!(arrange(&s, Choice::Take(2), &text).unwrap_err(), "it says take 1, in take-2");
        let other = ScoreRecord { site: 0x8001, ..s.clone() };
        assert_eq!(arrange(&other, Choice::Take(1), &text).unwrap_err(), format!("its key is {}, not the score's {}", score_key(&s), score_key(&other)));
        let longer = ScoreRecord { loop_t: s.loop_t + 64, ..s.clone() };
        let other = take(&longer, &[good]);
        assert_eq!(arrange(&s, Choice::Take(1), &other).unwrap_err(), format!("its timeline (intro_t {i}, loop_t {}) is not the score's ({i}, {l})", l + 64));
        assert!(arrange(&s, Choice::Take(1), "key = 3").unwrap_err().starts_with("arranged.toml does not read: line 1: "));
        assert!(arrange(&s, Choice::Take(1), &text.replace("key", "kee")).unwrap_err().starts_with("arranged.toml does not read: "));
    }

    #[test]
    fn a_frame_holds_eight_events() {
        let s = score();
        let f = i64::from(F);
        // frame 1 of the intro: 4 lead notes (5 events, legato), 2 pad notes (4) and 3 drum hits
        // (6): 15, so the 3 drums and a pad go (the latest first), leaving 7
        let mut notes = vec![(0, f, f + 640, 2000, 9), (0, f + 640, f + 1280, 2100, 9), (0, f + 1280, f + 1920, 2200, 9), (0, f + 1920, f + 2560, 2300, 9)];
        notes.extend([(2, f + 100, f + 200, 5000, 9), (2, f + 300, f + 400, 5000, 9)]);
        notes.extend([(3, f + 1000, f + 1100, 0, 9), (3, f + 1200, f + 1300, 0, 9), (3, f + 1400, f + 1500, 0, 9)]);
        let a = arrange(&s, Choice::Take(1), &take(&s, &notes)).unwrap();
        assert_eq!(a.dropped, (3, 1));
        assert_eq!(a.rec.intro_events.len(), 7);
        assert!(a.rec.intro_events.iter().any(|e| e.t == F + 128) && !a.rec.intro_events.iter().any(|e| e.t == F + 320));
        assert_eq!(Status::Take { take: Choice::Take(1), generator: "g".into(), dropped: a.dropped }.text(false), "arranged: take 1 (g), 3 drum notes and 1 pad note dropped (over 8 events in a frame)");
        // the lead and two drums, 9 events: the later drum goes
        notes.truncate(4);
        notes.extend([(3, f + 1000, f + 1100, 0, 9), (3, f + 1200, f + 1300, 0, 9)]);
        let a = arrange(&s, Choice::Take(1), &take(&s, &notes)).unwrap();
        assert_eq!(a.dropped, (1, 0));
        let drums: Vec<u32> = a.rec.intro_events.iter().filter(|e| e.voice == D).map(|e| e.t).collect();
        assert_eq!(drums, [F + 1024, F + 1088]);
        // the lead and bass alone over it: refused
        let busy: Vec<_> = (0..5).flat_map(|k| [(0, 64 * (2 * k), 64 * (2 * k + 1), 2000, 9), (1, 64 * (2 * k), 64 * (2 * k + 1), 4000, 9)]).collect();
        assert_eq!(arrange(&s, Choice::Take(1), &take(&s, &busy)).unwrap_err(), "frame 0 of the intro has more than 8 events on the lead and bass alone");
    }

    #[test]
    fn statuses() {
        assert_eq!(Status::Original.text(false), "arranged: original");
        assert_eq!(Status::Take { take: Choice::Take(2), generator: "m".into(), dropped: (0, 0) }.text(false), "arranged: take 2 (m)");
        assert_eq!(Status::Take { take: Choice::Take(2), generator: "m".into(), dropped: (1, 0) }.text(false), "arranged: take 2 (m), 1 drum note dropped (over 8 events in a frame)");
        assert_eq!(Status::Refused { take: Choice::Take(3), why: "x".into() }.text(false), "arranged: take 3 refused: x");
    }

    #[test]
    fn the_scores_bytes_are_limited() {
        let s = score();
        assert!(fits(std::slice::from_ref(&s)).is_ok());
        // a note on a unit for a long loop: over 128 KB
        let long = ScoreRecord {
            loop_t: 3000 * F,
            loop_fp: vec![9; 3000],
            loop_events: (0..140_000).map(|k| ev(k * 64, 0, Some((2000 + k % 2, 9)))).collect(),
            ..s
        };
        assert!(fits(&[long]).unwrap_err().ends_with(" KB with it, over 128 KB"));
    }

    /// A fresh temporary folder, removed when dropped.
    struct Tmp(std::path::PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("fmzx-arrange-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn choices_judged_table() {
        // tools/arrange-music judge --apply marks its own choices in a [judged] table: the
        // choices are [choices]'s alone, [judged] is not read
        let dir = Tmp::new("choices-judged");
        std::fs::write(
            dir.0.join(CHOICES),
            "[choices]\n\"g/8000-0a0b0c0d\" = \"chip-1\"\n\"h/1\" = 0\n\n[judged]\n\"g/8000-0a0b0c0d\" = \"chip-1\"\n\"x/2\" = \"ay-9\"\n",
        )
        .unwrap();
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::from([("g/8000-0a0b0c0d".into(), Choice::Chip(1)), ("h/1".into(), Choice::Take(0))])));
    }

    #[test]
    fn choices_files() {
        let dir = Tmp::new("choices");
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::new()));
        std::fs::write(dir.0.join(CHOICES), "[choices]\n\"g/8000-0a0b0c0d\" = 2\n\"h/1\" = 0\n\"h/2\" = \"chip-3\"\n\"h/3\" = \"ay-12\"\n\"h/4\" = \"engine-1\"\n").unwrap();
        assert_eq!(
            read_choices(&dir.0),
            Ok(BTreeMap::from([
                ("g/8000-0a0b0c0d".into(), Choice::Take(2)),
                ("h/1".into(), Choice::Take(0)),
                ("h/2".into(), Choice::Chip(3)),
                ("h/3".into(), Choice::Ay(12)),
                ("h/4".into(), Choice::Engine(1))
            ]))
        );
        std::fs::write(dir.0.join(CHOICES), "").unwrap();
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::new()));
        // not TOML: the line
        std::fs::write(dir.0.join(CHOICES), "[choices]\n\"g/k\" = \n").unwrap();
        let e = read_choices(&dir.0).unwrap_err();
        assert!(e.starts_with(&format!("{}: line 2: ", dir.0.join(CHOICES).display())), "{e}");
        // values that name no take: the file and the key
        for v in ["1.5", "-1", "4294967296", "\"two\"", "\"chip-0\"", "\"chip-\"", "\"chip-+1\"", "\"take-1\"", "\"chip-1 \"", "\"ay-0\"", "\"ay-\"", "\"ay-1x\"", "\"AY-1\"", "\"engine-0\"", "\"engine\"", "true", "[1]"] {
            std::fs::write(dir.0.join(CHOICES), format!("[choices]\n\"g/k\" = {v}\n")).unwrap();
            let e = read_choices(&dir.0).unwrap_err();
            let want = format!(
                "{}: \"g/k\" = {v}: a choice is a take's number (1 or more, 0 the original), \"engine-<n>\" (an engine take), \"chip-<n>\" (a C64 take) or \"ay-<n>\" (a Spectrum 128 take) (fix it, or choose again with tools/arrange-music pick)",
                dir.0.join(CHOICES).display()
            );
            assert_eq!(e, want);
        }
        assert_eq!((Choice::Take(2).to_string(), Choice::Chip(1).to_string()), ("take 2".into(), "C64 take chip-1".into()));
        assert_eq!((Choice::Take(2).dir(), Choice::Chip(1).dir()), ("take-2".into(), "chip-1".into()));
        assert_eq!((Choice::Ay(3).to_string(), Choice::Ay(3).dir()), ("Spectrum 128 take ay-3".into(), "ay-3".into()));
        assert_eq!((Choice::own_named("ay-3"), Choice::own_named("chip-3"), Choice::own_named("take-3")), (Some(Choice::Ay(3)), Some(Choice::Chip(3)), None));
        assert!(Choice::Ay(1).is_own() && Choice::Chip(1).is_own() && !Choice::Take(1).is_own());
    }

    /// A C64 take of `s` (`chip-1`, saying `take`), on its own timeline: an intro of `oi` T and a
    /// loop of `ol` T, then its notes.
    fn chip(s: &ScoreRecord, take: &str, oi: i64, ol: i64, notes: &[(i64, i64, i64, i64, i64)]) -> String {
        let mut t = format!(
            "key = \"{}\"\nintro_t = {}\nloop_t = {}\ntake = {take}\ngenerator = \"chip sid\"\ntimeline = \"own\"\nown_intro_t = {oi}\nown_loop_t = {ol}\n\
             source = \"C64: A Tune by Someone (1985 Nobody), subtune 2, X/A_Tune.sid\"\n[metrics]\ncloseness = 0.5\n",
            score_key(s),
            s.intro_t,
            s.loop_t
        );
        for (v, a, b, p, l) in notes {
            t += &format!("[[note]]\nvoice = {v}\nstart_t = {a}\nend_t = {b}\nperiod_t = {p}\nlevel = {l}\n");
        }
        t
    }

    #[test]
    fn a_chip_take_is_a_replacement_on_its_own_timeline() {
        let s = score();
        let f = i64::from(F);
        // its own intro 3 frames and 100 T, its loop 7 frames and 10 T (the score's: 10 and 20)
        let (oi, ol) = (3 * f + 100, 7 * f + 10);
        let notes = [
            // a lead note across the intro's end: cut there, restated at the loop's start
            (0, 0, 640, 3000, 90),
            (0, oi - 640, oi + 640, 2800, 90),
            // a bass note to the loop's end (no off), a pad note
            (1, oi + 1000, oi + ol, 9000, 50),
            (2, 128, 1280, 5000, 40),
            // drums: one ending where the next starts (shortened), one across the intro's end
            // (not restated)
            (3, 0, 640, 0, 200),
            (3, 640, 1280, 0, 120),
            (3, oi - 64, oi + 64, 0, 255),
        ];
        let u = |t: i64| ((t + 32) / 64 * 64) as u32;
        for take in ["\"chip-1\"", "1"] {
            let a = arrange(&s, Choice::Chip(1), &chip(&s, take, oi, ol, &notes)).unwrap();
            assert_eq!((a.generator.as_str(), a.dropped), ("C64: A Tune by Someone (1985 Nobody), subtune 2, X/A_Tune.sid", (0, 0)));
            // a replacement: precedence on, its own sections; the trigger and fingerprints kept
            assert_eq!((a.rec.replacement, a.rec.precedence), (Some((oi as u32, ol as u32)), true));
            assert_eq!(a.rec.event_lengths(), (oi as u32, ol as u32));
            assert_eq!((a.rec.site, a.rec.code, a.rec.quiet, a.rec.intro_t, a.rec.loop_t), (s.site, s.code, s.quiet, s.intro_t, s.loop_t));
            assert_eq!((&a.rec.intro_fp, &a.rec.loop_fp, score_key(&a.rec)), (&s.intro_fp, &s.loop_fp, score_key(&s)));
            assert_eq!((a.rec.voices, a.rec.roles), (3, [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_PAD]));
            assert_eq!(
                a.rec.intro_events,
                [
                    ev(0, 0, Some((3000, 90))),
                    ev(0, D, Some((0, 200))),
                    ev(128, 2, Some((5000, 40))),
                    ev(576, D, None),
                    ev(640, 0, None),
                    ev(640, D, Some((0, 120))),
                    ev(1280, 2, None),
                    ev(1280, D, None),
                    ev(u(oi - 640), 0, Some((2800, 90))),
                    ev(u(oi - 64), D, Some((0, 255))),
                ]
            );
            assert_eq!(a.rec.loop_events, [ev(0, 0, Some((2800, 90))), ev(640, 0, None), ev(1024, 1, Some((9000, 50)))]);
            // packed and read back as it is
            let list = soundlearn::encode_list_scores(&[], &[], std::slice::from_ref(&a.rec));
            assert_eq!(list[4 + 3] & 0xC0, 0xC0, "bits 6 and 7");
            assert_eq!(soundlearn::decode_scores(&list).unwrap()[0].replacement, a.rec.replacement);
            assert!(SndEngines::with_learned(&list).is_ok());
        }
        // no loop of its own: the intro alone, past the score's end
        let long = 40 * f;
        let a = arrange(&s, Choice::Chip(1), &chip(&s, "\"chip-1\"", long, 0, &[(0, 0, 640, 3000, 90), (0, long - 640, long, 2000, 90)])).unwrap();
        assert_eq!(a.rec.replacement, Some((long as u32, 0)));
        assert_eq!(a.rec.intro_events, [ev(0, 0, Some((3000, 90))), ev(640, 0, None), ev(u(long - 640), 0, Some((2000, 90)))]);
        assert!(a.rec.loop_events.is_empty());
        // a score with no loop is followed only to its end: the take stops there, its loop and
        // the notes past the end left out, a note across it cut
        let short = ScoreRecord { loop_t: 0, loop_fp: Vec::new(), ..s.clone() };
        let end = i64::from(short.intro_t);
        let across = [(0, 0, 640, 3000, 90), (1, end - 640, end + 640, 9000, 50), (0, end + 64, end + 1280, 2000, 90)];
        let a = arrange(&short, Choice::Chip(1), &chip(&short, "\"chip-1\"", end + 4 * f, ol, &across)).unwrap();
        assert_eq!(a.rec.replacement, Some((short.intro_t, 0)));
        assert_eq!(a.rec.intro_events, [ev(0, 0, Some((3000, 90))), ev(640, 0, None), ev(u(end - 640), 1, Some((9000, 50)))]);
        assert!(a.rec.loop_events.is_empty());
        assert_eq!(
            Status::Take { take: Choice::Chip(1), generator: a.generator, dropped: (0, 0) }.text(false),
            "arranged: C64 take chip-1 (C64: A Tune by Someone (1985 Nobody), subtune 2, X/A_Tune.sid)"
        );
        assert_eq!(Status::Refused { take: Choice::Chip(2), why: "x".into() }.text(true), "arranged: C64 take chip-2 refused: x");
        // its frames counted on its own grid: at the end of frame 1 of its loop (across a frame
        // from T0), 4 lead notes (5 events) and 3 drum hits (6): the 2 latest drums go
        let at = oi + 2 * f - 3700;
        let mut busy: Vec<_> = (0..4).map(|k| (0, at + 640 * k, at + 640 * (k + 1), 2000 + k, 9)).collect();
        busy.extend((0..3).map(|k| (3, at + 3000 + 200 * k, at + 3100 + 200 * k, 0, 9)));
        let a = arrange(&s, Choice::Chip(1), &chip(&s, "\"chip-1\"", oi, ol, &busy)).unwrap();
        assert_eq!(a.dropped, (2, 0));
        assert_eq!(a.rec.loop_events.len(), 7);
        assert_eq!(a.rec.loop_events.iter().filter(|e| e.voice == D).map(|e| e.t).collect::<Vec<_>>(), [u(at + 3000 - oi), u(at + 3100 - oi)]);
    }

    #[test]
    fn chip_takes_that_do_not_fit() {
        let s = score();
        let f = i64::from(F);
        let (oi, ol) = (3 * f + 100, 7 * f + 10);
        let good = [(0, 0, 640, 3000, 90)];
        let why = |text: &str| arrange(&s, Choice::Chip(1), text).unwrap_err();
        let ok = chip(&s, "\"chip-1\"", oi, ol, &good);
        assert!(arrange(&s, Choice::Chip(1), &ok).is_ok());
        assert_eq!(why(&chip(&s, "\"chip-2\"", oi, ol, &good)), "it says take \"chip-2\", in chip-1");
        assert_eq!(why(&chip(&s, "2", oi, ol, &good)), "it says take 2, in chip-1");
        // a chip take's file in a take-<n> folder, and an AI take's in a chip-<n> folder
        assert_eq!(arrange(&s, Choice::Take(1), &ok).unwrap_err(), "it says take \"chip-1\", in take-1");
        assert_eq!(
            arrange(&s, Choice::Take(1), &ok.replace("take = \"chip-1\"", "take = 1")).unwrap_err(),
            "its timeline is \"own\": take-1 is on the score's (only a C64 or Spectrum 128 take, chip-<n> or ay-<n>, has its own)"
        );
        assert_eq!(why(&take(&s, &good)), "its timeline is not given: a C64 take's is \"own\"");
        assert_eq!(why(&ok.replace("timeline = \"own\"", "timeline = \"score\"")), "its timeline is \"score\": a C64 take's is \"own\"");
        assert_eq!(why(&ok.replace(&format!("own_loop_t = {ol}\n"), "")), "it has no own_intro_t and own_loop_t (its own timeline)");
        let own = |a: i64, b: i64| why(&chip(&s, "\"chip-1\"", a, b, &good));
        assert_eq!(own(-64, ol), format!("its own timeline (own_intro_t -64, own_loop_t {ol}): below 0"));
        assert_eq!(own(0, 0), "its own timeline (own_intro_t 0, own_loop_t 0): no length");
        assert_eq!(own(oi, f - 1), format!("its own timeline (own_intro_t {oi}, own_loop_t {}): a loop shorter than a frame", f - 1));
        assert!(arrange(&s, Choice::Chip(1), &chip(&s, "\"chip-1\"", OWN_MAX, 0, &good)).is_ok(), "the longest");
        assert_eq!(own(OWN_MAX, f), format!("its own timeline (own_intro_t {OWN_MAX}, own_loop_t {f}): longer than a score may be"));
        // the key and the score's header still the score's
        let other = ScoreRecord { site: 0x8001, ..s.clone() };
        assert_eq!(arrange(&other, Choice::Chip(1), &ok).unwrap_err(), format!("its key is {}, not the score's {}", score_key(&s), score_key(&other)));
        // notes: within its own timeline, voices 0-3, periods, levels, no overlaps
        let n = |notes: &[(i64, i64, i64, i64, i64)]| why(&chip(&s, "\"chip-1\"", oi, ol, notes));
        assert_eq!(n(&[(0, 0, oi + ol + 1, 3000, 9)]), format!("note 1: it ends at {} T, past the take's own end at {} T", oi + ol + 1, oi + ol));
        assert_eq!(n(&[(4, 0, 64, 3000, 9)]), "note 1: voice 4 (0-3)");
        assert_eq!(n(&[(1, 0, 64, 63, 9)]), "note 1: period_t 63 (64-16777215 for a note)");
        assert_eq!(n(&[(3, 0, 64, 1, 9)]), "note 1: period_t 1 (0 for a drum)");
        assert_eq!(n(&[(2, 0, 64, 3000, 0)]), "note 1: level 0 (1-255)");
        assert_eq!(n(&[(2, 0, 640, 3000, 9), (2, 600, 700, 3000, 9)]), "notes 1 and 2 overlap on voice 2");
        assert_eq!(n(&[]), "it has no notes");
    }

    /// A Spectrum 128 take of `s` (`ay-1`, saying `take`): [`chip`]'s, from a `.ay` file.
    fn ay(s: &ScoreRecord, take: &str, oi: i64, ol: i64, notes: &[(i64, i64, i64, i64, i64)]) -> String {
        chip(s, take, oi, ol, notes)
            .replace("generator = \"chip sid\"", "generator = \"chip ay\"")
            .replace("C64: A Tune by Someone (1985 Nobody), subtune 2, X/A_Tune.sid", "Spectrum 128: A Tune by Someone, song 2, games/A_Tune.ay")
    }

    #[test]
    fn an_ay_take_is_a_replacement_as_a_chip_take_is() {
        let s = score();
        let f = i64::from(F);
        let (oi, ol) = (3 * f + 100, 7 * f + 10);
        let notes = [(0, 0, 640, 3000, 90), (1, oi + 1000, oi + ol, 9000, 50), (3, 640, 1280, 0, 120)];
        let c = arrange(&s, Choice::Chip(1), &chip(&s, "\"chip-1\"", oi, ol, &notes)).unwrap();
        for take in ["\"ay-1\"", "1"] {
            let a = arrange(&s, Choice::Ay(1), &ay(&s, take, oi, ol, &notes)).unwrap();
            assert_eq!(a.rec, c.rec, "the same record as the C64 take's");
            assert_eq!((a.rec.replacement, a.rec.precedence), (Some((oi as u32, ol as u32)), true));
            assert_eq!(a.generator, "Spectrum 128: A Tune by Someone, song 2, games/A_Tune.ay");
            assert_eq!(
                Status::Take { take: Choice::Ay(1), generator: a.generator, dropped: (0, 0) }.text(false),
                "arranged: Spectrum 128 take ay-1 (Spectrum 128: A Tune by Someone, song 2, games/A_Tune.ay)"
            );
        }
        // a score with no loop: the take stops at its end
        let short = ScoreRecord { loop_t: 0, loop_fp: Vec::new(), ..s.clone() };
        let end = i64::from(short.intro_t);
        let a = arrange(&short, Choice::Ay(1), &ay(&short, "\"ay-1\"", end + 4 * f, ol, &[(0, 0, 640, 3000, 90), (0, end + 64, end + 1280, 2000, 90)])).unwrap();
        assert_eq!(a.rec.replacement, Some((short.intro_t, 0)));
        assert!(a.rec.loop_events.is_empty());
        // what does not fit
        let why = |text: &str| arrange(&s, Choice::Ay(1), text).unwrap_err();
        let ok = ay(&s, "\"ay-1\"", oi, ol, &notes);
        assert_eq!(why(&ay(&s, "\"chip-1\"", oi, ol, &notes)), "it says take \"chip-1\", in ay-1");
        assert_eq!(why(&ay(&s, "\"ay-2\"", oi, ol, &notes)), "it says take \"ay-2\", in ay-1");
        assert_eq!(arrange(&s, Choice::Chip(1), &ok).unwrap_err(), "it says take \"ay-1\", in chip-1");
        assert_eq!(arrange(&s, Choice::Take(1), &ok).unwrap_err(), "it says take \"ay-1\", in take-1");
        assert_eq!(why(&take(&s, &notes)), "its timeline is not given: a Spectrum 128 take's is \"own\"");
        assert_eq!(why(&ay(&s, "\"ay-1\"", oi, f - 1, &notes)), format!("its own timeline (own_intro_t {oi}, own_loop_t {}): a loop shorter than a frame", f - 1));
        assert_eq!(Status::Refused { take: Choice::Ay(2), why: "x".into() }.text(false), "arranged: Spectrum 128 take ay-2 refused: x");
        // its folder holds a take
        let dir = Tmp::new("ay-takes");
        assert!(!has_takes(&dir.0));
        std::fs::create_dir_all(dir.0.join("ay-1")).unwrap();
        std::fs::write(dir.0.join("ay-1").join(TAKE_FILE), &ok).unwrap();
        assert!(has_takes(&dir.0));
    }

    #[test]
    fn a_choice_applied() {
        let dir = Tmp::new("apply");
        let s = score();
        let list = soundlearn::encode_list_scores(&[], &[], std::slice::from_ref(&s));
        let key = score_key(&s);
        let folder = dir.0.join("g").join(&key);
        let ok = |_: &[u8], _: usize, _: Of| Ok(());
        // no takes, no choice: nothing to say; a take: the original
        let none = BTreeMap::new();
        assert_eq!(apply(&dir.0, "g", &list, std::slice::from_ref(&s), &[], &none, &ok), Applied { list: list.clone(), status: vec![None], shadow_status: vec![] });
        std::fs::create_dir_all(folder.join("take-1")).unwrap();
        assert!(!has_takes(&folder));
        std::fs::write(folder.join("take-1").join(TAKE_FILE), take(&s, &[(0, 0, 640, 3000, 92)])).unwrap();
        assert!(has_takes(&folder));
        assert_eq!(apply(&dir.0, "g", &list, std::slice::from_ref(&s), &[], &none, &ok).status, [Some(Status::Original)]);
        // chosen: in the list, verified with the list and the score's index
        let chosen = BTreeMap::from([(format!("g/{key}"), Choice::Take(1))]);
        let seen = std::cell::RefCell::new(Vec::new());
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &[], &chosen, &|l, i, of| {
            assert_eq!(of, Of::Score(0));
            seen.borrow_mut().push((l.to_vec(), i));
            Ok(())
        });
        let want = arrange(&s, Choice::Take(1), &take(&s, &[(0, 0, 640, 3000, 92)])).unwrap();
        assert_eq!(got.status, [Some(Status::Take { take: Choice::Take(1), generator: "g 1".into(), dropped: (0, 0) })]);
        assert_eq!(soundlearn::decode_scores(&got.list).unwrap(), [want.rec]);
        assert_eq!(seen.into_inner(), [(got.list.clone(), 0)]);
        // the replay fails, or the take is not there: the original, with the reason
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &[], &chosen, &|_, _, _| Err("it stops".into()));
        assert_eq!(
            got,
            Applied { list: list.clone(), status: vec![Some(Status::Refused { take: Choice::Take(1), why: "the replay: it stops".into() })], shadow_status: vec![] }
        );
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &[], &BTreeMap::from([(format!("g/{key}"), Choice::Take(4))]), &ok);
        assert_eq!(got.list, list);
        assert!(matches!(&got.status[0], Some(Status::Refused { take: Choice::Take(4), why }) if why.starts_with("take-4/arranged.toml: ")), "{got:?}");
    }

    #[test]
    fn a_shadow_score_goes_in_only_with_a_take() {
        let dir = Tmp::new("shadow");
        let s = score();
        let shadow = ScoreRecord { site: 0x8100, precedence: true, ..s.clone() };
        let list = soundlearn::encode_list_scores(&[], &[], std::slice::from_ref(&s));
        let key = score_key(&shadow);
        let folder = dir.0.join("g").join(&key);
        let ok = |_: &[u8], _: usize, _: Of| Ok(());
        // not chosen: the list as it was; with takes, the live version plays
        let none = BTreeMap::new();
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), std::slice::from_ref(&shadow), &none, &ok);
        assert_eq!(got, Applied { list: list.clone(), status: vec![None], shadow_status: vec![None] });
        std::fs::create_dir_all(folder.join("take-1")).unwrap();
        std::fs::write(folder.join("take-1").join(TAKE_FILE), take(&shadow, &[(0, 0, 640, 3000, 92)])).unwrap();
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), std::slice::from_ref(&shadow), &none, &ok);
        assert_eq!((got.list.as_slice(), got.shadow_status[0].as_ref()), (list.as_slice(), Some(&Status::Original)));
        assert_eq!(Status::Original.text(true), "arranged: the live version (no take chosen)");
        // chosen: after the scores, with precedence, verified as score 1 of the list
        let chosen = BTreeMap::from([(format!("g/{key}"), Choice::Take(1))]);
        let seen = std::cell::RefCell::new(Vec::new());
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), std::slice::from_ref(&shadow), &chosen, &|l, i, of| {
            seen.borrow_mut().push((l.to_vec(), i, of));
            Ok(())
        });
        let mut want = arrange(&shadow, Choice::Take(1), &take(&shadow, &[(0, 0, 640, 3000, 92)])).unwrap().rec;
        want.precedence = true;
        assert_eq!(got.shadow_status, [Some(Status::Take { take: Choice::Take(1), generator: "g 1".into(), dropped: (0, 0) })]);
        let back = soundlearn::decode_scores(&list).unwrap().remove(0);
        assert_eq!(soundlearn::decode_scores(&got.list).unwrap(), [back, want]);
        assert_eq!(got.list[0], list[0] + 1);
        assert_eq!(seen.into_inner(), [(got.list.clone(), 1, Of::Shadow(0))]);
        assert!(SndEngines::with_learned(&got.list).is_ok());
        // the list full: refused, the live version plays
        let eight = vec![s.clone(); zx_sys::SND_SCORES_MAX];
        let full = soundlearn::encode_list_scores(&[], &[], &eight);
        let got = apply(&dir.0, "g", &full, &eight, std::slice::from_ref(&shadow), &chosen, &ok);
        assert_eq!(got.list, full);
        assert!(matches!(&got.shadow_status[0], Some(Status::Refused { take: Choice::Take(1), why }) if why.contains("8 scores already")), "{got:?}");
        // the replay fails: the list as it was
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), std::slice::from_ref(&shadow), &chosen, &|_, _, _| Err("it stops".into()));
        assert_eq!(got.list, list);
        assert_eq!(got.shadow_status, [Some(Status::Refused { take: Choice::Take(1), why: "the replay: it stops".into() })]);
        // its score.toml says shadow
        assert!(score_toml("g", "G", &shadow).contains("\nshadow = true\n"));
        assert!(!score_toml("g", "G", &s).contains("shadow"));
    }

    #[test]
    fn scores_are_written_when_they_change() {
        let dir = Tmp::new("export");
        let s = score();
        export(&dir.0, "g", "G", std::slice::from_ref(&s)).unwrap();
        let path = dir.0.join("g").join(score_key(&s)).join(SCORE_FILE);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), score_toml("g", "G", &s));
        std::fs::write(&path, "old").unwrap();
        export(&dir.0, "g", "G", std::slice::from_ref(&s)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), score_toml("g", "G", &s));
    }
}
