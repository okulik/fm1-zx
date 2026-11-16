// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The resolved key map of every game as JSON, for the browser page.

use crate::manifest::{action_name, basic_keymap, encode_keymap, input_name, Manifest, INPUTS, NONE};

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// One game's `{"id":..,"name":..,"keys":[{"input":..,"actions":[..]}]}`: inputs in index order,
/// inputs without an action left out.
fn game_json(id: &str, name: &str, map: &[[u8; 2]; INPUTS]) -> String {
    let mut keys = Vec::new();
    for (i, acts) in map.iter().enumerate() {
        if acts[0] == NONE {
            continue;
        }
        let names: Vec<String> = acts.iter().filter_map(|&a| action_name(a)).map(|n| json_str(&n)).collect();
        keys.push(format!("{{\"input\":{},\"actions\":[{}]}}", json_str(&input_name(i)), names.join(",")));
    }
    format!("{{\"id\":{},\"name\":{},\"keys\":[{}]}}", json_str(id), json_str(name), keys.join(","))
}

/// `{"basic":{..},"games":[{..}]}`: the picker's Spectrum BASIC (its built-in map), then the games
/// in manifest order (the pack's order: `games[i]` is pack entry `i`).
pub fn keymaps_json(m: &Manifest) -> Result<String, String> {
    let mut games = Vec::new();
    for g in &m.game {
        let map = encode_keymap(&g.keys).map_err(|e| format!("game {}: {e}", g.id))?;
        games.push(game_json(&g.id, &g.name, &map));
    }
    let basic = game_json("basic", "Spectrum BASIC", &basic_keymap());
    Ok(format!("{{\"basic\":{basic},\"games\":[{}]}}\n", games.join(",")))
}
