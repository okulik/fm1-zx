// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A game of a personal pack's learning workspace (`fmzx-pack personal ... -o W/all`), as
//! `music-preview` and `fmzx-pack engine-notes` find it: its converted file in
//! `W/all/converted/manifest.toml`, its key script and overrides in `W/all/personal-games.toml`,
//! and where a score starts in the run that recorded it.

use crate::keys::parse_keys;
use crate::manifest::{game_path, parse_manifest, parse_personal_manifest};
use crate::personal::{LearnerRun, Sound, GAMES_FILE};
use soundlearn::listen::RunKind;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A game of a workspace: its converted file's name and bytes, and where its key script would be
/// and its overrides.
pub struct GameFiles {
    pub file: String,
    pub data: Vec<u8>,
    pub keys: PathBuf,
    pub instruments: BTreeMap<u16, Vec<u8>>,
}

/// The game `id` of the workspace `all`. Err: no workspace there, no such game, or a file that
/// does not read.
pub fn game_files(all: &Path, id: &str) -> Result<GameFiles, String> {
    let conv = all.join("converted");
    let manifest = conv.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|_| format!("{}: no converted/manifest.toml: not a personal pack's workspace (make it with fmzx-pack personal <games> -o {0})", all.display()))?;
    let games = parse_manifest(&text).map_err(|e| format!("{}: {e}", manifest.display()))?.game;
    let Some(g) = games.iter().find(|g| g.id == id) else {
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        return Err(format!("no game {id:?} in {} (its games: {})", all.display(), ids.join(", ")));
    };
    let at = conv.join(&g.file);
    let data = std::fs::read(&at).map_err(|e| format!("{}: {e}", at.display()))?;
    // the key script and overrides as the run had them: the games file's entry (a workspace made
    // from a games file elsewhere has none here, and its game is learned again if that matters)
    let entry = std::fs::read_to_string(all.join(GAMES_FILE))
        .ok()
        .and_then(|t| parse_personal_manifest(&t).ok())
        .and_then(|m| m.game.into_iter().find(|e| e.id == id));
    let (keys, instruments) = match entry {
        Some(e) => (game_path(all, &e).with_extension("keys"), e.instruments),
        None => (conv.join(format!("{id}.no-key-script")), BTreeMap::new()),
    };
    Ok(GameFiles { file: g.file.clone(), data, keys, instruments })
}

/// The run the game's music was learned under: the learner's result's, else its key script's, else
/// the generic run seeded as the personal pack seeds it.
pub fn learner_run(s: &Sound, g: &GameFiles) -> LearnerRun {
    s.runs.as_ref().map(|r| r.learner.clone()).unwrap_or_else(|| match std::fs::read_to_string(&g.keys).ok().as_deref().map(parse_keys) {
        Some(Ok(steps)) => LearnerRun::Script(steps),
        _ => LearnerRun::Kind(RunKind::Generic, u64::from(crc32(&g.data))),
    })
}

/// The frame a score starts in, in the run that recorded it: from its line in the learner's
/// details (`recorded score: 20.7 s from frame 6378, ... (score C21A-62aabd0d)`, a shadow
/// score's `shadow score ...`), the frame of the edge that starts it.
pub fn score_frame(details: &str, key: &str) -> Option<usize> {
    let tag = format!("(score {key}");
    let line = details.lines().find(|l| {
        let t = l.trim_start();
        (t.starts_with("recorded score") || t.starts_with("shadow score")) && l.contains(&tag)
    })?;
    let rest = &line[line.find(" from frame ")? + " from frame ".len()..];
    rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())].parse().ok()
}

/// CRC-32 (IEEE), as the personal pack seeds a game's generic run.
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |mut c, &b| {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & (c & 1).wrapping_neg());
        }
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scores_frame_from_the_details() {
        let d = "x\n    recorded score: 20.7 s from frame 6378, 3 voices (score C21A-62aabd0d)\n  shadow score: 9 s from frame 12, x (score 9000-00000001; packed only with a chosen take)\n";
        assert_eq!(score_frame(d, "C21A-62aabd0d"), Some(6378));
        assert_eq!(score_frame(d, "9000-00000001"), Some(12));
        assert_eq!(score_frame(d, "9000-00000002"), None);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
