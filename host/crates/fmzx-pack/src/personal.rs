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
//! (`games.bin`, `keymaps.json`, the two packages, `sound-report.txt` and `converted/`), so a
//! failed run leaves no pack or packages that look valid; an output folder that holds any of them
//! without being a personal pack's (no `personal-games.toml`, no `converted/manifest.toml` of
//! ours) is refused.
//!
//! # The music
//!
//! Each packed game's music is learned ([`learn`](soundlearn::learn::learn)) from its converted
//! file, started as the device starts it, under the key script `<file>.keys` beside the game file
//! if there is one (the `music-corpus` format, its times from time 0; one that does not parse is
//! reported and the generic run is used), else the generic run seeded with the converted file's
//! CRC-32. The games are learned in parallel, one thread per core. The pack's sound section holds each
//! game's learned entries ([`attach_sound`]); [`table`] ends with one line per game
//! (`<name>: music learned (...)` or `<name>: no music learned (<reason>)`), and
//! `<out>/sound-report.txt` holds them with each routine's details. The results are cached in
//! `<out>/sound-cache/<crc>-<version>.bin` (the CRC-32 of the converted file and of the key
//! script, the learner's version), which runs keep: an unchanged game is not learned again. A
//! cache file is used only if its CRC matches and method B's decoder takes its list; otherwise
//! the game is learned again. A panic while learning a game gives that game a learner error,
//! and the pack is still built.

use crate::keymaps::keymaps_json;
use crate::keys::{inputs, parse_keys};
use crate::layout::pack_room;
use crate::manifest::{game_path, parse_manifest, parse_personal_manifest, Game, NAME_MAX};
use crate::pack::{attach_sound, build_pack};
use crate::tapeload::{self, Kind, Loaded};
use soundlearn::listen::{schedule, Inputs};
use soundlearn::{encode_list, learn, LEARNER_VERSION};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use zx_sys::{pack_game_at, Machine, SndEngines, PACK_MAX_RAW};

/// The editable games file, in the output folder.
pub const GAMES_FILE: &str = "personal-games.toml";
/// The outputs of a run, removed when the next one starts; `converted/` too (not `sound-cache/`).
const OUTPUTS: [&str; 5] = ["games.bin", "keymaps.json", "fmzx.fwsc", "fmzx-preview.fwsc", SOUND_REPORT];
const CONVERTED: &str = "converted";
/// The learned music's details, in the output folder.
pub const SOUND_REPORT: &str = "sound-report.txt";
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
/// the pack's size and the room left beside it, in bytes; the learned music's line for each
/// packed game, in pack order (`<name>: music learned (...)`).
#[derive(Clone, Debug)]
pub struct Report {
    pub rows: Vec<Row>,
    pub used: u32,
    pub free: u32,
    pub sound: Vec<String>,
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
                jobs.push(Job { name: g.name.clone(), data: l.data.clone(), file: file.clone(), keys: key.with_extension("keys") });
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

    // each game's music, then the pack from the converted files' own manifest, with the sound
    // section
    let sounds = learn_all(&jobs, &out.join(SOUND_CACHE));
    let manifest_path = conv.join("manifest.toml");
    let text = manifest_text(&packed);
    std::fs::write(&manifest_path, &text).map_err(|e| format!("{}: {e}", shown(&manifest_path)))?;
    let mut pack = build_pack(&manifest_path)?;
    attach_sound(&mut pack, &sounds.iter().map(|s| s.list.clone()).collect::<Vec<_>>())?;
    for (i, (_, _, r)) in packed.iter().enumerate() {
        let g = pack_game_at(&pack, i as u32).map_err(|e| format!("the pack just built does not read back (error {e})"))?;
        rows[*r].packed = Some(g.stored_len());
    }
    let used = u32::try_from(pack.len()).unwrap_or(u32::MAX);
    if used > limit {
        return Err(too_big(&rows, used, limit, &games_file, input.is_dir()));
    }
    let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", shown(&manifest_path)))?;
    let keymaps = keymaps_json(&manifest)?;
    let sound: Vec<String> = jobs.iter().zip(&sounds).map(|(j, s)| format!("{}: {}", j.name, s.line)).collect();
    let mut details = String::from(REPORT_HEAD);
    for ((j, s), l) in jobs.iter().zip(&sounds).zip(&sound) {
        details += &format!("\n{l}\n{}{}", keys_note(j, s), s.details);
    }
    for (name, bytes) in [("keymaps.json", keymaps.as_bytes()), (SOUND_REPORT, details.as_bytes()), ("games.bin", &pack[..])] {
        let p = out.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", shown(&p)))?;
    }
    Ok(Report { rows, used, free: limit - used, sound })
}

