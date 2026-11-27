// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The built-in games' learned music: the sound section of the built-in pack, kept as a file
//! beside `games/games.toml` so that building the pack needs no learning.
//!
//! The games file names it with a top-level `sound = "sound.toml"`, and [`crate::pack::build_pack`]
//! attaches its lists, each to the game of its `id` ([`lists`]). The file is written by
//! `fmzx-pack games-sound <games.toml> <work> -o <sound.toml>` ([`write`]), which builds the
//! personal pack of the games file in `<work>` ([`crate::personal`]: the music learned, its scores
//! recorded, the takes `<work>/arrange/choices.toml` chooses packed) and keeps each game's entry
//! list as that pack holds it. `tools/games-sound` runs the whole of it (BUILDING.md, "The built-in
//! games' music"): the learning, an engine take for each score whose game plays a known engine
//! (`tools/arrange-music engine`), that take chosen, the file written; `--check` writes it under
//! `build/` and compares.
//!
//! The file:
//!
//! ```toml
//! [[game]]
//! id = "some-game"         # the game's id in the games file
//! crc = "1a2b3c4d"         # CRC-32 of the game file the list was learned from
//! list = """
//! 0200000000000000...      # the entry list (docs/pack-format.md, "Entry list"), in hex
//! """
//! ```
//!
//! A game with no entry list has no `[[game]]`. The test programs (the games file's games under
//! `test/`, which `fmzx-pack testprogs` writes) get none: the tests pin how the firmware plays
//! them with its built-in entries alone. [`lists`] refuses a file that names a game the games
//! file does not have, a game whose file has another CRC-32 than the one its list was learned
//! from (run `tools/games-sound` again), or a list the firmware's decoder rejects.

use crate::manifest::{game_path, parse_manifest, parse_personal_manifest, Manifest};
use crate::pack::sound_list;
use crate::personal;
use serde::Deserialize;
use std::fmt::Write as _;
use std::path::Path;

/// The folder of the test programs in the games file, whose games get no list.
const TEST_DIR: &str = "test/";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    game: Vec<RawGame>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGame {
    id: String,
    crc: String,
    list: String,
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    digits.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok()).collect()
}

/// What an entry list holds, entry by entry (its decoder checked it).
fn describe(list: &[u8]) -> String {
    const ROLE: [&str; 4] = ["live", "bass", "lead", "pad"];
    let mut parts = Vec::new();
    let mut at = 4;
    for _ in 0..list.first().copied().unwrap_or(0) {
        let Some(e) = list.get(at..at + 16) else { break };
        let size = u16::from_le_bytes([e[0], e[1]]) as usize;
        let n = (e[3] & 0x3F) as usize;
        parts.push(match e[2] {
            0 => format!("a learned routine of {n} channels"),
            1 => format!(
                "a built-in entry's channel roles ({})",
                e[12..12 + n.min(3)].iter().map(|&r| ROLE.get(r as usize).copied().unwrap_or("live")).collect::<Vec<_>>().join(", ")
            ),
            _ => format!("a recorded score of {n} voices{}", if e[3] & 0x40 != 0 { ", a replacement" } else if e[3] & 0x80 != 0 { ", with precedence" } else { "" }),
        });
        at += size.max(4);
    }
    parts.join("; ")
}

/// The entry list of each game of `manifest` (in its order) from its `sound` file, or all `None`
/// when it names none; `manifest_path` is the games file's path. See the module docs for what it
/// refuses.
pub fn lists(manifest_path: &Path, manifest: &Manifest) -> Result<Vec<Option<Vec<u8>>>, String> {
    let mut out = vec![None; manifest.game.len()];
    let Some(file) = &manifest.sound else { return Ok(out) };
    let dir = manifest_path.parent().unwrap_or(Path::new("."));
    let path = dir.join(file);
    let at = |e: String| format!("{}: {e}", path.display());
    let text = std::fs::read_to_string(&path).map_err(|e| at(e.to_string()))?;
    let raw: RawFile = toml::from_str(&text).map_err(|e| at(e.to_string()))?;
    for g in raw.game {
        let i = manifest.game.iter().position(|m| m.id == g.id).ok_or_else(|| at(format!("game {:?} is not in the games file", g.id)))?;
        if out[i].is_some() {
            return Err(at(format!("game {:?} is given twice", g.id)));
        }
        let crc = u32::from_str_radix(&g.crc, 16).map_err(|_| at(format!("game {}: crc {:?} is not 8 hex digits", g.id, g.crc)))?;
        let game_file = game_path(dir, &manifest.game[i]);
        let data = std::fs::read(&game_file).map_err(|e| format!("{}: {e}", game_file.display()))?;
        let now = crc32fast::hash(&data);
        if now != crc {
            return Err(at(format!(
                "game {}: its music was learned from another file (CRC-32 {crc:08x}, {} is {now:08x}): run tools/games-sound",
                g.id,
                manifest.game[i].file
            )));
        }
        let list = unhex(&g.list).ok_or_else(|| at(format!("game {}: list is not hex bytes", g.id)))?;
        if list.len() % 4 != 0 {
            return Err(at(format!("game {}: list is {} bytes, not a multiple of 4", g.id, list.len())));
        }
        zx_sys::SndEngines::with_learned(&list).map_err(|e| at(format!("game {}: the firmware's decoder rejects its list (error {e})", g.id)))?;
        out[i] = Some(list);
    }
    Ok(out)
}

