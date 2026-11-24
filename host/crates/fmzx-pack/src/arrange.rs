// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Arranged music: the personal pack's recorded scores written out for `tools/arrange-music`,
//! and the takes the owner chose put into the pack in place of the scores' notes.
//!
//! # The scores, written out
//!
//! Each recorded score (kind-2 record) of each packed game is written to
//! `<out>/arrange/<game-id>/<key>/score.toml` (`<key>`: [`score_key`]): the game's id and name,
//! the key, `clock_hz`, the record's `intro_t` and `loop_t`, its tonal `voices` and their
//! `roles` by name, and its notes on one timeline, the intro and then one pass of the loop (a
//! loop note at `intro_t` plus its time in the loop), each `[[note]]` with its `voice` (0-2),
//! `start_t` and `end_t` (T-states since T0), `period_t` (the full period in T-states) and
//! `level` (1-255), by start and then voice. A note runs from its note on to the voice's next
//! note on or note off, or to its section's end (where the device releases or restates it). A
//! file is written only when its text changes; the folder's other files (the takes, the renders)
//! are left alone, and the folder of a score the pack no longer holds stays as it is.
//!
//! # The takes chosen
//!
//! `<out>/arrange/choices.toml` holds a `[choices]` table, `"<game-id>/<key>" = <take>`, 0 for
//! the original (one that does not read fails the run). A chosen take is read from
//! `<key>/take-<n>/arranged.toml` (the header of `score.toml` and `take`, `generator`, and notes
//! on 4 voices: 0 the lead, 1 the bass, 2 the pad, 3 the drum with `period_t = 0`, a hit from
//! `start_t` to `end_t`) and checked ([`arrange`]): its key and timeline the score's, every note
//! within the timeline, on voices 0-3, with no two on one voice overlapping, a tonal note's
//! period 64 to 16,777,215 T, a drum's 0, levels 1-255. Its notes become the record's events
//! as the learner builds a score's ([`soundlearn::scores::score_events`]): times rounded to 64
//! T, the notes cut at the intro's end and the loop's, each tonal voice sounding at the loop's
//! start restated there, a note off at a note's end unless the voice's next note starts there
//! (or the note reaches its section's end); voices 0-2 tonal with the roles lead, bass and pad,
//! voice 3 the drum's hits and offs, a drum off at least 64 T before the next hit (shortened
//! where needed; a drum note is never restated: the device strikes the drum anew at each pass).
//! A frame (a bucket of 69,888 T on its section's grid) holds at most [`SND_EVENTS`] events: past
//! that, drum notes are dropped first, then pad notes (the latest in the frame first), and the
//! take says how many; a frame over it with the lead and bass alone refuses the take. The record
//! keeps the score's trigger, code, quiet and fingerprints, so it plays in step as before.
//! Then ([`apply`]) the game's scores must stay within [`SCORES_BYTES_MAX`], method B's decoder
//! must take the list, and the replay must verify the score ([`soundlearn::scores::verify`]).
//! A take that fails any of these leaves the original, with the reason.

use serde::Deserialize;
use soundlearn::scores::{record_bytes, SCORES_BYTES_MAX};
use soundlearn::{replace_scores, score_key, ScoreEvent, ScoreRecord, SCORE_DRUM};
use std::collections::BTreeMap;
use std::path::Path;
use zx_sys::{SndEngines, SND_EVENTS, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD, SND_TONAL, ZX_FRAME_TSTATES};

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