const REPORT_HEAD: &str = "\
# The music tools/make-personal-pack learned for each game (BUILDING.md, \"Your own games\"), and
# per routine: the speaker OUT it was read from, the routine, each channel's half period in
# T-states (an input in a product counts a loop: 0 counts 256, 65,536 for a word), the gates'
# figures, and whether it was learned or why not.
";

/// A packed game, for learning its music: its name, its converted file's bytes and name, and
/// where its key script would be.
struct Job {
    name: String,
    data: Vec<u8>,
    file: String,
    keys: PathBuf,
}

/// What was learned of a game: its entry list (None: none), its line without the name, its
/// details, and the key script's problem if it had one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
    pub list: Option<Vec<u8>>,
    pub line: String,
    pub details: String,
    pub keys_error: Option<String>,
}

/// The key script's problem as a details line, if any.
fn keys_note(j: &Job, s: &Sound) -> String {
    s.keys_error.as_ref().map_or(String::new(), |e| format!("  {}: {e}; the generic run was used\n", j.keys.display()))
}

/// The machine of a converted file, started as the device starts it.
fn machine(file: &str, data: &[u8]) -> Result<Machine, String> {
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
    Sound { list: None, line: format!("no music learned (learner error: {why})"), details: format!("  learner error: {why}\n"), keys_error: None }
}

/// Milliseconds per Spectrum frame, for the key scripts' times.
const FRAME_MS: u64 = 20;
/// The start of a cache file.
const CACHE_MAGIC: &[u8; 8] = b"FMZXSND2";

/// A game's music: from the cache, or learned and then cached.
fn learn_cached(job: &Job, cache: &Path) -> Sound {
    learn_music(&job.file, &job.data, &job.keys, cache)
}

