// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack: the game pack built from a folder of the owner's own games (or a games
//! file), for `tools/make-personal-pack` (BUILDING.md, "Your own games").
//!
//! # From a folder
//!
//! The folder's `.sna`, `.z80`, `.tap` and `.tzx` files (not its subfolders or hidden files) are
//! listed in `<out>/personal-games.toml`, which has the syntax of `games/games.toml`
//! (docs/controls.md) with `file` an absolute path. Each loadable file gets an entry:
//!
//! - `name`: the file name without its extension, its `(...)` and `[...]` tags, and anything from
//!   a ` - ` on (`Night Raid II - The Return - 48k.tzx` is `Night Raid II`), printable ASCII,
//!   at most 28 characters;
//! - `id`: the name in lower case with every run of other characters a `-`, made unique with
//!   `-2`, `-3` and so on;
//! - no `[game.keys]` table: the default key map.
//!
//! On the next run the file is updated, never rewritten: the existing text stays byte for byte,
//! new loadable files are appended in file name order, and an entry whose file is gone gets a
//! `# missing: <file>` line just above it (and loses it when the file is back). Entries are never
//! reordered or removed; a file that does not load gets no entry (it is listed as left out on
//! every run). A file counts as listed when an entry has it (its folder compared canonical, its
//! name as it is, so a symlink stays the link) or when its quoted path is anywhere in the text,
//! comment lines included: to leave a game out, the owner comments out its lines with `#`, and
//! later runs leave it out too. Deleting an entry works when building from the file.
//!
//! # From a games file
//!
//! Used as written: `file` absolute or relative to the games file's folder. Nothing is rewritten.
//!
//! # The build
//!
//! Every game goes through [`tapeload::prepare`]: snapshots as they are, standard tapes as tapes
//! (after the trap trial), other tapes as the `.z80` of their signal load. 128K files and files
//! that fail to load are left out with the reason. A run with no game in the pack is an error:
//! that every game in the games file is commented out or missing, naming the file, or else the
//! games left out and why. The results go to `<out>/converted/` as
//! `<id>.<sna|z80|tap>`, with a manifest of their own, `<out>/converted/manifest.toml`, from
//! which [`build_pack`] makes `<out>/games.bin`; `<out>/keymaps.json` is the browser page's key
//! table. A pack bigger than the room the app slot leaves it (the limit) is refused, listing
//! every game by packed size, largest first. Each run first removes the outputs of the last one
//! (`games.bin`, `keymaps.json`, the two packages, `sound-report.txt`, `plan.json` and
//! `converted/`), so a failed run leaves no pack or packages that look valid; an output folder
//! that holds any of them without being a personal pack's (no `personal-games.toml`, no
//! `converted/manifest.toml` of ours) is refused.
//!
//! # The music
//!
//! Each packed game's music is learned ([`learn`](soundlearn::learn::learn)) from its converted
//! file, started as the device starts it, under the key script `<file>.keys` beside the game file
//! if there is one (the `music-corpus` format, its times from time 0; one that does not parse is
//! reported and the generic run is used), else the generic run seeded with the converted file's
//! CRC-32. The games are learned in parallel, one thread per core. The pack's sound section holds each
//! game's learned entries and the stored roles of its routines' channels ([`attach_sound`]:
//! learned entries' `instrument[]`, built-in entries' kind-1 records), the game's
//! `[game.instruments]` overrides ([`crate::manifest`]) winning over the learner's analysis, and
//! its recorded scores (kind-2 records; the learner is told the run it listens to, so that its
//! extra runs for the scores skip that run's key); an
//! override that names no learned or built-in routine of the game (or more channels than it has)
//! fails the run, naming the game and the key, and no pack is written. [`table`] ends with one
//! line per game (`<name>: music learned (...)`, `<name>: music recorded (...)` or
//! `<name>: no music learned (<reason>)`), and
//! `<out>/sound-report.txt` holds them with each routine's details and its channels' roles. The
//! results are cached in `<out>/sound-cache/<crc>-<version>.bin` (one CRC-32 over the converted
//! file's data followed by the key script's text, if there is one, and the overrides; the
//! learner's version), which runs keep: an unchanged game is not learned again. A cache file
//! holds the list, the runs its scores were recorded from ([`ScoreRuns`]: the learner's run, its
//! key script's steps or [`choose_run`]'s run and seed, and per score and shadow score that run
//! or the extra key run), the line, the details and the shadow scores. It is used only if its CRC matches, method B's decoder takes
//! its list and it names a run for each score; otherwise the game is
//! learned again (a game with a bad override is not cached). A panic while learning a game gives
//! that game a learner error, and the pack is still built.
//!
//! # Arranged music
//!
//! After the learning (from the cache or not), each game's recorded scores and shadow scores are
//! written to `<out>/arrange/<game-id>/<key>/score.toml`, and the takes `<out>/arrange/choices.toml`
//! chooses (AI takes, and C64 and Spectrum 128 takes as replacement records on their own timeline) go into the pack
//! in place of the scores' notes, or (a shadow score's) after the scores with precedence
//! ([`crate::arrange`]), every run: the cache holds the learned list and the
//! shadow scores, so a changed choice never needs the game learned again. A take is verified by a
//! replay of the run that recorded its score, as the cache keeps it
//! ([`soundlearn::scores::verify`]). In `sound-report.txt` a score with takes or a choice gets a
//! line under its `recorded score` (or `shadow score`) line (the one ending with its key,
//! `(score 8951-1a2b3c4d)`; [`Status::text`]), and the game's line `, 1 arranged` when a take
//! plays; [`table`] ends with a line per take chosen (that plays, or refused and why) and per
//! choice that names no score of the pack (ignored).
//!
//! # The plan
//!
//! [`run_plan`] (`fmzx-pack personal --plan`) is a run up to the pack: the games converted, their
//! music learned and arranged, the pack built to measure it, but a pack over the room is no error,
//! and no `games.bin` or `keymaps.json` is written (those of an earlier run are removed as by any
//! run). It writes `sound-report.txt`, the scores and `<out>/plan.json`: the room and the pack's
//! size in bytes, and each game (the packed ones in pack order, then those left out) with its id,
//! name, file, how it went in, its bytes in the pack with its music (`null` when left out), why it
//! was left out (else `null`), its music line (`null` when left out) and its scores and shadow
//! scores: key, whether a shadow score, length in seconds (intro and one loop pass), and what
//! plays (`original`, `take 2 (<generator>)`, `chip-1 (<source>)`, `ay-1 (<source>)` or
//! `refused: <why>`). A run of
//! either kind removes the plan of an earlier one.

use crate::arrange::{self, Choice, Of, Status, ARRANGE, CHOICES, CLOCK_HZ};
use crate::keymaps::keymaps_json;
use crate::keys::{inputs, parse_keys, Step};
use crate::layout::pack_room;
use crate::manifest::{game_path, parse_manifest, parse_personal_manifest, Game, NAME_MAX};
use crate::pack::{attach_sound, build_pack, sound_list};
use crate::tapeload::{self, Kind, Loaded};
use soundlearn::listen::{candidates, choose_run, run_inputs, Inputs, RunKind, KEYS};
use soundlearn::report::SHADOW_TAIL;
use soundlearn::{decode_scores, encode_list_scores, learn_with, score_key, LearnOptions, ScoreRecord, LEARNER_VERSION};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use zx_sys::{pack_game_at, Machine, SndEngines, PACK_MAX_RAW};

/// The editable games file, in the output folder.
pub const GAMES_FILE: &str = "personal-games.toml";
/// The outputs of a run, removed when the next one starts; `converted/` too (not `sound-cache/`).
const OUTPUTS: [&str; 6] = ["games.bin", "keymaps.json", "fmzx.fwsc", "fmzx-preview.fwsc", SOUND_REPORT, PLAN];
const CONVERTED: &str = "converted";
/// The learned music's details, in the output folder.
pub const SOUND_REPORT: &str = "sound-report.txt";
/// The plan ([`run_plan`]), in the output folder.
pub const PLAN: &str = "plan.json";
/// The learned music's cache, in the output folder.
const SOUND_CACHE: &str = "sound-cache";
const MISSING: &str = "# missing:";
const GAME_HEADER: &str = "[[game]]";

const HEADER: &str = "\
# Your games for tools/make-personal-pack (BUILDING.md, \"Your own games\"), in the syntax of
# games/games.toml (docs/controls.md). Edit a game's name, or give it a [game.keys] table: a
# re-run on the folder keeps your edits, adds new files at the end and marks a game whose file
# is gone with a comment line above it. To leave a game out, comment out its lines with #: a
# re-run on the folder leaves it out too. (Deleting an entry works when you build from this
# file, tools/make-personal-pack <this file>; a re-run on the folder adds it back.)
";