/// Builds the personal pack of the games file `manifest_path` in `work` (the music learned and
/// arranged as `fmzx-pack personal` does it, with no room limit but the empty image's) and, with
/// `out`, writes the sound file from it. Returns the personal pack's table (its games and music
/// lines).
pub fn write(manifest_path: &Path, work: &Path, out: Option<&Path>) -> Result<String, String> {
    let text = std::fs::read_to_string(manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    std::fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let report = personal::run(manifest_path, work)?;
    let table = personal::table(&report);
    let Some(out) = out else { return Ok(table) };

    let left: Vec<&str> = report.rows.iter().filter(|r| r.packed.is_none()).map(|r| r.name.as_str()).collect();
    if !left.is_empty() {
        return Err(format!("games left out of the pack: {}", left.join(", ")));
    }
    let conv = work.join("converted");
    let conv_text = std::fs::read_to_string(conv.join("manifest.toml")).map_err(|e| format!("{}: {e}", conv.display()))?;
    let converted = parse_personal_manifest(&conv_text)?;
    let pack_path = work.join("games.bin");
    let pack = std::fs::read(&pack_path).map_err(|e| format!("{}: {e}", pack_path.display()))?;
    let dir = manifest_path.parent().unwrap_or(Path::new("."));

    let mut s = String::from(
        "# The built-in games' learned music, the sound section of the pack `fmzx-pack games games.toml`\n\
         # builds (games.toml names this file). Written by tools/games-sound (BUILDING.md, \"The built-in\n\
         # games' music\"); do not edit. Each game's lines say what its list holds; the lists,\n\
         # docs/pack-format.md's entry lists in hex, are data made from the games, under each game's\n\
         # licence (LICENSING.md, \"The built-in games\").\n",
    );
    for g in manifest.game.iter().filter(|g| !g.file.starts_with(TEST_DIR)) {
        let Some(i) = converted.game.iter().position(|c| c.id == g.id) else {
            return Err(format!("game {} is not in {}", g.id, conv.join("manifest.toml").display()));
        };
        let original = std::fs::read(game_path(dir, g)).map_err(|e| format!("{}: {e}", g.file))?;
        let packed = std::fs::read(game_path(&conv, &converted.game[i])).map_err(|e| format!("{}: {e}", converted.game[i].file))?;
        if packed != original {
            return Err(format!(
                "game {}: the personal pack holds it converted ({}), so its music was learned from another file than the built-in pack holds",
                g.id, converted.game[i].file
            ));
        }
        let Some(list) = sound_list(&pack, i as u32) else { continue };
        writeln!(s, "\n[[game]]\nid = \"{}\"\ncrc = \"{:08x}\"", g.id, crc32fast::hash(&original)).unwrap();
        let prefix = format!("{}: ", g.name);
        for line in report.sound.iter().chain(&report.arranged).filter(|l| l.starts_with(&prefix)) {
            writeln!(s, "# {line}").unwrap();
        }
        writeln!(s, "# list: {}", describe(list)).unwrap();
        s += "list = \"\"\"\n";
        let hex: String = list.iter().map(|b| format!("{b:02x}")).collect();
        for chunk in hex.as_bytes().chunks(96) {
            s += std::str::from_utf8(chunk).unwrap();
            s += "\n";
        }
        s += "\"\"\"\n";
    }
    std::fs::write(out, &s).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(table)
}
