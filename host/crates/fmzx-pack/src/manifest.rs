// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game manifest (TOML) and its key syntax.
//!
//! ```toml
//! [[game]]
//! id = "cave-runner"
//! name = "Cave Runner"
//! file = "games/mm.sna"
//! [game.keys]
//! "note.F3" = "key.O"
//! ```

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Buttons, notes and encoder directions: the size of the key map.
pub const INPUTS: usize = 55;
/// Action byte of "no action".
pub const NONE: u8 = 0xFF;
pub const NAME_MAX: usize = 28;
pub const CROP_DEFAULT: u8 = 8;
pub const CROP_MAX: u8 = 16;

const BUTTONS: [&str; 14] = ["OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
const RESERVED: [&str; 2] = ["HOME", "PLAY"];
const ENCODERS: [&str; 7] = ["SELECT", "ALGORITHM", "K1", "K2", "K3", "K4", "PRESETS"];
const BUTTON_COUNT: usize = 14;
const NOTE_COUNT: usize = 27;
const PITCHES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Spectrum keys by index (row*5 + bit; row r = address line A(8+r)).
const SPECTRUM_KEYS: [&str; 40] = [
    "CAPS", "Z", "X", "C", "V", //
    "A", "S", "D", "F", "G", //
    "Q", "W", "E", "R", "T", //
    "1", "2", "3", "4", "5", //
    "0", "9", "8", "7", "6", //
    "P", "O", "I", "U", "Y", //
    "ENTER", "L", "K", "J", "H", //
    "SPACE", "SYM", "M", "N", "B",
];
const KEMPSTON: [&str; 5] = ["right", "left", "down", "up", "fire"];

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub id: String,
    pub name: String,
    /// Path relative to the manifest file; see [`game_path`].
    pub file: String,
    pub crop: u8,
    /// Input name to action string; check with [`encode_keymap`].
    pub keys: BTreeMap<String, String>,
    /// Stored verbatim; parsed by the pack writer.
    pub demo: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub game: Vec<Game>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGame {
    id: String,
    name: String,
    file: String,
    crop: Option<i64>,
    #[serde(default)]
    keys: BTreeMap<String, String>,
    demo: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(default)]
    game: Vec<RawGame>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Parses and validates the manifest text. Key strings are checked too.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let raw: RawManifest = toml::from_str(text).map_err(|e| format!("manifest: {e}"))?;
    let mut game: Vec<Game> = Vec::new();
    for g in raw.game {
        let id = g.id;
        if !valid_id(&id) {
            return Err(format!("game id {id:?} must match [a-z0-9-]+"));
        }
        if game.iter().any(|o| o.id == id) {
            return Err(format!("duplicate game id {id:?}"));
        }
        if g.name.is_empty() || !g.name.is_ascii() || g.name.len() > NAME_MAX || g.name.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(format!("game {id}: name must be 1-{NAME_MAX} printable ASCII characters"));
        }
        let crop = match g.crop {
            None => CROP_DEFAULT,
            Some(c) if (0..=CROP_MAX as i64).contains(&c) => c as u8,
            Some(c) => return Err(format!("game {id}: crop {c} is not in 0-{CROP_MAX}")),
        };
        encode_keymap(&g.keys).map_err(|e| format!("game {id}: {e}"))?;
        game.push(Game { id, name: g.name, file: g.file, crop, keys: g.keys, demo: g.demo });
    }
    Ok(Manifest { game })
}

/// The game file's path: `file` resolved against the manifest's directory.
pub fn game_path(manifest_dir: &Path, game: &Game) -> PathBuf {
    manifest_dir.join(&game.file)
}