/// One game of the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// The game file's path.
    pub file: String,
    /// How it went in ([`Loaded::how`]: "as is", "tape", "loaded", "loaded (own loader)" and
    /// the like), or "left out".
    pub how: String,
    /// Its bytes in the pack (LZ4 or stored); `None` when left out.
    pub packed: Option<u32>,
    /// Why it was left out.
    pub reason: Option<String>,
}

/// A run's outcome: the games in games file order, then the folder's files that did not load;
/// the pack's size, the room for it and the room left beside it (0 when it does not fit, which
/// only a plan allows), in bytes; the learned music's line for each packed game, in pack order
/// (`<name>: music learned (...)`); a line per take chosen and per choice that names no score
/// (see the module docs).
#[derive(Clone, Debug)]
pub struct Report {
    pub rows: Vec<Row>,
    pub used: u32,
    pub room: u32,
    pub free: u32,
    pub sound: Vec<String>,
    pub arranged: Vec<String>,
}

/// Builds the personal pack from `input` (a folder or a games file) into `out`; see the module
/// docs. The limit is the room for the pack beside the smallest possible image.
pub fn run(input: &Path, out: &Path) -> Result<Report, String> {
    run_with_limit(input, out, None)
}

/// [`run`] with the pack's limit in bytes: `fmzx-pack personal` passes the room the app images
/// leave ([`pack_room`]); `None` is the room beside an empty image.
#[doc(hidden)]
pub fn run_with_limit(input: &Path, out: &Path, limit: Option<u32>) -> Result<Report, String> {
    build(input, out, limit, false)
}

/// The plan: [`run_with_limit`] up to the pack, which may be over the limit, writing
/// `<out>/plan.json` and no pack (see the module docs).
pub fn run_plan(input: &Path, out: &Path, limit: Option<u32>) -> Result<Report, String> {
    build(input, out, limit, true)
}

fn build(input: &Path, out: &Path, limit: Option<u32>, plan: bool) -> Result<Report, String> {
    let limit = limit.unwrap_or_else(|| pack_room(0));
    let shown = |p: &Path| p.display().to_string();
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", shown(out)))?;
    let out = out.canonicalize().map_err(|e| format!("{}: {e}", shown(out)))?;
    let input = input.canonicalize().map_err(|e| format!("{}: {e}", shown(input)))?;
    if input == out {
        return Err(format!("{}: the output folder must not be the games folder", shown(&out)));
    }
    if !input.is_dir() && !input.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")) {
        return Err(format!("{}: give a folder of games or a games file (.toml)", shown(&input)));
    }
    let conv = out.join(CONVERTED);
    let present: Vec<&str> =
        OUTPUTS.iter().copied().filter(|f| out.join(f).exists()).chain(conv.exists().then_some(CONVERTED)).collect();
    if !present.is_empty() && !is_ours(&out) {
        return Err(format!(
            "{}: the output folder holds {} but no {GAMES_FILE} (nor a {CONVERTED}/manifest.toml of ours): it is not a \
             personal pack's folder, and a run clears those files first. Give another folder with -o.",
            shown(&out),
            present.join(", ")
        ));
    }
    for f in OUTPUTS {
        let p = out.join(f);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", shown(&p)))?;
        }
    }
    if conv.exists() {
        std::fs::remove_dir_all(&conv).map_err(|e| format!("{}: {e}", shown(&conv)))?;
    }

    let mut prepared: HashMap<PathBuf, Result<Loaded, String>> = HashMap::new();
    let mut rows = Vec::new();
    let mut left_new = Vec::new();
    let mut no_files = false;
    let (games, games_dir, games_file) = if input.is_dir() {
        let path = out.join(GAMES_FILE);
        let files = game_files(&input)?;
        no_files = files.is_empty();
        let text = update_games_file(&path, &files, &mut prepared, &mut left_new)?;
        (entries(&text).map_err(|e| format!("{}: {e}", shown(&path)))?, out.clone(), path)
    } else {
        let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", shown(&input)))?;
        let games = parse_personal_manifest(&text).map_err(|e| format!("{}: {e}", shown(&input)))?.game;
        (games, input.parent().unwrap_or(Path::new("/")).to_path_buf(), input.clone())
    };

    // each game prepared and written to converted/
    std::fs::create_dir_all(&conv).map_err(|e| format!("{}: {e}", shown(&conv)))?;
    let mut packed: Vec<(Game, String, usize)> = Vec::new(); // game, converted file, row
    let mut jobs: Vec<Job> = Vec::new(); // the same games, for learning their music
    for g in &games {
        let path = game_path(&games_dir, g);
        let mut row =
            Row { name: g.name.clone(), file: shown(&path), how: "left out".into(), packed: None, reason: None };
        if !path.is_file() {
            row.reason = Some(MISSING_REASON.into());
            rows.push(row);
            continue;
        }
        let key = path.clone();
        let got = prepared.entry(key.clone()).or_insert_with(|| tapeload::prepare(&key));
        match got {
            Err(e) => row.reason = Some(reason(e)),
            Ok(l) if l.data.len() > PACK_MAX_RAW as usize => {
                row.reason = Some(format!("{} bytes, over the pack's {PACK_MAX_RAW}-byte limit for a game", l.data.len()));
            }
            Ok(l) => {
                let file = format!("{}.{}", g.id, converted_ext(l, &key));
                let at = conv.join(&file);
                std::fs::write(&at, &l.data).map_err(|e| format!("{}: {e}", shown(&at)))?;
                row.how = l.how.clone();
                jobs.push(Job {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    data: l.data.clone(),
                    file: file.clone(),
                    keys: key.with_extension("keys"),
                    instruments: g.instruments.clone(),
                });
                packed.push((g.clone(), file, rows.len()));
            }
        }
        rows.push(row);
    }
    // tried and failed: not missing, not a commented-out entry (which is no game of the run)
    let failed = rows.iter().any(|w| w.reason.is_some() && !is_missing(w)) || !left_new.is_empty();
    rows.extend(left_new);
    if packed.is_empty() {
        if !failed {
            return Err(if no_files && rows.is_empty() {
                format!("no game files (.sna, .z80, .tap or .tzx) in {}", shown(&input))
            } else {
                format!(
                    "every game in {} is commented out or missing: nothing to build\n{}",
                    games_file.display(),
                    left_out_list(&rows)
                )
            });
        }
        return Err(format!("no game could be loaded:\n{}", left_out_list(&rows)));
    }

    // the picker shows the pack's order: by name, case-insensitively (the games file and the
    // table keep the file's order; a re-run adds new files at the end)
    let mut both: Vec<_> = packed.into_iter().zip(jobs).collect();
    both.sort_by_cached_key(|(p, _)| p.0.name.to_lowercase());
    let (packed, jobs): (Vec<_>, Vec<_>) = both.into_iter().unzip();

    // each game's music, then the pack from the converted files' own manifest, with the sound
    // section
    let mut sounds = learn_all(&jobs, &out.join(SOUND_CACHE));
    let manifest_path = conv.join("manifest.toml");
    let text = manifest_text(&packed);
    std::fs::write(&manifest_path, &text).map_err(|e| format!("{}: {e}", shown(&manifest_path)))?;
    // after the converted files' manifest, which keeps the folder ours for the next run
    if let Some((j, e)) = jobs.iter().zip(&sounds).find_map(|(j, s)| s.override_error.as_ref().map(|e| (j, e))) {
        return Err(format!("game {:?}: {e}", j.name));
    }
    let (arranged, notes, plan_scores) = arrange_all(&out.join(ARRANGE), &jobs, &mut sounds)?;
    let mut pack = build_pack(&manifest_path)?;
    attach_sound(&mut pack, &sounds.iter().map(|s| s.list.clone()).collect::<Vec<_>>())?;
    let mut with_music = Vec::new(); // per packed game: its bytes and its list's
    for (i, (_, _, r)) in packed.iter().enumerate() {
        let g = pack_game_at(&pack, i as u32).map_err(|e| format!("the pack just built does not read back (error {e})"))?;
        rows[*r].packed = Some(g.stored_len());
        let music = sound_list(&pack, i as u32).map_or(0, <[u8]>::len);
        with_music.push(g.stored_len().saturating_add(u32::try_from(music).unwrap_or(u32::MAX)));
    }
    let used = u32::try_from(pack.len()).unwrap_or(u32::MAX);
    if used > limit && !plan {
        return Err(too_big(&rows, used, limit, &games_file, input.is_dir()));
    }
    let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", shown(&manifest_path)))?;
    let keymaps = keymaps_json(&manifest)?;
    let sound: Vec<String> = jobs.iter().zip(&sounds).map(|(j, s)| format!("{}: {}", j.name, s.line)).collect();
    let mut details = String::from(REPORT_HEAD);
    if !notes.is_empty() {
        details += &format!("\n{}", notes.iter().map(|n| format!("{n}\n")).collect::<String>());
    }
    for ((j, s), l) in jobs.iter().zip(&sounds).zip(&sound) {
        details += &format!("\n{l}\n{}{}", keys_note(j, s), s.details);
    }
    let plan_text = plan.then(|| {
        // the rows' ids: each games file entry's (its row has its index), then one for each file
        // that has no entry, from its name
        let mut taken: HashSet<String> = games.iter().map(|g| g.id.clone()).collect();
        let ids: Vec<String> = (0..rows.len())
            .map(|i| match games.get(i) {
                Some(g) => g.id.clone(),
                None => {
                    let id = unique_id(&rows[i].name, &taken);
                    taken.insert(id.clone());
                    id
                }
            })
            .collect();
        let packed_games = packed.iter().enumerate().map(|(k, (_, _, r))| (*r, Some((with_music[k], sounds[k].line.as_str(), &plan_scores[k][..]))));
        let left = rows.iter().enumerate().filter(|(_, w)| w.packed.is_none()).map(|(r, _)| (r, None));
        let objects: Vec<String> = packed_games.chain(left).map(|(r, got)| plan_game(&ids[r], &rows[r], got)).collect();
        format!("{{\n  \"room\": {limit},\n  \"used\": {used},\n  \"games\": [{}\n  ]\n}}\n", objects.join(","))
    });
    let mut writes: Vec<(&str, &[u8])> = vec![(SOUND_REPORT, details.as_bytes())];
    match &plan_text {
        Some(t) => writes.push((PLAN, t.as_bytes())),
        None => writes.extend([("keymaps.json", keymaps.as_bytes()), ("games.bin", &pack[..])]),
    }
    for (name, bytes) in writes {
        let p = out.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", shown(&p)))?;
    }
    let free = limit.saturating_sub(used);
    Ok(Report { rows, used, room: limit, free, sound, arranged: arranged.into_iter().chain(notes).collect() })
}

