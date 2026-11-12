// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every game in `games/games.toml` loads and runs in the host core.

use std::path::{Path, PathBuf};
use zx_tests::*;

/// Snapshots run this many frames; tapes, counted from the autostart, five times as many.
const FRAMES: u32 = 300;
const TAPE_FRAMES: u32 = 1_500;
/// Frames before the end at which the screen is compared.
const LOOKBACK: u32 = 50;

fn games_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games")
}

/// `(id, file)` of each `[[game]]` table; just enough TOML for our manifest
/// (`fmzx-pack` parses it properly and dev-depends on this crate).
fn manifest_games(text: &str) -> Vec<(String, String)> {
    fn value(line: &str, key: &str) -> Option<String> {
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.split('#').next()?.trim();
        Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
    }
    let mut games = Vec::new();
    let mut id = None;
    let mut in_game = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_game = line == "[[game]]";
            continue;
        }
        if !in_game {
            continue;
        }
        if let Some(v) = value(line, "id") {
            id = Some(v);
        } else if let Some(v) = value(line, "file") {
            games.push((id.take().expect("id before file"), v));
        }
    }
    games
}

fn screen_hash(m: &Machine) -> u32 {
    let bytes: Vec<u8> = (0..192).flat_map(|y| m.screen_line(y)).collect();
    fnv1a(&bytes)
}

fn blank(m: &Machine) -> bool {
    let first = m.screen_line(0)[0];
    (0..192).all(|y| m.screen_line(y).iter().all(|&p| p == first))
}

#[test]
fn manifest_reader_finds_each_game() {
    let text = "[[game]]\nid = \"a\"\nname = \"A = 1\"\nfile = \"x/a.sna\" # comment\n[game.keys]\n\"note.F3\" = \"key.O\"\n\n[[game]]\nid = \"b\"\nfile = \"b.tap\"\n";
    assert_eq!(manifest_games(text), [("a".to_string(), "x/a.sna".to_string()), ("b".to_string(), "b.tap".to_string())]);
}

#[test]
fn each_game_runs() {
    let text = std::fs::read_to_string(games_dir().join("games.toml")).unwrap();
    let games = manifest_games(&text);
    assert!(games.len() >= 3, "the two test programs and at least one game");
    for (id, file) in games {
        let data = std::fs::read(games_dir().join(&file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut m = Machine::new();
        let frames = match Path::new(&file).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("sna") => {
                m.load_sna(&data).unwrap_or_else(|e| panic!("{id}: .sna error {e}"));
                FRAMES
            }
            Some("z80") => {
                m.load_z80(&data).unwrap_or_else(|e| panic!("{id}: .z80 error {e}"));
                FRAMES
            }
            Some("tap") => {
                m.tape_insert(&data).unwrap_or_else(|e| panic!("{id}: .tap error {e}"));
                m.tape_autostart();
                TAPE_FRAMES
            }
            other => panic!("{id}: unknown file type {other:?}"),
        };
        let mut earlier = 0;
        for f in 1..=frames {
            m.run_frame();
            if f == frames - LOOKBACK {
                earlier = screen_hash(&m);
            }
        }
        let last = screen_hash(&m);
        assert!(last != earlier || !blank(&m), "{id}: screen frozen and blank at frame {frames}");
        golden(&format!("game_{id}_frame{frames}"), &format!("{last:08x}"));
    }
}