/// The tonal notes of a record on one timeline (see the module docs), by start and then voice.
pub fn timeline(rec: &ScoreRecord) -> Vec<Note> {
    let mut out = Vec::new();
    let intro_t = u64::from(rec.intro_t);
    for (events, from, len) in [(&rec.intro_events, 0, intro_t), (&rec.loop_events, intro_t, u64::from(rec.loop_t))] {
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
/// game's name.
pub fn score_toml(game: &str, name: &str, rec: &ScoreRecord) -> String {
    let roles: Vec<String> = rec.roles[..usize::from(rec.voices).min(SND_TONAL)].iter().map(|&r| quoted(role_name(r))).collect();
    let mut s = format!(
        "# A recorded score's notes, for tools/arrange-music (written by tools/make-personal-pack).\n\
         game = {}\nname = {}\nkey = {}\nclock_hz = {CLOCK_HZ}\nintro_t = {}\nloop_t = {}\nvoices = {}\nroles = [{}]\n",
        quoted(game),
        quoted(name),
        quoted(&score_key(rec)),
        rec.intro_t,
        rec.loop_t,
        rec.voices,
        roles.join(", ")
    );
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

#[derive(Deserialize)]
struct ChoicesFile {
    #[serde(default)]
    choices: BTreeMap<String, i64>,
}

/// The owner's choices, from `<dir>/choices.toml` (`dir`: the arranged music's folder): each
/// `"<game-id>/<key>"` and its take (0: the original); none when the file is not there. Err: a
/// file that does not read, naming it.
pub fn read_choices(dir: &Path) -> Result<BTreeMap<String, u32>, String> {
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
        .map(|(k, n)| match u32::try_from(n) {
            Ok(n) => Ok((k, n)),
            Err(_) => Err(bad(format!("{k:?} = {n}: a take is 1 or more, 0 the original"))),
        })
        .collect()
}

#[derive(Deserialize)]
struct TakeFile {
    key: String,
    intro_t: i64,
    loop_t: i64,
    take: Option<i64>,
    generator: Option<String>,
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
    /// What made it (`generator` in its file).
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

/// Take `n` of score `rec`, from the text of its `arranged.toml`, made into a record (see the
/// module docs). Err: why it does not fit.
pub fn arrange(rec: &ScoreRecord, n: u32, text: &str) -> Result<Arranged, String> {
    let f: TakeFile = toml::from_str(text).map_err(|e| format!("{TAKE_FILE} does not read: {}", toml_error(text, &e)))?;
    let key = score_key(rec);
    if f.key != key {
        return Err(format!("its key is {}, not the score's {key}", f.key));
    }
    if (f.intro_t, f.loop_t) != (i64::from(rec.intro_t), i64::from(rec.loop_t)) {
        return Err(format!("its timeline (intro_t {}, loop_t {}) is not the score's ({}, {})", f.intro_t, f.loop_t, rec.intro_t, rec.loop_t));
    }
    if let Some(t) = f.take.filter(|&t| t != i64::from(n)) {
        return Err(format!("it says take {t}, in take-{n}"));
    }
    if f.note.is_empty() {
        return Err("it has no notes".into());
    }
    let (intro_t, loop_t) = (u64::from(rec.intro_t), u64::from(rec.loop_t));
    let end_t = (intro_t + loop_t) as i64;
    let drum = i64::from(SCORE_DRUM);
    for (k, x) in f.note.iter().enumerate() {
        let k = k + 1;
        let why = if !(0..=drum).contains(&x.voice) {
            format!("voice {} (0-3)", x.voice)
        } else if x.start_t < 0 || x.end_t <= x.start_t {
            format!("it starts at {} T and ends at {} T", x.start_t, x.end_t)
        } else if x.end_t > end_t {
            format!("it ends at {} T, past the score's end at {end_t} T", x.end_t)
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
    let mut pieces = pieces(&notes, intro_t, loop_t);
    let dropped = fit_frames(&mut pieces)?;
    let [intro_events, loop_events] = events(&pieces);
    let voices = notes.iter().filter(|x| x.voice < SCORE_DRUM).map(|x| x.voice + 1).max().unwrap_or(1);
    let mut roles = [0u8; SND_TONAL];
    roles[..usize::from(voices)].copy_from_slice(&TAKE_ROLES[..usize::from(voices)]);
    let generator = f.generator.unwrap_or_else(|| "no generator named".into());
    Ok(Arranged { rec: ScoreRecord { voices, roles, intro_events, loop_events, ..rec.clone() }, generator, dropped })
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
    !p.cut && (p.voice == SCORE_DRUM || !pieces.iter().any(|x| x.voice == p.voice && x.sec == p.sec && x.start == p.end))
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
    /// Take `take` plays: its generator, and the drum and pad notes dropped.
    Take { take: u32, generator: String, dropped: (usize, usize) },
    /// Take `take` was chosen and does not fit: why.
    Refused { take: u32, why: String },
}

impl Status {
    /// Its line in the report: `arranged: take 2 (<generator>)`, `arranged: original` or
    /// `arranged: take 2 refused: <why>`.
    pub fn text(&self) -> String {
        match self {
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
                format!("arranged: take {take} ({generator}){dropped}")
            }
            Status::Refused { take, why } => format!("arranged: take {take} refused: {why}"),
        }
    }
}

/// Whether a score's folder holds a take (a `take-<n>/arranged.toml`).
pub fn has_takes(folder: &Path) -> bool {
    std::fs::read_dir(folder).is_ok_and(|rd| {
        rd.flatten().any(|e| e.file_name().to_str().is_some_and(|n| n.strip_prefix("take-").is_some_and(|k| k.parse::<u32>().is_ok())) && e.path().join(TAKE_FILE).is_file())
    })
}

/// How [`apply`] verifies a take by replay: given the list with it and the score's index; Err:
/// why it does not play.
pub type Verify<'a> = &'a dyn Fn(&[u8], usize) -> Result<(), String>;

/// A game's list with the chosen takes in it, and what plays for each score.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub list: Vec<u8>,
    /// Per score, in the list's order: None when it has no takes and no choice.
    pub status: Vec<Option<Status>>,
}

/// Puts the takes `choices` names for game `game` into its entry list `list` (its `scores`
/// decoded), each verified by `verify` (the list with it, the score's index) after the checks
/// (see the module docs); `dir` the arranged music's folder. The takes go in one after another,
/// each verified on the list with those before it.
pub fn apply(
    dir: &Path,
    game: &str,
    list: &[u8],
    scores: &[ScoreRecord],
    choices: &BTreeMap<String, u32>,
    verify: Verify,
) -> Applied {
    let mut cur = list.to_vec();
    let mut recs = scores.to_vec();
    let mut status = Vec::new();
    for (i, rec) in scores.iter().enumerate() {
        let key = score_key(rec);
        let folder = dir.join(game).join(&key);
        status.push(match choices.get(&format!("{game}/{key}")).copied().filter(|&n| n > 0) {
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
    Applied { list: cur, status }
}

/// Take `take` of score `i` (of `recs`, the list `list`'s scores) in its `folder`: made into a
/// record and put into the list, checked and verified. Err: why not.
fn one_take(
    folder: &Path,
    take: u32,
    recs: &[ScoreRecord],
    i: usize,
    list: &[u8],
    verify: Verify,
) -> Result<(Arranged, Vec<u8>), String> {
    let path = folder.join(format!("take-{take}")).join(TAKE_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("take-{take}/{TAKE_FILE}: {e}"))?;
    let a = arrange(&recs[i], take, &text)?;
    let mut all = recs.to_vec();
    all[i] = a.rec.clone();
    fits(&all)?;
    let l = replace_scores(list, &all)?;
    SndEngines::with_learned(&l).map_err(|e| format!("method B's decoder refuses it (error {e})"))?;
    verify(&l, i).map_err(|why| format!("the replay: {why}"))?;
    Ok((a, l))
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

    #[test]
    fn a_take_becomes_the_records_events() {
        let s = score();
        let (i, l) = (i64::from(s.intro_t), i64::from(s.loop_t));
        let a = arrange(
            &s,
            1,
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
        let a = arrange(&plain, 1, &take(&plain, &[(0, 0, 640, 3000, 90), (3, 640, i, 0, 9)])).unwrap();
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
        let why = |notes: &[(i64, i64, i64, i64, i64)]| arrange(&s, 1, &take(&s, notes)).unwrap_err();
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
        assert_eq!(arrange(&s, 2, &text).unwrap_err(), "it says take 1, in take-2");
        let other = ScoreRecord { site: 0x8001, ..s.clone() };
        assert_eq!(arrange(&other, 1, &text).unwrap_err(), format!("its key is {}, not the score's {}", score_key(&s), score_key(&other)));
        let longer = ScoreRecord { loop_t: s.loop_t + 64, ..s.clone() };
        let other = take(&longer, &[good]);
        assert_eq!(arrange(&s, 1, &other).unwrap_err(), format!("its timeline (intro_t {i}, loop_t {}) is not the score's ({i}, {l})", l + 64));
        assert!(arrange(&s, 1, "key = 3").unwrap_err().starts_with("arranged.toml does not read: line 1: "));
        assert!(arrange(&s, 1, &text.replace("key", "kee")).unwrap_err().starts_with("arranged.toml does not read: "));
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
        let a = arrange(&s, 1, &take(&s, &notes)).unwrap();
        assert_eq!(a.dropped, (3, 1));
        assert_eq!(a.rec.intro_events.len(), 7);
        assert!(a.rec.intro_events.iter().any(|e| e.t == F + 128) && !a.rec.intro_events.iter().any(|e| e.t == F + 320));
        assert_eq!(Status::Take { take: 1, generator: "g".into(), dropped: a.dropped }.text(), "arranged: take 1 (g), 3 drum notes and 1 pad note dropped (over 8 events in a frame)");
        // the lead and two drums, 9 events: the later drum goes
        notes.truncate(4);
        notes.extend([(3, f + 1000, f + 1100, 0, 9), (3, f + 1200, f + 1300, 0, 9)]);
        let a = arrange(&s, 1, &take(&s, &notes)).unwrap();
        assert_eq!(a.dropped, (1, 0));
        let drums: Vec<u32> = a.rec.intro_events.iter().filter(|e| e.voice == D).map(|e| e.t).collect();
        assert_eq!(drums, [F + 1024, F + 1088]);
        // the lead and bass alone over it: refused
        let busy: Vec<_> = (0..5).flat_map(|k| [(0, 64 * (2 * k), 64 * (2 * k + 1), 2000, 9), (1, 64 * (2 * k), 64 * (2 * k + 1), 4000, 9)]).collect();
        assert_eq!(arrange(&s, 1, &take(&s, &busy)).unwrap_err(), "frame 0 of the intro has more than 8 events on the lead and bass alone");
    }

    #[test]
    fn statuses() {
        assert_eq!(Status::Original.text(), "arranged: original");
        assert_eq!(Status::Take { take: 2, generator: "m".into(), dropped: (0, 0) }.text(), "arranged: take 2 (m)");
        assert_eq!(Status::Take { take: 2, generator: "m".into(), dropped: (1, 0) }.text(), "arranged: take 2 (m), 1 drum note dropped (over 8 events in a frame)");
        assert_eq!(Status::Refused { take: 3, why: "x".into() }.text(), "arranged: take 3 refused: x");
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
    fn choices_files() {
        let dir = Tmp::new("choices");
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::new()));
        std::fs::write(dir.0.join(CHOICES), "[choices]\n\"g/8000-0a0b0c0d\" = 2\n\"h/1\" = 0\n").unwrap();
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::from([("g/8000-0a0b0c0d".into(), 2), ("h/1".into(), 0)])));
        std::fs::write(dir.0.join(CHOICES), "").unwrap();
        assert_eq!(read_choices(&dir.0), Ok(BTreeMap::new()));
        std::fs::write(dir.0.join(CHOICES), "[choices]\n\"g/k\" = 1.5\n").unwrap();
        let e = read_choices(&dir.0).unwrap_err();
        assert!(e.starts_with(&format!("{}: line 2: ", dir.0.join(CHOICES).display())), "{e}");
    }

    #[test]
    fn a_choice_applied() {
        let dir = Tmp::new("apply");
        let s = score();
        let list = soundlearn::encode_list_scores(&[], &[], std::slice::from_ref(&s));
        let key = score_key(&s);
        let folder = dir.0.join("g").join(&key);
        let ok = |_: &[u8], _: usize| Ok(());
        // no takes, no choice: nothing to say; a take: the original
        let none = BTreeMap::new();
        assert_eq!(apply(&dir.0, "g", &list, std::slice::from_ref(&s), &none, &ok), Applied { list: list.clone(), status: vec![None] });
        std::fs::create_dir_all(folder.join("take-1")).unwrap();
        assert!(!has_takes(&folder));
        std::fs::write(folder.join("take-1").join(TAKE_FILE), take(&s, &[(0, 0, 640, 3000, 92)])).unwrap();
        assert!(has_takes(&folder));
        assert_eq!(apply(&dir.0, "g", &list, std::slice::from_ref(&s), &none, &ok).status, [Some(Status::Original)]);
        // chosen: in the list, verified with the list and the score's index
        let chosen = BTreeMap::from([(format!("g/{key}"), 1)]);
        let seen = std::cell::RefCell::new(Vec::new());
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &chosen, &|l, i| {
            seen.borrow_mut().push((l.to_vec(), i));
            Ok(())
        });
        let want = arrange(&s, 1, &take(&s, &[(0, 0, 640, 3000, 92)])).unwrap();
        assert_eq!(got.status, [Some(Status::Take { take: 1, generator: "g 1".into(), dropped: (0, 0) })]);
        assert_eq!(soundlearn::decode_scores(&got.list).unwrap(), [want.rec]);
        assert_eq!(seen.into_inner(), [(got.list.clone(), 0)]);
        // the replay fails, or the take is not there: the original, with the reason
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &chosen, &|_, _| Err("it stops".into()));
        assert_eq!(got, Applied { list: list.clone(), status: vec![Some(Status::Refused { take: 1, why: "the replay: it stops".into() })] });
        let got = apply(&dir.0, "g", &list, std::slice::from_ref(&s), &BTreeMap::from([(format!("g/{key}"), 4)]), &ok);
        assert_eq!(got.list, list);
        assert!(matches!(&got.status[0], Some(Status::Refused { take: 4, why }) if why.starts_with("take-4/arranged.toml: ")), "{got:?}");
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