/// A score in the plan: its key, whether a shadow score, its length in seconds and what plays.
#[derive(Clone, Debug)]
struct PlanScore {
    key: String,
    shadow: bool,
    seconds: f64,
    status: String,
}

/// What plays for a score, as the plan says it (see the module docs).
fn plan_status(st: Option<&Status>) -> String {
    match st {
        Some(Status::Take { take: Choice::Take(n), generator, .. }) => format!("take {n} ({generator})"),
        Some(Status::Take { take, generator, .. }) => format!("{} ({generator})", take.dir()),
        Some(Status::Refused { why, .. }) => format!("refused: {why}"),
        Some(Status::Original) | None => "original".into(),
    }
}

/// A game's object in `plan.json`: `got` its bytes with its music, its music line and its
/// scores, when packed.
fn plan_game(id: &str, w: &Row, got: Option<(u32, &str, &[PlanScore])>) -> String {
    let opt = |s: Option<&str>| s.map_or("null".to_string(), json_string);
    let scores: Vec<String> = got
        .map_or(&[][..], |g| g.2)
        .iter()
        .map(|p| {
            format!(
                "\n        {{\"key\": {}, \"shadow\": {}, \"seconds\": {:.1}, \"status\": {}}}",
                json_string(&p.key),
                p.shadow,
                p.seconds,
                json_string(&p.status)
            )
        })
        .collect();
    let scores = if scores.is_empty() { "[]".to_string() } else { format!("[{}\n      ]", scores.join(",")) };
    format!(
        "\n    {{\n      \"id\": {},\n      \"name\": {},\n      \"file\": {},\n      \"how\": {},\n      \"packed\": {},\n      \
         \"reason\": {},\n      \"music\": {},\n      \"scores\": {scores}\n    }}",
        json_string(id),
        json_string(&w.name),
        json_string(&w.file),
        json_string(&w.how),
        got.map_or("null".to_string(), |g| g.0.to_string()),
        opt(w.reason.as_deref()),
        opt(got.map(|g| g.1)),
    )
}

/// A JSON string.
fn json_string(s: &str) -> String {
    let mut o = String::from('"');
    for c in s.chars() {
        match c {
            '"' => o += "\\\"",
            '\\' => o += "\\\\",
            '\n' => o += "\\n",
            '\r' => o += "\\r",
            '\t' => o += "\\t",
            c if u32::from(c) < 0x20 => o += &format!("\\u{:04x}", u32::from(c)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

const REPORT_HEAD: &str = "\
# The music tools/make-personal-pack learned for each game (BUILDING.md, \"Your own games\"), and
# per routine: the speaker OUT it was read from, the routine, each channel's half period in
# T-states (an input in a product counts a loop: 0 counts 256, 65,536 for a word), the gates'
# figures, whether it was learned or why not, and its channels' stored roles in the FM synth
# (from the analysis of the run, or your override in [game.instruments]). A heard routine whose
# code was not used is observed instead: each channel's value, how it gives the pitch, its score
# (the share of the music in which that pitch was heard), and whether it was learned or why not.
# Music no routine gives that played long enough is recorded as scores instead, a score per tune
# (from the run listened to and, when that gives one, from a run per key: the key named on its
# line): from the edge that starts it, its length, its intro and loop when its music loops (the
# loop stored once, played round for as long as the beeper keeps to it), voices, notes and bytes
# (FM: engines plays it in step with the beeper), or why it has none. Each recorded score's notes
# are written to arrange/<game>/<key>/score.toml for tools/arrange-music; a score with takes, or
# a take chosen in arrange/choices.toml, says under its line which plays: `arranged: take 2
# (<what made it>)` or `arranged: C64 take chip-1 (<where the tune comes from>)` (with any notes
# dropped for frames over 8 events), `arranged: original`, or `arranged: take 2 refused: <why>`
# (the original plays). Music a learned or built-in routine plays
# gets a shadow score (`shadow score: ...; packed only with a chosen take`): its notes are written
# out the same way, and it goes into the pack only with a take chosen for it, which then plays in
# place of the routine (`arranged: the live version (no take chosen)` until then).
";

/// A packed game, for learning its music: its id and name, its converted file's bytes and name,
/// where its key script would be, and its overrides of the stored roles.
struct Job {
    id: String,
    name: String,
    data: Vec<u8>,
    file: String,
    keys: PathBuf,
    instruments: BTreeMap<u16, Vec<u8>>,
}

/// What was learned of a game: its entry list (None: none), its line without the name, its
/// details, the key script's problem if it had one, and an override that does not fit
/// ([`soundlearn::GameSound::override_error`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
    pub list: Option<Vec<u8>>,
    /// The shadow scores ([`soundlearn::GameSound::shadows`]): not in the list unless a take is
    /// chosen for one.
    pub shadows: Vec<ScoreRecord>,
    /// The runs its scores were recorded from (None when nothing was learned).
    pub runs: Option<ScoreRuns>,
    pub line: String,
    pub details: String,
    pub keys_error: Option<String>,
    pub override_error: Option<String>,
}

/// The key script's problem as a details line, if any.
fn keys_note(j: &Job, s: &Sound) -> String {
    s.keys_error.as_ref().map_or(String::new(), |e| format!("  {}: {e}; the generic run was used\n", j.keys.display()))
}

/// The machine of a converted file, started as the device starts it.
pub fn machine(file: &str, data: &[u8]) -> Result<Machine, String> {
    let mut m = Machine::new();
    match Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("sna") => m.load_sna(data).map_err(|e| format!("not a loadable .sna (error {e})"))?,
        Some("z80") => m.load_z80(data).map_err(|e| format!("not a loadable .z80 (error {e})"))?,
        _ => {
            m.tape_insert(data).map_err(|e| format!("not a valid .tap (error {e})"))?;
            m.tape_autostart();
        }
    }
    Ok(m)
}

/// The music of every job, in parallel (one thread per core), each from the cache when it holds
/// it (see the module docs).
fn learn_all(jobs: &[Job], cache: &Path) -> Vec<Sound> {
    let _ = std::fs::create_dir_all(cache);
    let next = AtomicUsize::new(0);
    let done: Vec<Mutex<Option<Sound>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(jobs.len()).max(1);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let (Some(job), Some(slot)) = (jobs.get(i), done.get(i)) else { break };
                let got = isolated(|| learn_cached(job, cache));
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(got);
            });
        }
    });
    // every job's slot is filled (its panics caught by `isolated`), so the default is never used
    done.into_iter().map(|d| d.into_inner().unwrap_or_else(|e| e.into_inner()).unwrap_or_else(|| learner_error("no result"))).collect()
}

