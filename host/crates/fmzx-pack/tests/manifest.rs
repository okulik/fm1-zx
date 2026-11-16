// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::manifest::*;
use std::collections::BTreeMap;
use std::path::Path;

const MINIMAL: &str = r#"
[[game]]
id = "cave-runner"
name = "Cave Runner"
file = "games/mm.sna"
"#;

fn keys(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn parse_minimal() {
    let m = parse_manifest(MINIMAL).unwrap();
    assert_eq!(m.game.len(), 1);
    let g = &m.game[0];
    assert_eq!(g.id, "cave-runner");
    assert_eq!(g.name, "Cave Runner");
    assert_eq!(g.file, "games/mm.sna");
    assert_eq!(g.crop, 8);
    assert!(g.keys.is_empty());
    assert_eq!(g.demo, None);
    assert_eq!(game_path(Path::new("/x/y"), g), Path::new("/x/y/games/mm.sna"));
}

#[test]
fn parse_full() {
    let m = parse_manifest(
        r#"
[[game]]
id = "a"
name = "A"
file = "a.tap"
crop = 16
demo = "5:UP"
[game.keys]
"button.FX" = "key.Q+key.W"

[[game]]
id = "b"
name = "B"
file = "b.z80"
crop = 0
"#,
    )
    .unwrap();
    assert_eq!(m.game.len(), 2);
    assert_eq!(m.game[0].crop, 16);
    assert_eq!(m.game[0].demo.as_deref(), Some("5:UP"));
    assert_eq!(m.game[0].keys["button.FX"], "key.Q+key.W");
    assert_eq!(m.game[1].crop, 0);
}

#[test]
fn parse_rejects_bad_manifests() {
    let bad = |extra: &str| parse_manifest(&format!("[[game]]\nid = \"a\"\nname = \"A\"\nfile = \"f\"\n{extra}")).is_err();
    assert!(bad("crop = 17"));
    assert!(bad("crop = -1"));
    assert!(bad("bogus = 1"));
    assert!(!bad("crop = 0"));
    let with = |id: &str, name: &str| parse_manifest(&format!("[[game]]\nid = \"{id}\"\nname = \"{name}\"\nfile = \"f\"\n"));
    assert!(with("Bad_Id", "A").is_err());
    assert!(with("", "A").is_err());
    assert!(with("a", &"x".repeat(29)).is_err());
    assert!(with("a", &"x".repeat(28)).is_ok());
    assert!(with("a", "caf\u{e9}").is_err());
    assert!(parse_manifest("[[game]]\nid = \"a\"\nfile = \"f\"\n").is_err());
    let dup = format!("{MINIMAL}{MINIMAL}");
    assert!(parse_manifest(&dup).is_err());
    assert!(parse_manifest("[[game]]\nid=\"a\"\nname=\"A\"\nfile=\"f\"\n[game.keys]\n\"button.HOME\" = \"key.A\"\n").is_err());
}

#[test]
fn override_note() {
    let base = default_keymap();
    let m = encode_keymap(&keys(&[("note.F3", "key.O")])).unwrap();
    assert_eq!(m[14], [5 * 5 + 1, 0xFF]);
    for i in 0..INPUTS {
        if i != 14 {
            assert_eq!(m[i], base[i], "input {i}");
        }
    }
}

#[test]
fn other_inputs() {
    let m = encode_keymap(&keys(&[
        ("button.OCT-", "none"),
        ("note.G5", "kempston.left+key.SPACE"),
        ("note.F#3", "key.CAPS+key.SYM"),
        ("enc.SELECT.ccw", "key.0"),
        ("enc.PRESETS.cw", "key.Z"),
    ]))
    .unwrap();
    assert_eq!(m[0], [0xFF, 0xFF]);
    assert_eq!(m[14 + 26], [0x41, 35]);
    assert_eq!(m[15], [0, 36]);
    assert_eq!(m[41], [20, 0xFF]);
    assert_eq!(m[42 + 12], [1, 0xFF]);
}

#[test]
fn reserved_home_rejected() {
    assert!(encode_keymap(&keys(&[("button.HOME", "key.A")])).is_err());
    assert!(encode_keymap(&keys(&[("button.PLAY", "none")])).is_err());
}

#[test]
fn unknown_key_rejected() {
    for (k, v) in [
        ("button.FX", "key.ESC"),
        ("button.FX", "key.a"),
        ("button.FX", "joy.up"),
        ("button.FX", "kempston.jump"),
        ("button.FX", ""),
        ("button.NOPE", "key.A"),
        ("note.C6", "key.A"),
        ("note.E3", "key.A"),
        ("enc.K1", "key.A"),
        ("enc.K9.cw", "key.A"),
        ("enc.K1.up", "key.A"),
        ("knob", "key.A"),
    ] {
        assert!(encode_keymap(&keys(&[(k, v)])).is_err(), "{k} = {v:?}");
    }
}

#[test]
fn three_actions_rejected() {
    assert!(encode_keymap(&keys(&[("button.FX", "key.A+key.B+key.C")])).is_err());
    assert!(encode_keymap(&keys(&[("button.FX", "key.A+none")])).is_err());
}

#[test]
fn default_matches_c() {
    assert_eq!(default_keymap(), zx_sys::km_default());
}

#[test]
fn default_joystick() {
    let m = default_keymap();
    let at = |n: &str| m[input_index(n).unwrap()][0];
    for n in ["note.F3", "note.G3", "note.A3", "note.B3"] {
        assert_eq!(at(n), parse_action("kempston.down").unwrap(), "{n}");
    }
    for n in ["note.F#3", "note.G#3", "note.A#3"] {
        assert_eq!(at(n), parse_action("kempston.up").unwrap(), "{n}");
    }
    for (n, a) in [("note.C#4", "left"), ("note.D#4", "right"), ("note.C#5", "left"), ("note.D#5", "right")] {
        assert_eq!(at(n), parse_action(&format!("kempston.{a}")).unwrap(), "{n}");
    }
    for n in ["note.C5", "note.D5", "note.E5", "note.F5", "note.G5"] {
        assert_eq!(at(n), parse_action("kempston.fire").unwrap(), "{n}");
    }
    assert_eq!(at("note.F#4"), NONE);
}

#[test]
fn rejects_empty_file() {
    let e = parse_manifest("[[game]]\nid = \"a\"\nname = \"A\"\nfile = \"\"\n").unwrap_err();
    assert!(e.contains("file") && e.contains("empty"), "{e}");
}

#[test]
fn rejects_absolute_file() {
    let e = parse_manifest("[[game]]\nid = \"a\"\nname = \"A\"\nfile = \"/etc/a.sna\"\n").unwrap_err();
    assert!(e.contains("file") && e.contains("relative"), "{e}");
}

#[test]
fn rejects_no_games() {
    let e = parse_manifest("").unwrap_err();
    assert!(e.contains("no games"), "{e}");
}