fn input_index(name: &str) -> Result<usize, String> {
    let bad = || format!("unknown input {name:?}");
    let (kind, rest) = name.split_once('.').ok_or_else(bad)?;
    match kind {
        "button" => {
            if RESERVED.contains(&rest) {
                return Err(format!("{name}: HOME and PLAY are reserved"));
            }
            BUTTONS.iter().position(|b| *b == rest).ok_or_else(bad)
        }
        "note" => (0..NOTE_COUNT).find(|&n| note_name(n) == rest).map(|n| BUTTON_COUNT + n).ok_or_else(bad),
        "enc" => {
            let (enc, dir) = rest.split_once('.').ok_or_else(bad)?;
            let e = ENCODERS.iter().position(|x| *x == enc).ok_or_else(bad)?;
            let d = match dir {
                "ccw" => 0,
                "cw" => 1,
                _ => return Err(bad()),
            };
            Ok(BUTTON_COUNT + NOTE_COUNT + 2 * e + d)
        }
        _ => Err(bad()),
    }
}

/// Note 0 is F3.
fn note_name(n: usize) -> String {
    let s = n + 5; // semitones above C3
    format!("{}{}", PITCHES[s % 12], 3 + s / 12)
}

fn parse_action(a: &str) -> Result<u8, String> {
    let bad = || format!("unknown action {a:?}");
    if a == "none" {
        return Ok(NONE);
    }
    let (kind, rest) = a.split_once('.').ok_or_else(bad)?;
    match kind {
        "key" => SPECTRUM_KEYS.iter().position(|k| *k == rest).map(|i| i as u8).ok_or_else(bad),
        "kempston" => KEMPSTON.iter().position(|k| *k == rest).map(|i| 0x40 | i as u8).ok_or_else(bad),
        _ => Err(bad()),
    }
}

fn parse_actions(v: &str) -> Result<[u8; 2], String> {
    let parts: Vec<&str> = v.split('+').map(str::trim).collect();
    match parts.as_slice() {
        [a] => Ok([parse_action(a)?, NONE]),
        [a, b] => {
            let (a, b) = (parse_action(a)?, parse_action(b)?);
            if a == NONE || b == NONE {
                return Err(format!("{v:?}: none cannot be combined with another action"));
            }
            Ok([a, b])
        }
        _ => Err(format!("{v:?}: at most two actions, joined by +")),
    }
}

fn apply_keys(map: &mut [[u8; 2]; INPUTS], keys: &BTreeMap<String, String>) -> Result<(), String> {
    for (input, value) in keys {
        let i = input_index(input)?;
        map[i] = parse_actions(value).map_err(|e| format!("{input}: {e}"))?;
    }
    Ok(())
}

const DEFAULTS: [(&str, &str); 26] = [
    ("note.F3", "kempston.left"),
    ("note.F#3", "kempston.up"),
    ("note.G3", "kempston.down"),
    ("note.G#3", "kempston.up"),
    ("note.A3", "kempston.right"),
    ("note.C4", "key.ENTER"),
    ("note.D4", "key.SPACE"),
    ("note.E4", "key.Y"),
    ("note.F4", "key.N"),
    ("note.C5", "kempston.fire"),
    ("note.D5", "kempston.fire"),
    ("note.E5", "kempston.fire"),
    ("note.F5", "kempston.fire"),
    ("note.G5", "kempston.fire"),
    ("button.OCT-", "key.1"),
    ("button.OCT+", "key.2"),
    ("button.FX", "key.3"),
    ("button.SCL", "key.4"),
    ("button.ENV", "key.5"),
    ("button.LFO", "key.6"),
    ("button.EDIT", "key.7"),
    ("button.GLO", "key.8"),
    ("button.SAVE", "key.9"),
    ("button.ARP", "key.0"),
    ("button.SEQ", "key.CAPS"),
    ("button.REC", "key.SYM"),
];

/// The built-in key map; mirrors `km_default` in `app/keymap.c`.
pub fn default_keymap() -> [[u8; 2]; INPUTS] {
    let mut map = [[NONE; 2]; INPUTS];
    for (input, value) in DEFAULTS {
        apply_keys(&mut map, &BTreeMap::from([(input.to_string(), value.to_string())])).expect("default table is valid");
    }
    map
}

/// The key map of a game: the default with each listed input replaced.
pub fn encode_keymap(keys: &BTreeMap<String, String>) -> Result<[[u8; 2]; INPUTS], String> {
    let mut map = default_keymap();
    apply_keys(&mut map, keys)?;
    Ok(map)
}