/// `f`'s result, or a learner error if it panics: one game's failure never fails the pack.
fn isolated(f: impl FnOnce() -> Sound) -> Sound {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let why = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned());
        learner_error(why.as_deref().unwrap_or("a panic"))
    })
}

/// A game's music when learning it failed.
fn learner_error(why: &str) -> Sound {
    Sound {
        list: None,
        shadows: Vec::new(),
        runs: None,
        line: format!("no music learned (learner error: {why})"),
        details: format!("  learner error: {why}\n"),
        keys_error: None,
        override_error: None,
    }
}

/// Milliseconds per Spectrum frame, for the key scripts' times.
const FRAME_MS: u64 = 20;
/// The start of a cache file.
const CACHE_MAGIC: &[u8; 8] = b"FMZXSND4";

/// A game's music: from the cache, or learned and then cached.
fn learn_cached(job: &Job, cache: &Path) -> Sound {
    learn_music(&job.file, &job.data, &job.keys, &job.instruments, cache)
}

/// The music of a game as the personal pack learns it: `file` and `data` its converted file
/// ([`tapeload::prepare`]'s data, named with [`converted_ext`]), `keys` where its key script
/// would be, `instruments` its overrides of the stored roles ([`crate::manifest::Game`]), `cache`
/// the cache folder (it must exist). From the cache, or learned under the key script's steps (or
/// the generic run, seeded by the converted data's CRC-32) and then cached (not with an override
/// that does not fit). `music-corpus` learns through this too, so both learn the same.
pub fn learn_music(file: &str, data: &[u8], keys: &Path, instruments: &BTreeMap<u16, Vec<u8>>, cache: &Path) -> Sound {
    let script = std::fs::read_to_string(keys).ok();
    let (steps, keys_error) = match script.as_deref().map(parse_keys) {
        Some(Ok(steps)) => (Some(steps), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let path = cache_path(data, script.as_deref(), instruments, cache);
    if let Some((list, runs, line, details, shadows)) = std::fs::read(&path).ok().as_deref().and_then(cache_decode) {
        return Sound { list, shadows, runs, line, details, keys_error, override_error: None };
    }
    let got = learn_converted(file, data, steps, instruments);
    // written whole, then renamed: a half-written file is never read
    let tmp = cache.join(format!("{file}.tmp"));
    if got.override_error.is_none()
        && std::fs::write(&tmp, cache_encode(got.list.as_deref(), &got.shadows, got.runs.as_ref(), &got.line, &got.details)).is_ok()
        && std::fs::rename(&tmp, &path).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
    Sound { keys_error, ..got }
}

/// The cache file [`learn_music`] reads and writes for a game: `data` its converted file's bytes,
/// `script` its key script's text (if it has one), `instruments` its overrides, `cache` the cache
/// folder (see the module docs).
pub fn cache_path(data: &[u8], script: Option<&str>, instruments: &BTreeMap<u16, Vec<u8>>, cache: &Path) -> PathBuf {
    let mut crc = crc32fast::Hasher::new();
    crc.update(data);
    if let Some(t) = script {
        crc.update(t.as_bytes());
    }
    for (out, roles) in instruments {
        crc.update(&out.to_le_bytes());
        crc.update(&[roles.len() as u8]);
        crc.update(roles);
    }
    cache.join(format!("{:08x}-{LEARNER_VERSION}.bin", crc.finalize()))
}

/// Learns a game's music under its key script's steps, or the run [`choose_run`] chooses, with
/// its overrides of the stored roles: its entry list (None: none), the runs its scores were
/// recorded from, line, details and an override that does not fit (no key script problem).
fn learn_converted(file: &str, data: &[u8], steps: Option<Vec<Step>>, instruments: &BTreeMap<u16, Vec<u8>>) -> Sound {
    if let Err(e) = machine(file, data) {
        let line = format!("no music learned (the game does not load: {e})");
        return Sound { list: None, shadows: Vec::new(), runs: None, line, details: String::new(), keys_error: None, override_error: None };
    }
    // it loaded just now, so it loads again: the fallback is never used
    let load = || machine(file, data).unwrap_or_else(|_| Machine::new());
    let mut run_line = String::new();
    let mut run_kind = None;
    let (run, learner): (Inputs, LearnerRun) = match steps {
        Some(steps) => {
            let kept = steps.clone();
            (Box::new(move |f| inputs(&steps, u64::from(f) * FRAME_MS)), LearnerRun::Script(kept))
        }
        None => {
            let seed = u64::from(crc32fast::hash(data));
            let (run, kind, won, generic) = choose_run(&load, seed);
            run_kind = Some(kind);
            if kind != RunKind::Generic {
                run_line = format!("  run: {} (it heard {won} music frames, the generic run {generic})\n", kind.describe());
            }
            (run, LearnerRun::Kind(kind, seed))
        }
    };
    let opts = LearnOptions { instruments: instruments.clone(), run: run_kind, ..LearnOptions::default() };
    let got = learn_with(&load, &run, &opts);
    let list = (!got.entries.is_empty() || !got.roles.is_empty() || !got.scores.is_empty()).then(|| encode_list_scores(&got.entries, &got.roles, &got.scores));
    let runs = Some(ScoreRuns { learner, scores: got.score_runs, shadows: got.shadow_runs });
    // each shadow score as its record reads back (as the list's scores and the cache's do)
    let shadows = got.shadows.iter().map(|r| decode_scores(&encode_list_scores(&[], &[], std::slice::from_ref(r))).map(|mut d| d.remove(0)).unwrap_or_else(|_| r.clone())).collect();
    Sound { list, shadows, runs, line: got.line, details: run_line + &got.details, keys_error: None, override_error: got.override_error }
}

/// [`arrange_all`]'s lines for the takes chosen, its notes, and each game's scores for the plan.
type ArrangedAll = (Vec<String>, Vec<String>, Vec<Vec<PlanScore>>);

/// Writes every game's scores out and puts the chosen takes into the lists, the games' lines and
/// details (see the module docs); `dir` the arranged music's folder. Returns the lines for the
/// takes chosen, the notes on choices that name no score, and each game's scores and shadow
/// scores as the plan gives them. Err: a choices file that does not read, or a score file that
/// cannot be written.
fn arrange_all(dir: &Path, jobs: &[Job], sounds: &mut [Sound]) -> Result<ArrangedAll, String> {
    let choices = arrange::read_choices(dir)?;
    let mut known: HashSet<String> = HashSet::new();
    let mut lines = Vec::new();
    let mut plan: Vec<Vec<PlanScore>> = vec![Vec::new(); jobs.len()];
    for ((j, s), planned) in jobs.iter().zip(sounds.iter_mut()).zip(plan.iter_mut()) {
        // a game with shadow scores alone: an empty list, kept only if a take goes in
        let Some(list) = s.list.clone().or_else(|| (!s.shadows.is_empty()).then(|| encode_list_scores(&[], &[], &[]))) else { continue };
        // the list went through method B's decoder: its scores read back
        let Ok(scores) = decode_scores(&list) else { continue };
        if scores.is_empty() && s.shadows.is_empty() {
            continue;
        }
        arrange::export(dir, &j.id, &j.name, &scores)?;
        arrange::export(dir, &j.id, &j.name, &s.shadows)?;
        known.extend(scores.iter().chain(&s.shadows).map(|r| format!("{}/{}", j.id, score_key(r))));
        let runs = s.runs.clone();
        let got = arrange::apply(dir, &j.id, &list, &scores, &s.shadows, &choices, &|l, i, of| verify_take(j, runs.as_ref(), l, i, of));
        let all = scores.iter().zip(&got.status).map(|(r, st)| (r, st, false)).chain(s.shadows.iter().zip(&got.shadow_status).map(|(r, st)| (r, st, true)));
        for (rec, st, shadow) in all {
            let seconds = (f64::from(rec.intro_t) + f64::from(rec.loop_t)) / f64::from(CLOCK_HZ);
            planned.push(PlanScore { key: score_key(rec), shadow, seconds, status: plan_status(st.as_ref()) });
            let (what, live) = if shadow { ("shadow score", "the live version") } else { ("score", "the original") };
            match st {
                Some(Status::Take { take, generator, .. }) => {
                    lines.push(format!("{}: {what} {} plays {take} ({generator})", j.name, score_key(rec)))
                }
                Some(Status::Refused { take, why }) => {
                    lines.push(format!("{}: {what} {}: {take} refused, {live} plays: {why}", j.name, score_key(rec)))
                }
                _ => {}
            }
        }
        let n = got.status.iter().chain(&got.shadow_status).filter(|st| matches!(st, Some(Status::Take { .. }))).count();
        if n > 0 {
            if let Some(at) = s.line.rfind(')') {
                s.line.insert_str(at, &format!(", {n} arranged"));
            }
        }
        s.details = with_status(&s.details, &scores, &got.status, &s.shadows, &got.shadow_status);
        if s.list.is_some() || got.list != list {
            s.list = Some(got.list);
        }
    }
    let notes = choices
        .keys()
        .filter(|k| !known.contains(*k))
        .map(|k| format!("{}: {k:?} names no score of the pack; ignored", dir.join(CHOICES).display()))
        .collect();
    Ok((lines, notes, plan))
}

/// A game's details with each score's status under its `recorded score` line (the line that
/// ends with its key, `(score 8951-1a2b3c4d)`), and each shadow score's under its `shadow score`
/// line (`(score 8951-1a2b3c4d; packed only with a chosen take)`); one not found at the end,
/// with its key.
fn with_status(details: &str, scores: &[ScoreRecord], status: &[Option<Status>], shadows: &[ScoreRecord], shadow_status: &[Option<Status>]) -> String {
    let tag = |rec: &ScoreRecord, shadow: bool| format!("(score {}{})", score_key(rec), if shadow { SHADOW_TAIL } else { "" });
    let keyed: Vec<(String, &Status, bool)> = scores
        .iter()
        .zip(status)
        .map(|(r, st)| (r, st, false))
        .chain(shadows.iter().zip(shadow_status).map(|(r, st)| (r, st, true)))
        .filter_map(|(rec, st, shadow)| st.as_ref().map(|st| (tag(rec, shadow), st, shadow)))
        .collect();
    let mut placed = vec![false; keyed.len()];
    let mut out = String::new();
    for l in details.lines() {
        out += &format!("{l}\n");
        if l.starts_with("    recorded score") || l.starts_with("    shadow score") {
            for (k, (tag, st, shadow)) in keyed.iter().enumerate() {
                if !placed[k] && l.ends_with(tag.as_str()) {
                    placed[k] = true;
                    out += &format!("      {}\n", st.text(*shadow));
                }
            }
        }
    }
    for ((tag, st, shadow), _) in keyed.iter().zip(&placed).filter(|(_, p)| !**p) {
        let key = tag.trim_start_matches("(score ").trim_end_matches(')').trim_end_matches(SHADOW_TAIL);
        let what = if *shadow { "shadow score" } else { "score" };
        out += &format!("  {what} {key}: {}\n", st.text(*shadow));
    }
    out
}

/// Verifies by replay score `index` of `list`, a game's list with a take in it, in the run that
/// recorded the score `of` (`runs`, kept with the learned list). Err: why not.
fn verify_take(j: &Job, runs: Option<&ScoreRuns>, list: &[u8], index: usize, of: Of) -> Result<(), String> {
    verify_in_run(&j.file, &j.data, runs, list, index, of)
}

/// [`verify_take`] for the converted file `file` (`data` its bytes): the check the pack applies
/// to a take. Err: why it does not play.
pub fn verify_in_run(file: &str, data: &[u8], runs: Option<&ScoreRuns>, list: &[u8], index: usize, of: Of) -> Result<(), String> {
    let inputs = runs.and_then(|r| r.inputs(of)).ok_or("the run that recorded it is not known")?;
    let load = || machine(file, data).unwrap_or_else(|_| Machine::new());
    soundlearn::scores::verify(&load, &inputs, list, index, true)
}

/// The run a game's music was learned under: its key script's steps, or one of
/// [`choose_run`]'s runs with its seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearnerRun {
    Script(Vec<Step>),
    Kind(RunKind, u64),
}

impl LearnerRun {
    /// The run's inputs.
    pub fn inputs(&self) -> Inputs {
        match self {
            LearnerRun::Kind(kind, seed) => run_inputs(*kind, *seed),
            LearnerRun::Script(steps) => {
                let steps = steps.clone();
                Box::new(move |f| inputs(&steps, u64::from(f) * FRAME_MS))
            }
        }
    }
}

/// The runs a game's scores were recorded from: the learner's run, and per score (in the list's
/// order) None for it, else the extra run ([`RunKind::Key`]); per shadow score the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreRuns {
    pub learner: LearnerRun,
    pub scores: Vec<Option<RunKind>>,
    pub shadows: Vec<Option<RunKind>>,
}