/// The music of a game as the personal pack learns it: `file` and `data` its converted file
/// ([`tapeload::prepare`]'s data, named with [`converted_ext`]), `keys` where its key script
/// would be, `cache` the cache folder (it must exist). From the cache, or learned under the key
/// script's steps (or the generic run, seeded by the converted data's CRC-32) and then cached.
/// `music-corpus` learns through this too, so both learn the same.
pub fn learn_music(file: &str, data: &[u8], keys: &Path, cache: &Path) -> Sound {
    let script = std::fs::read_to_string(keys).ok();
    let (steps, keys_error) = match script.as_deref().map(parse_keys) {
        Some(Ok(steps)) => (Some(steps), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let mut crc = crc32fast::Hasher::new();
    crc.update(data);
    if let Some(t) = &script {
        crc.update(t.as_bytes());
    }
    let path = cache.join(format!("{:08x}-{LEARNER_VERSION}.bin", crc.finalize()));
    if let Some((list, line, details)) = std::fs::read(&path).ok().as_deref().and_then(cache_decode) {
        return Sound { list, line, details, keys_error };
    }
    let (list, line, details) = learn_converted(file, data, steps);
    // written whole, then renamed: a half-written file is never read
    let tmp = cache.join(format!("{file}.tmp"));
    if std::fs::write(&tmp, cache_encode(list.as_deref(), &line, &details)).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Sound { list, line, details, keys_error }
}

/// Learns a game's music under its key script's steps, or the generic run: its entry list (None:
/// none), line and details.
fn learn_converted(file: &str, data: &[u8], steps: Option<Vec<crate::keys::Step>>) -> (Option<Vec<u8>>, String, String) {
    if let Err(e) = machine(file, data) {
        return (None, format!("no music learned (the game does not load: {e})"), String::new());
    }
    let run: Inputs = match steps {
        Some(steps) => Box::new(move |f| inputs(&steps, u64::from(f) * FRAME_MS)),
        None => schedule(u64::from(crc32fast::hash(data))),
    };
    // it loaded just now, so it loads again: the fallback is never used
    let got = learn(&|| machine(file, data).unwrap_or_else(|_| Machine::new()), &run);
    let list = (!got.entries.is_empty()).then(|| encode_list(&got.entries));
    (list, got.line, got.details)
}

/// A cache file: the magic, the CRC-32 of the rest (u32, little-endian), then the entry list,
/// the line and the details, each a u32 length and its bytes; an empty list is none.
pub fn cache_encode(list: Option<&[u8]>, line: &str, details: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    for part in [list.unwrap_or(&[]), line.as_bytes(), details.as_bytes()] {
        payload.extend((part.len() as u32).to_le_bytes());
        payload.extend(part);
    }
    let mut b = CACHE_MAGIC.to_vec();
    b.extend(crc32fast::hash(&payload).to_le_bytes());
    b.extend(payload);
    b
}

/// A cache file's contents; None if it is not one, its CRC does not match, or method B's
/// decoder refuses its list (the cache is not trusted: a list goes into the pack only as the
/// decoder takes it).
pub fn cache_decode(b: &[u8]) -> Option<(Option<Vec<u8>>, String, String)> {
    let rest = b.strip_prefix(CACHE_MAGIC)?;
    let (crc, mut rest) = (u32::from_le_bytes(rest.get(..4)?.try_into().ok()?), rest.get(4..)?);
    if crc32fast::hash(rest) != crc {
        return None;
    }
    let mut part = || -> Option<Vec<u8>> {
        let len = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let bytes = rest.get(4..4 + len)?.to_vec();
        rest = &rest[4 + len..];
        Some(bytes)
    };
    let (list, line, details) = (part()?, part()?, part()?);
    if !rest.is_empty() || (!list.is_empty() && SndEngines::with_learned(&list).is_err()) {
        return None;
    }
    Some(((!list.is_empty()).then_some(list), String::from_utf8(line).ok()?, String::from_utf8(details).ok()?))
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
    s += &format!("used {} of {} KB, {} KB free\n", kb(r.used), kb(r.used + r.free), kb(r.free));
    if r.rows.iter().any(|w| w.packed.is_none()) {
        s += &format!("left out:\n{}", left_out_list(&r.rows));
    }
    for l in &r.sound {
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
        let empty = cache_encode(None, "no music learned (x)", "  d\n");
        assert_eq!(cache_decode(&empty), Some((None, "no music learned (x)".into(), "  d\n".into())));
        // any byte changed fails the CRC
        for i in 8..empty.len() {
            let mut bad = empty.clone();
            bad[i] ^= 1;
            assert_eq!(cache_decode(&bad), None, "byte {i}");
        }
        // a list method B's decoder refuses (one entry announced, none there), its CRC right
        assert_eq!(cache_decode(&cache_encode(Some(&[1, 0, 0, 0]), "music learned (x)", "")), None);
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
    fn commented_ids() {
        assert_eq!(commented_id("# id = \"tune\""), Some("tune".into()));
        assert_eq!(commented_id("  ## id = \"t-2\"  # note"), Some("t-2".into()));
        assert_eq!(commented_id("id = \"live\""), None);
        assert_eq!(commented_id("# ids are made from names"), None);
        assert_eq!(commented_id("# identity"), None);
    }
}
