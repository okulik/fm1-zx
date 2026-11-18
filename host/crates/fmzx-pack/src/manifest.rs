// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game manifest (TOML) and its key syntax.
//!
//! ```toml
//! [[game]]
//! id = "cave-runner"
//! name = "Cave Runner"
//! file = "games/cave-runner.sna"
//! [game.keys]
//! "note.F3" = "key.O"
//! ```
//!
//! A personal games file (`personal-games.toml`) may give a game the stored roles of its music
//! routines' channels, overriding the learner's analysis (BUILDING.md, "Your own games",
//! Instruments): a routine's speaker `OUT` (4 hex digits, as `sound-report.txt` shows it) to its
//! channels' roles, from channel 0, each `"bass"`, `"lead"` or `"pad"`.
//!
//! ```toml
//! [game.instruments]
//! "C694" = ["lead"]
//! "881F" = ["bass", "lead"]
//! ```

use serde::Deserialize;
use soundlearn::report::ROLES;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Buttons, notes and encoder directions: the size of the key map.
pub const INPUTS: usize = 55;
/// Action byte of "no action".
pub const NONE: u8 = 0xFF;
pub const NAME_MAX: usize = 28;
pub const CROP_DEFAULT: u8 = 8;
pub const CROP_MAX: u8 = 16;

pub const BUTTONS: [&str; 14] = ["OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
const RESERVED: [&str; 2] = ["HOME", "PLAY"];
pub const ENCODERS: [&str; 7] = ["SELECT", "ALGORITHM", "K1", "K2", "K3", "K4", "PRESETS"];
pub const BUTTON_COUNT: usize = 14;
pub const NOTE_COUNT: usize = 27;
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
    /// A music routine's speaker `OUT` to its channels' stored roles (`SND_ROLE_*`, 1-3), from
    /// channel 0; a personal games file's only.
    pub instruments: BTreeMap<u16, Vec<u8>>,
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
    #[serde(default)]
    instruments: BTreeMap<String, Vec<String>>,
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
    parse(text, false)
}

/// [`parse_manifest`] for a personal games file (`personal-games.toml`): `file` may also be an
/// absolute path.
pub fn parse_personal_manifest(text: &str) -> Result<Manifest, String> {
    parse(text, true)
}

fn parse(text: &str, absolute_ok: bool) -> Result<Manifest, String> {
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
        if g.file.is_empty() {
            return Err(format!("game {id}: file is empty"));
        }
        if !absolute_ok && (Path::new(&g.file).is_absolute() || g.file.starts_with('/') || g.file.starts_with('\\')) {
            return Err(format!("game {id}: file {:?} must be relative to the manifest", g.file));
        }
        encode_keymap(&g.keys).map_err(|e| format!("game {id}: {e}"))?;
        if !absolute_ok && !g.instruments.is_empty() {
            return Err(format!("game {id}: [game.instruments] belongs in a personal games file"));
        }
        let instruments = parse_instruments(&g.instruments).map_err(|e| format!("game {:?}: [game.instruments] {e}", g.name))?;
        game.push(Game { id, name: g.name, file: g.file, crop, keys: g.keys, demo: g.demo, instruments });
    }
    if game.is_empty() {
        return Err("manifest: no games".to_string());
    }
    Ok(Manifest { game })
}

/// A `[game.instruments]` table: each key 4 hex digits (once, whatever its case), each value 1-3
/// role names.
fn parse_instruments(raw: &BTreeMap<String, Vec<String>>) -> Result<BTreeMap<u16, Vec<u8>>, String> {
    let names: Vec<&str> = ROLES.iter().map(|r| r.0).collect();
    let mut out = BTreeMap::new();
    for (key, roles) in raw {
        let addr = (key.len() == 4 && key.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u16::from_str_radix(key, 16).ok())
            .flatten()
            .ok_or_else(|| format!("{key:?}: not a speaker OUT's address (4 hex digits, as sound-report.txt shows it)"))?;
        if !(1..=3).contains(&roles.len()) {
            return Err(format!("{key:?}: give 1-3 roles, one per channel from channel 0"));
        }
        let roles = roles
            .iter()
            .map(|r| ROLES.iter().find(|x| x.0 == r).map(|x| x.1).ok_or_else(|| format!("{key:?}: {r:?} is not a role ({})", names.join(", "))))
            .collect::<Result<Vec<u8>, String>>()?;
        if out.insert(addr, roles).is_some() {
            return Err(format!("{key:?}: the address is given twice"));
        }
    }
    Ok(out)
}

/// The game file's path: `file` resolved against the manifest's directory (an absolute `file`
/// stays as it is).
pub fn game_path(manifest_dir: &Path, game: &Game) -> PathBuf {
    manifest_dir.join(&game.file)
}

pub fn input_index(name: &str) -> Result<usize, String> {
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
pub fn note_name(n: usize) -> String {
    let s = n + 5; // semitones above C3
    format!("{}{}", PITCHES[s % 12], 3 + s / 12)
}

pub fn parse_action(a: &str) -> Result<u8, String> {
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

/// The inverse of [`input_index`]: `button.OCT-`, `note.F#3` or `enc.K1.cw`.
pub fn input_name(i: usize) -> String {
    if i < BUTTON_COUNT {
        format!("button.{}", BUTTONS[i])
    } else if i < BUTTON_COUNT + NOTE_COUNT {
        format!("note.{}", note_name(i - BUTTON_COUNT))
    } else {
        let e = i - BUTTON_COUNT - NOTE_COUNT;
        format!("enc.{}.{}", ENCODERS[e / 2], if e % 2 == 0 { "ccw" } else { "cw" })
    }
}

/// The inverse of [`parse_action`]: `None` for [`NONE`], else `key.SPACE` or `kempston.left`.
pub fn action_name(a: u8) -> Option<String> {
    if a == NONE {
        None
    } else if a & 0x40 != 0 {
        KEMPSTON.get((a & 0x3F) as usize).map(|k| format!("kempston.{k}"))
    } else {
        SPECTRUM_KEYS.get(a as usize).map(|k| format!("key.{k}"))
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

pub const DEFAULTS: [(&str, &str); 32] = [
    ("note.F3", "kempston.down"),
    ("note.F#3", "kempston.up"),
    ("note.G3", "kempston.down"),
    ("note.G#3", "kempston.up"),
    ("note.A3", "kempston.down"),
    ("note.A#3", "kempston.up"),
    ("note.B3", "kempston.down"),
    ("note.C4", "key.ENTER"),
    ("note.C#4", "kempston.left"),
    ("note.D4", "key.SPACE"),
    ("note.D#4", "kempston.right"),
    ("note.E4", "key.Y"),
    ("note.F4", "key.N"),
    ("note.C5", "kempston.fire"),
    ("note.C#5", "kempston.left"),
    ("note.D5", "kempston.fire"),
    ("note.D#5", "kempston.right"),
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