impl ScoreRuns {
    /// The inputs of the run that recorded the score `of`; None when there is no such score.
    pub fn inputs(&self, of: Of) -> Option<Inputs> {
        let run = match of {
            Of::Score(k) => self.scores.get(k)?,
            Of::Shadow(k) => self.shadows.get(k)?,
        };
        Some(match run {
            Some(kind) => run_inputs(*kind, 0),
            None => self.learner.inputs(),
        })
    }

    /// Its bytes in a cache file: the learner's run (0, the steps' count (u32) and each step's
    /// action, time and hold (u8, u64, u64); or 1, the kind's code and the seed (u64)), then
    /// the scores' count (u32) and each one's run (255 the learner's, else the kind's code), then
    /// the shadow scores' the same way; numbers little-endian.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match &self.learner {
            LearnerRun::Script(steps) => {
                b.push(0);
                b.extend((steps.len() as u32).to_le_bytes());
                for st in steps {
                    b.push(st.action);
                    b.extend(st.at.to_le_bytes());
                    b.extend(st.hold.to_le_bytes());
                }
            }
            LearnerRun::Kind(kind, seed) => {
                b.push(1);
                b.push(kind_code(*kind));
                b.extend(seed.to_le_bytes());
            }
        }
        for runs in [&self.scores, &self.shadows] {
            b.extend((runs.len() as u32).to_le_bytes());
            b.extend(runs.iter().map(|k| k.map_or(LEARNER_CODE, kind_code)));
        }
        b
    }

    /// [`ScoreRuns::encode`]'s bytes read back; None if they are not that.
    pub fn decode(b: &[u8]) -> Option<ScoreRuns> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let got = b.get(at..at + n)?;
            at += n;
            Some(got)
        };
        let u32_of = |x: &[u8]| u32::from_le_bytes(x.try_into().unwrap_or_default());
        let u64_of = |x: &[u8]| u64::from_le_bytes(x.try_into().unwrap_or_default());
        let learner = match take(1)?[0] {
            0 => {
                let n = u32_of(take(4)?) as usize;
                let mut steps = Vec::with_capacity(n.min(b.len()));
                for _ in 0..n {
                    let action = take(1)?[0];
                    let (at_ms, hold) = (u64_of(take(8)?), u64_of(take(8)?));
                    steps.push(Step { action, at: at_ms, hold });
                }
                LearnerRun::Script(steps)
            }
            1 => {
                let kind = kind_of(take(1)?[0])?;
                LearnerRun::Kind(kind, u64_of(take(8)?))
            }
            _ => return None,
        };
        let mut runs = || -> Option<Vec<Option<RunKind>>> {
            let n = u32_of(take(4)?) as usize;
            take(n)?.iter().map(|&c| if c == LEARNER_CODE { Some(None) } else { kind_of(c).map(Some) }).collect()
        };
        let (scores, shadows) = (runs()?, runs()?);
        (at == b.len()).then_some(ScoreRuns { learner, scores, shadows })
    }
}

/// A score recorded in the learner's run, in a cache file.
const LEARNER_CODE: u8 = 255;

/// A run kind's code in a cache file: 0 the generic run, 1 nothing pressed, 16 + i start key i
/// once, 64 + i key i once.
fn kind_code(k: RunKind) -> u8 {
    match k {
        RunKind::Generic => 0,
        RunKind::Nothing => 1,
        RunKind::Once(i) => 16 + i as u8,
        RunKind::Key(i) => 64 + i as u8,
    }
}

