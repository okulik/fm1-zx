// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::keymaps::keymaps_json;
use fmzx_pack::manifest::*;

#[test]
fn input_names_round_trip() {
    for i in 0..INPUTS {
        let n = input_name(i);
        // HOME and PLAY are reserved: input_index refuses them, but the name still decodes.
        if n == "button.HOME" || n == "button.PLAY" {
            continue;
        }
        assert_eq!(input_index(&n), Ok(i), "{n}");
    }
    assert_eq!(input_name(0), "button.OCT-");
    assert_eq!(input_name(14), "note.F3");
}

#[test]
fn action_names_round_trip() {
    let mut all: Vec<String> = ["kempston.fire", "key.CAPS"].iter().map(|s| s.to_string()).collect();
    for (_, a) in DEFAULTS {
        all.push(a.to_string());
    }
    for s in all {
        assert_eq!(action_name(parse_action(&s).unwrap()), Some(s.clone()), "{s}");
    }
    assert_eq!(action_name(NONE), None);
}

/// The entry for `input` in `game`'s object, or None.
fn entry<'a>(json: &'a str, game: &str, input: &str) -> Option<&'a str> {
    let start = json.find(&format!("{{\"id\":\"{game}\""))?;
    let rest = &json[start..];
    let end = rest[1..].find("{\"id\":").map_or(rest.len(), |e| e + 1);
    let obj = &rest[..end];
    let k = obj.find(&format!("{{\"input\":\"{input}\""))?;
    let tail = &obj[k..];
    Some(&tail[..tail.find("]}")? + 2])
}

#[test]
fn games_toml_keymaps() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/games.toml");
    let m = parse_manifest(&std::fs::read_to_string(path).unwrap()).unwrap();
    let json = keymaps_json(&m).unwrap();
    let ids: Vec<usize> = ["scroll", "border", "mooncaves", "castaway"].iter().map(|g| json.find(&format!("\"id\":\"{g}\"")).unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "game order");
    assert_eq!(json.matches("\"id\":").count(), 4);
    assert!(entry(&json, "scroll", "note.F3").unwrap().ends_with("\"actions\":[\"kempston.left\"]}"));
    assert!(entry(&json, "castaway", "note.F3").unwrap().ends_with("\"actions\":[\"key.O\"]}"));
    assert!(entry(&json, "castaway", "note.G3").is_none());
    assert!(entry(&json, "mooncaves", "note.B3").unwrap().ends_with("\"actions\":[\"key.M\"]}"));
    assert!(!json.contains("button.HOME") && !json.contains("button.PLAY"));
}

#[test]
fn json_escapes_names() {
    let m = parse_manifest("[[game]]\nid = \"a\"\nname = 'Say \"hi\" \\ bye'\nfile = \"a.sna\"\n").unwrap();
    let json = keymaps_json(&m).unwrap();
    assert!(json.contains(r#""name":"Say \"hi\" \\ bye""#), "{json}");
    assert!(json.starts_with("{\"games\":[{\"id\":\"a\",") && json.trim_end().ends_with("]}]}"));
}