/// The run kind of a code ([`kind_code`]); None for none.
fn kind_of(c: u8) -> Option<RunKind> {
    let kind = match c {
        0 => RunKind::Generic,
        1 => RunKind::Nothing,
        16..=63 => RunKind::Once(usize::from(c - 16)),
        64..=254 => RunKind::Key(usize::from(c - 64)),
        _ => return None,
    };
    let known = candidates().contains(&kind) || (0..KEYS.len()).any(|i| RunKind::Key(i) == kind);
    known.then_some(kind)
}

/// A cache file's contents: the entry list (None: none), the runs its scores were recorded
/// from, the line, the details and the shadow scores.
pub type Cached = (Option<Vec<u8>>, Option<ScoreRuns>, String, String, Vec<ScoreRecord>);

/// A cache file: the magic, the CRC-32 of the rest (u32, little-endian), then the entry list,
/// the runs ([`ScoreRuns::encode`]), the line, the details and the shadow scores (each an entry
/// list of the one record), each a u32 length and its bytes; an empty list or runs is none.
pub fn cache_encode(list: Option<&[u8]>, shadows: &[ScoreRecord], runs: Option<&ScoreRuns>, line: &str, details: &str) -> Vec<u8> {
    let runs = runs.map_or_else(Vec::new, ScoreRuns::encode);
    let shadows: Vec<Vec<u8>> = shadows.iter().map(|r| encode_list_scores(&[], &[], std::slice::from_ref(r))).collect();
    let mut payload = Vec::new();
    for part in [list.unwrap_or(&[]), &runs, line.as_bytes(), details.as_bytes()].into_iter().chain(shadows.iter().map(Vec::as_slice)) {
        payload.extend((part.len() as u32).to_le_bytes());
        payload.extend(part);
    }
    let mut b = CACHE_MAGIC.to_vec();
    b.extend(crc32fast::hash(&payload).to_le_bytes());
    b.extend(payload);
    b
}

/// A cache file's contents; None if it is not one, its CRC does not match, method B's decoder
/// refuses its list or a shadow score's (the cache is not trusted: a list goes into the pack only
/// as the decoder takes it), or its runs do not name a run for each of the list's scores and
/// each shadow score.
pub fn cache_decode(b: &[u8]) -> Option<Cached> {
    let rest = b.strip_prefix(CACHE_MAGIC)?;
    let (crc, mut rest) = (u32::from_le_bytes(rest.get(..4)?.try_into().ok()?), rest.get(4..)?);
    if crc32fast::hash(rest) != crc {
        return None;
    }
    let part = |rest: &mut &[u8]| -> Option<Vec<u8>> {
        let len = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let bytes = rest.get(4..4 + len)?.to_vec();
        *rest = &rest[4 + len..];
        Some(bytes)
    };
    let (list, runs, line, details) = (part(&mut rest)?, part(&mut rest)?, part(&mut rest)?, part(&mut rest)?);
    let mut shadows = Vec::new();
    while !rest.is_empty() {
        let one = part(&mut rest)?;
        let rec = match SndEngines::with_learned(&one).ok().and(decode_scores(&one).ok()).as_deref() {
            Some([rec]) if rec.precedence => rec.clone(),
            _ => return None,
        };
        shadows.push(rec);
    }
    if !list.is_empty() && SndEngines::with_learned(&list).is_err() {
        return None;
    }
    let runs = if runs.is_empty() { None } else { Some(ScoreRuns::decode(&runs)?) };
    let scores = if list.is_empty() { 0 } else { decode_scores(&list).ok()?.len() };
    if (scores > 0 || !shadows.is_empty()) && runs.as_ref().is_none_or(|r| r.scores.len() != scores || r.shadows.len() != shadows.len()) {
        return None;
    }
    Some(((!list.is_empty()).then_some(list), runs, String::from_utf8(line).ok()?, String::from_utf8(details).ok()?, shadows))
}

/// The first line of the converted files' manifest, which marks an output folder as ours.
const MANIFEST_HEAD: &str = "# Written by tools/make-personal-pack from personal-games.toml; it is rewritten on each run.";

/// Whether `out` is a personal pack's folder: it has the games file, or the converted files'
/// manifest of an earlier run (from a games file, the games file lives elsewhere).
fn is_ours(out: &Path) -> bool {
    out.join(GAMES_FILE).is_file()
        || std::fs::read_to_string(out.join(CONVERTED).join("manifest.toml")).is_ok_and(|t| t.lines().next() == Some(MANIFEST_HEAD))
}

const MISSING_REASON: &str = "missing: the file is not there";

fn is_missing(w: &Row) -> bool {
    w.reason.as_deref() == Some(MISSING_REASON)
}

/// The table `tools/make-personal-pack` prints: each game, how it went in, its packed size and
/// its file; the space used and left; the games left out and why; then each packed game's
/// learned music line.
pub fn table(r: &Report) -> String {
    let how = how_width(&r.rows);
    let mut s = format!("  {:<28} {:<how$} {:>9}  file\n", "name", "how", "packed");
    for w in r.rows.iter().filter(|w| w.packed.is_some()) {
        s += &game_line(w, how);
    }
    s += &if r.used > r.room {
        format!("used {} of {} KB, {} KB over\n", kb(r.used), kb(r.room), kb(r.used - r.room))
    } else {
        format!("used {} of {} KB, {} KB free\n", kb(r.used), kb(r.room), kb(r.free))
    };
    if r.rows.iter().any(|w| w.packed.is_none()) {
        s += &format!("left out:\n{}", left_out_list(&r.rows));
    }
    for l in r.sound.iter().chain(&r.arranged) {
        s += &format!("{l}\n");
    }
    s
}

fn kb(bytes: u32) -> String {
    format!("{:.1}", f64::from(bytes) / 1024.0)
}

/// The file name of a row's file.
fn file_name(w: &Row) -> String {
    Path::new(&w.file).file_name().map_or(w.file.clone(), |f| f.to_string_lossy().into_owned())
}

/// The width of the "how" column: its longest entry, at least 20.
fn how_width(rows: &[Row]) -> usize {
    rows.iter().filter(|w| w.packed.is_some()).map(|w| w.how.len()).max().unwrap_or(0).max(20)
}

fn game_line(w: &Row, how: usize) -> String {
    format!("  {:<28} {:<how$} {:>6} KB  {}\n", w.name, w.how, kb(w.packed.unwrap_or(0)), file_name(w))
}

fn left_out_list(rows: &[Row]) -> String {
    rows.iter()
        .filter(|w| w.packed.is_none())
        .map(|w| format!("  {}: {}\n", file_name(w), w.reason.as_deref().unwrap_or("not in the pack")))
        .collect()
}

fn too_big(rows: &[Row], used: u32, limit: u32, games_file: &Path, from_folder: bool) -> String {
    let mut by_size: Vec<&Row> = rows.iter().filter(|w| w.packed.is_some()).collect();
    by_size.sort_by_key(|w| std::cmp::Reverse(w.packed));
    let mut s = format!(
        "the pack does not fit: it needs {} KB, and the app slot leaves {} KB for it ({} KB over).\n\
         Games by packed size, largest first:\n",
        kb(used),
        kb(limit),
        kb(used - limit)
    );
    let how = how_width(rows);
    for w in by_size {
        s += &game_line(w, how);
    }
    if rows.iter().any(|w| w.packed.is_none()) {
        s += &format!("Left out:\n{}", left_out_list(rows));
    }
    let f = games_file.display();
    s += &if from_folder {
        format!("Leave some out: in {f}, comment out their lines with #, then run again.")
    } else {
        format!("Leave some out: in {f}, comment out their lines with # (or delete them), then run again.")
    };
    s
}

/// `prepare`'s refusal of a 128K file is just "128K" in the table.
fn reason(e: &str) -> String {
    if e == tapeload::LEFT_OUT_128K {
        "128K".into()
    } else {
        e.to_string()
    }
}

/// The converted file's extension: a tape is a `.tap`, a snapshot taken as it is keeps its own,
/// a loaded one is a `.z80`.
pub fn converted_ext(l: &Loaded, source: &Path) -> String {
    match l.kind {
        Kind::Tape => "tap".into(),
        Kind::Snapshot if l.frames == 0 => {
            source.extension().and_then(|e| e.to_str()).map_or("z80".into(), str::to_ascii_lowercase)
        }
        Kind::Snapshot => "z80".into(),
    }
}

/// The folder's game files, in file name order.
fn game_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    for e in rd {
        let e = e.map_err(|e| format!("{}: {e}", dir.display()))?;
        let p = e.path();
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !hidden && p.is_file() && ["sna", "z80", "tap", "tzx"].contains(&ext.as_str()) {
            files.push(p);
        }
    }
    files.sort();
    Ok(files)
}

/// A TOML basic string.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// The game name for a file name without its extension (see the module docs).
pub fn name_from_file(stem: &str) -> String {
    let mut s = String::new();
    let mut depth = 0u32;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => s.push(if c.is_whitespace() { ' ' } else { c }),
            _ => {}
        }
    }
    let s = s.trim();
    let s = match s.find(" - ") {
        Some(i) if i > 0 => &s[..i],
        _ => s,
    };
    let mut name = String::new();
    for c in s.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()) {
        if !(c == ' ' && (name.is_empty() || name.ends_with(' '))) {
            name.push(c);
        }
    }
    name.truncate(NAME_MAX);
    let name = name.trim_end();
    if name.is_empty() {
        "Game".into()
    } else {
        name.into()
    }
}

/// An id from a name, not one of `taken` (see the module docs).
fn unique_id(name: &str, taken: &HashSet<String>) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "game" } else { slug };
    if !taken.contains(slug) {
        return slug.into();
    }
    (2..).map(|n| format!("{slug}-{n}")).find(|id| !taken.contains(id)).expect("an unused id")
}

/// The id of a commented-out `id = "..."` line.
fn commented_id(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('#')?.trim_start_matches(['#', ' ', '\t']);
    if !rest.starts_with("id") {
        return None;
    }
    let t: toml::Table = toml::from_str(rest).ok()?;
    t.get("id")?.as_str().map(str::to_string)
}

fn is_game_header(line: &str) -> bool {
    let t = line.trim();
    t.strip_prefix(GAME_HEADER).is_some_and(|rest| rest.trim_start().is_empty() || rest.trim_start().starts_with('#'))
}

/// The entries of a games file's text (none when it has no `[[game]]`).
fn entries(text: &str) -> Result<Vec<Game>, String> {
    if !text.lines().any(is_game_header) {
        return Ok(Vec::new());
    }
    Ok(parse_personal_manifest(text)?.game)
}

/// Updates (or starts) the games file at `path` for the folder's `files` (see the module docs);
/// returns its text. Files tried and not loadable are pushed to `left_out`.
fn update_games_file(
    path: &Path,
    files: &[PathBuf],
    prepared: &mut HashMap<PathBuf, Result<Loaded, String>>,
    left_out: &mut Vec<Row>,
) -> Result<String, String> {
    let shown = path.display();
    let old = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{shown}: {e}")),
    };
    let mut text = old.clone().unwrap_or_else(|| HEADER.to_string());
    let fix = |e: String| format!("{shown}: {e} (fix it, or delete the file to start again)");
    let games = entries(&text).map_err(fix)?;
    let dir = path.parent().unwrap_or(Path::new("/"));
    // like with like: the folder's files are its canonical path joined with their names, so an
    // entry's path is compared with its folder made canonical and its file name as it is (a
    // symlinked game stays the link, never its target)
    let as_listed = |p: PathBuf| match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
        (Some(d), Some(n)) => d.join(n),
        _ => p,
    };
    let known: HashSet<PathBuf> = games.iter().map(|g| as_listed(game_path(dir, g))).collect();
    // ids in use, those of commented-out entries too (so uncommenting one never clashes)
    let mut ids: HashSet<String> = games.iter().map(|g| g.id.clone()).collect();
    ids.extend(text.lines().filter_map(commented_id));
    // a file is known when an entry has it, or when its quoted path is anywhere in the text,
    // comment lines included: the owner leaves a game out by commenting out its entry
    let listed = |f: &PathBuf| known.contains(f) || f.to_str().is_some_and(|s| text.contains(&quoted(s)));
    let new: Vec<&PathBuf> = files.iter().filter(|f| !listed(f)).collect();
    for f in new {
        let file = f.display().to_string();
        let name = name_from_file(&f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let got = prepared.entry(f.clone()).or_insert_with(|| tapeload::prepare(f));
        if let Err(e) = got {
            left_out.push(Row { name, file, how: "left out".into(), packed: None, reason: Some(reason(e)) });
            continue;
        }
        if f.to_str().is_none() {
            let why = "the file's path is not UTF-8, which the games file cannot hold".to_string();
            left_out.push(Row { name, file, how: "left out".into(), packed: None, reason: Some(why) });
            continue;
        }
        let id = unique_id(&name, &ids);
        ids.insert(id.clone());
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() && !text.ends_with("\n\n") {
            text.push('\n');
        }
        text += &format!("{GAME_HEADER}\nid = {}\nname = {}\nfile = {}\n", quoted(&id), quoted(&name), quoted(&file));
    }
    let games = entries(&text).map_err(fix)?;
    let text = mark_missing(&text, &games.iter().map(|g| (g.file.clone(), !game_path(dir, g).is_file())).collect::<Vec<_>>())
        .map_err(fix)?;
    if old.as_deref() != Some(text.as_str()) {
        std::fs::write(path, &text).map_err(|e| format!("{shown}: {e}"))?;
    }
    Ok(text)
}

/// The text with a `# missing: <file>` line just above each `[[game]]` whose file is gone, and
/// none above the others; every other line as it was. `games`: (file, missing) per entry.
fn mark_missing(text: &str, games: &[(String, bool)]) -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();
    let mut k = 0;
    for line in text.split_inclusive('\n') {
        if is_game_header(line) {
            while lines.last().is_some_and(|l| l.starts_with(MISSING)) {
                lines.pop();
            }
            let Some((file, missing)) = games.get(k) else { break };
            if *missing {
                lines.push(format!("{MISSING} {file}\n"));
            }
            k += 1;
        }
        lines.push(line.to_string());
    }
    if k != games.len() || text.split_inclusive('\n').filter(|l| is_game_header(l)).count() != games.len() {
        return Err(format!("its {GAME_HEADER} lines do not match its {} games: write each as a line of its own", games.len()));
    }
    Ok(lines.concat())
}

/// The converted files' manifest: the games' own names, crops, demos and keys.
fn manifest_text(games: &[(Game, String, usize)]) -> String {
    let mut s = format!("{MANIFEST_HEAD}\n");
    for (g, file, _) in games {
        s += &format!("\n{GAME_HEADER}\nid = {}\nname = {}\nfile = {}\ncrop = {}\n", quoted(&g.id), quoted(&g.name), quoted(file), g.crop);
        if let Some(d) = &g.demo {
            s += &format!("demo = {}\n", quoted(d));
        }
        if !g.keys.is_empty() {
            s += "[game.keys]\n";
            for (k, v) in &g.keys {
                s += &format!("{} = {}\n", quoted(k), quoted(v));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(name_from_file("Cave Runner (1983)(Some House)[a]"), "Cave Runner");
        assert_eq!(name_from_file("Night Raid II - The Return - 48k"), "Night Raid II");
        assert_eq!(name_from_file("MoonCaves(48K)"), "MoonCaves");
        assert_eq!(name_from_file("Foo (x)  Bar"), "Foo Bar");
        assert_eq!(name_from_file("(1984)"), "Game");
        assert_eq!(name_from_file("Ünïcode Nämé"), "ncode Nm");
        assert_eq!(name_from_file(&"A very long name that goes on".repeat(2)).len(), NAME_MAX);
    }

    #[test]
    fn ids() {
        let taken: HashSet<String> = ["cave-runner".into(), "cave-runner-2".into()].into();
        assert_eq!(unique_id("Cave Runner", &taken), "cave-runner-3");
        assert_eq!(unique_id("Night Raid II", &taken), "night-raid-ii");
        assert_eq!(unique_id("!!!", &taken), "game");
        assert_eq!(unique_id("A..B--", &taken), "a-b");
    }

    #[test]
    fn markers() {
        let text = "# top\n[[game]]\nid = \"a\"\n# missing: old\n[[game]] # b\nid = \"b\"\n";
        let got = mark_missing(text, &[("a.sna".into(), true), ("b.sna".into(), false)]).unwrap();
        assert_eq!(got, "# top\n# missing: a.sna\n[[game]]\nid = \"a\"\n[[game]] # b\nid = \"b\"\n");
        assert!(mark_missing(text, &[("a.sna".into(), false)]).is_err());
    }

    #[test]
    fn cache_files() {
        let empty = cache_encode(None, &[], None, "no music learned (x)", "  d\n");
        assert_eq!(cache_decode(&empty), Some((None, None, "no music learned (x)".into(), "  d\n".into(), vec![])));
        // any byte changed fails the CRC
        for i in 8..empty.len() {
            let mut bad = empty.clone();
            bad[i] ^= 1;
            assert_eq!(cache_decode(&bad), None, "byte {i}");
        }
        // a list method B's decoder refuses (one entry announced, none there), its CRC right
        assert_eq!(cache_decode(&cache_encode(Some(&[1, 0, 0, 0]), &[], None, "music learned (x)", "")), None);
    }

    fn a_score(site: u16) -> ScoreRecord {
        ScoreRecord {
            site,
            quiet: 25,
            code: 1,
            voices: 1,
            precedence: false,
            replacement: None,
            roles: [0; 3],
            intro_t: 10 * zx_sys::ZX_FRAME_TSTATES,
            loop_t: 0,
            intro_fp: vec![1; 10],
            loop_fp: vec![],
            intro_events: vec![soundlearn::ScoreEvent { t: 0, voice: 0, on: Some((2000, 100)) }],
            loop_events: vec![],
        }
    }

    #[test]
    fn cache_files_keep_the_scores_runs() {
        let list = encode_list_scores(&[], &[], &[a_score(0x8000), a_score(0x9000)]);
        let steps = vec![Step { action: 3, at: 1000, hold: 100 }, Step { action: 0x41, at: u64::MAX, hold: 7 }];
        for learner in [LearnerRun::Script(steps), LearnerRun::Script(vec![]), LearnerRun::Kind(RunKind::Generic, 0x1234_5678), LearnerRun::Kind(RunKind::Once(4), 0)] {
            let runs = ScoreRuns { learner, scores: vec![None, Some(RunKind::Key(11))], shadows: vec![Some(RunKind::Key(2))] };
            assert_eq!(ScoreRuns::decode(&runs.encode()).as_ref(), Some(&runs));
            let shadows = [ScoreRecord { precedence: true, ..a_score(0xA000) }];
            let b = cache_encode(Some(&list), &shadows, Some(&runs), "music recorded (2 scores, 1 voice)", "  d\n");
            let want = (Some(list.clone()), Some(runs.clone()), "music recorded (2 scores, 1 voice)".to_string(), "  d\n".to_string(), shadows.to_vec());
            assert_eq!(cache_decode(&b), Some(want));
            // a run for each score and each shadow score, or the cache is not used
            let short = ScoreRuns { scores: vec![None], ..runs.clone() };
            assert_eq!(cache_decode(&cache_encode(Some(&list), &shadows, Some(&short), "x", "")), None);
            assert_eq!(cache_decode(&cache_encode(Some(&list), &[], Some(&runs), "x", "")), None);
            assert_eq!(cache_decode(&cache_encode(Some(&list), &shadows, None, "x", "")), None);
            // a shadow score is one with precedence
            assert_eq!(cache_decode(&cache_encode(Some(&list), &[a_score(0xA000)], Some(&runs), "x", "")), None);
            // shadow scores alone
            let alone = ScoreRuns { scores: vec![], ..runs.clone() };
            let b = cache_encode(None, &shadows, Some(&alone), "x", "");
            assert_eq!(cache_decode(&b), Some((None, Some(alone.clone()), "x".into(), "".into(), shadows.to_vec())));
            // runs cut short or run on are not runs
            let e = runs.encode();
            assert_eq!(ScoreRuns::decode(&e[..e.len() - 1]), None);
            assert_eq!(ScoreRuns::decode(&[e.clone(), vec![0]].concat()), None);
        }
        // every run kind has a code, and no code names a run there is none of
        for k in candidates().into_iter().chain((0..KEYS.len()).map(RunKind::Key)) {
            assert_eq!(kind_of(kind_code(k)), Some(k));
        }
        assert_eq!((0..=255).filter_map(kind_of).count(), candidates().len() + KEYS.len());
        // the inputs: the learner's for its scores, the key run's for its
        let runs = ScoreRuns { learner: LearnerRun::Script(vec![Step { action: 0, at: 0, hold: 40 }]), scores: vec![None, Some(RunKind::Key(0))], shadows: vec![Some(RunKind::Key(0))] };
        let (a, b) = (runs.inputs(Of::Score(0)).unwrap(), runs.inputs(Of::Score(1)).unwrap());
        assert_eq!((a(0).0[0], a(2).0[0], b(0).0, b(500).0[3]), (1, 0, [0; 8], 1));
        assert!(runs.inputs(Of::Score(2)).is_none());
        assert_eq!(runs.inputs(Of::Shadow(0)).unwrap()(500).0[3], 1);
        assert!(runs.inputs(Of::Shadow(1)).is_none());
    }

    #[test]
    fn a_panic_is_one_games_learner_error() {
        let got = isolated(|| panic!("broken game"));
        assert_eq!((got.list, got.line.as_str()), (None, "no music learned (learner error: broken game)"));
        assert_eq!(got.details, "  learner error: broken game\n");
        let fine = learner_error("x");
        assert_eq!(isolated(|| fine.clone()), fine);
    }

    #[test]
    fn statuses_under_their_scores() {
        let (a, b, c) = (a_score(0x8057), a_score(0x8058), a_score(0x9000));
        let (ka, kb, kc) = (score_key(&a), score_key(&b), score_key(&c));
        let details = format!(
            "  OUT at 8057h, heard in 9 frames (9 edges)\n    recorded score: 1.0 s from frame 3, 1 voice, 1 note, 0.1 KB (score {ka})\n\
             \x20   recorded score (key 3 once): 1.0 s from frame 9, 1 voice, 1 note, 0.1 KB (score {kb})\n  OUT at 9000h, code only\n"
        );
        let status = [Some(Status::Refused { take: arrange::Choice::Take(2), why: "w".into() }), None, Some(Status::Original)];
        let got = with_status(&details, &[a.clone(), b.clone(), c.clone()], &status, &[], &[]);
        assert_eq!(
            got,
            format!(
                "  OUT at 8057h, heard in 9 frames (9 edges)\n    recorded score: 1.0 s from frame 3, 1 voice, 1 note, 0.1 KB (score {ka})\n\
                 \x20     arranged: take 2 refused: w\n    recorded score (key 3 once): 1.0 s from frame 9, 1 voice, 1 note, 0.1 KB (score {kb})\n\
                 \x20 OUT at 9000h, code only\n  score {kc}: arranged: original\n"
            )
        );
    }

    #[test]
    fn statuses_under_their_shadow_scores() {
        let (a, b) = (ScoreRecord { precedence: true, ..a_score(0x8057) }, ScoreRecord { precedence: true, ..a_score(0x9000) });
        let (ka, kb) = (score_key(&a), score_key(&b));
        let details = format!("  OUT at 8057h, heard in 9 frames (9 edges)\n    shadow score: 1.0 s from frame 3, 1 voice, 1 note, 0.1 KB (score {ka}{SHADOW_TAIL})\n");
        let status = [Some(Status::Take { take: arrange::Choice::Take(1), generator: "g".into(), dropped: (0, 0) }), Some(Status::Original)];
        let got = with_status(&details, &[], &[], &[a, b], &status);
        assert_eq!(
            got,
            format!("{details}      arranged: take 1 (g)\n  shadow score {kb}: arranged: the live version (no take chosen)\n"),
        );
    }

    #[test]
    fn commented_ids() {
        assert_eq!(commented_id("# id = \"tune\""), Some("tune".into()));
        assert_eq!(commented_id("  ## id = \"t-2\"  # note"), Some("t-2".into()));
        assert_eq!(commented_id("id = \"live\""), None);
        assert_eq!(commented_id("# ids are made from names"), None);
        assert_eq!(commented_id("# identity"), None);
    }

    #[test]
    fn json_strings() {
        assert_eq!(json_string("a\"b\\c\nd\te\u{1}f é"), "\"a\\\"b\\\\c\\nd\\te\\u0001f é\"");
        assert_eq!(plan_status(None), "original");
        assert_eq!(plan_status(Some(&Status::Original)), "original");
        assert_eq!(plan_status(Some(&Status::Take { take: Choice::Chip(1), generator: "C64: x".into(), dropped: (1, 0) })), "chip-1 (C64: x)");
        assert_eq!(plan_status(Some(&Status::Take { take: Choice::Ay(2), generator: "Spectrum 128: x".into(), dropped: (0, 0) })), "ay-2 (Spectrum 128: x)");
        assert_eq!(plan_status(Some(&Status::Refused { take: Choice::Take(2), why: "no".into() })), "refused: no");
    }
}
