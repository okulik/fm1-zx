// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack (`personal`): a folder of games in, `personal-games.toml`, the converted
//! files and the game pack out. Every folder holds only our generated test programs.

use fmzx_pack::personal::{run, run_with_limit, Report, Row};
use fmzx_pack::testprogs::{border_sna, hang_tzx, tape_tap, tune_sna, turbo_tzx, TAPE_DONE_BORDER};
use std::path::{Path, PathBuf};
use zx_sys::{pack_game_at, pack_open, Machine, PACK_MAX_RAW, PACK_SNA, PACK_TAP, PACK_Z80};

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fmzx-personal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(name), bytes).unwrap();
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `.z80` version 3 header for a 128K (hardware mode 4), with no pages.
fn z80_128k_stub() -> Vec<u8> {
    let mut z = vec![0u8; 32 + 54];
    z[30..32].copy_from_slice(&54u16.to_le_bytes());
    z[34] = 4;
    z
}

fn row<'a>(r: &'a Report, file: &str) -> &'a Row {
    r.rows.iter().find(|w| w.file.ends_with(file)).unwrap_or_else(|| panic!("no row for {file}: {:?}", r.rows))
}

/// The pack's games as `fmzx-pack games` checks them: each entry extracts, loads in the host
/// core and runs; returns their names and the machines after the run.
fn pack_games(path: &Path) -> Vec<(String, Machine)> {
    let pack = std::fs::read(path).expect("games.bin written");
    let n = pack_open(&pack).expect("the pack opens");
    (0..n)
        .map(|i| {
            let g = pack_game_at(&pack, i).expect("entry");
            let data = g.extract(PACK_MAX_RAW as usize).expect("extracts");
            let mut m = Machine::new();
            let frames = match g.kind() {
                PACK_SNA => m.load_sna(&data).map(|_| 50).expect("loads"),
                PACK_Z80 => m.load_z80(&data).map(|_| 50).expect("loads"),
                PACK_TAP => {
                    m.tape_insert(&data).expect("inserts");
                    m.tape_autostart();
                    500
                }
                k => panic!("kind {k}"),
            };
            for _ in 0..frames {
                m.run_frame();
            }
            (g.name(), m)
        })
        .collect()
}

#[test]
fn personal_builds_pack() {
    let games = Tmp::new("builds");
    games.file("turbo.tzx", &turbo_tzx());
    games.file("tape.tap", &tape_tap());
    games.file("tune.sna", &tune_sna());
    games.file("big.z80", &z80_128k_stub());
    games.file("notes.txt", b"not a game");
    let out = Tmp::new("builds-out");
    let r = run(&games.0, &out.0).expect("builds");

    assert_eq!(row(&r, "turbo.tzx").how, "loaded");
    assert_eq!(row(&r, "tape.tap").how, "tape");
    assert_eq!(row(&r, "tune.sna").how, "as is");
    let big = row(&r, "big.z80");
    assert_eq!((big.packed, big.reason.as_deref()), (None, Some("128K")));
    assert!(r.rows.iter().all(|w| !w.file.ends_with("notes.txt")), "only game files are taken");
    for f in ["turbo.tzx", "tape.tap", "tune.sna"] {
        let w = row(&r, f);
        assert!(w.packed.is_some_and(|p| p > 0) && w.reason.is_none(), "{w:?}");
    }

    let got = pack_games(&out.0.join("games.bin"));
    assert_eq!(got.len(), 3);
    let used = std::fs::metadata(out.0.join("games.bin")).unwrap().len() as u32;
    assert_eq!(r.used, used);
    assert!(r.free > 0);
    // the turbo game is its loaded snapshot: it starts with the payload loaded (border 4)
    let turbo = got.iter().find(|(n, _)| n == "turbo").expect("the turbo game, named after its file");
    assert_eq!(turbo.1.border(), TAPE_DONE_BORDER);

    // the editable file: one entry per loadable file, absolute paths, no keys table
    let toml = std::fs::read_to_string(out.0.join("personal-games.toml")).unwrap();
    assert_eq!(toml.matches("[[game]]").count(), 3, "{toml}");
    assert!(!toml.contains("big.z80") && !toml.lines().any(|l| l.trim() == "[game.keys]"), "{toml}");
    let abs = games.0.canonicalize().unwrap().join("turbo.tzx");
    assert!(toml.contains(&format!("file = \"{}\"", abs.display())), "{toml}");
    assert!(out.0.join("keymaps.json").exists());
}

#[test]
fn personal_names_from_file_names() {
    let games = Tmp::new("names");
    games.file("Cave Runner (1983)(Some House)[a].sna", &tune_sna());
    games.file("Night Raid II - The Return - 48k.sna", &tune_sna());
    games.file("tune.sna", &tune_sna());
    games.file("Tune - Copy.SNA", &tune_sna());
    let out = Tmp::new("names-out");
    run(&games.0, &out.0).expect("builds");
    let toml = std::fs::read_to_string(out.0.join("personal-games.toml")).unwrap();
    for want in [
        "id = \"cave-runner\"\nname = \"Cave Runner\"\n",
        "id = \"night-raid-ii\"\nname = \"Night Raid II\"\n",
        "id = \"tune\"\nname = \"Tune\"\n",
        "id = \"tune-2\"\nname = \"tune\"\n",
    ] {
        assert!(toml.contains(want), "{want:?} in\n{toml}");
    }
}

#[test]
fn personal_rerun_keeps_edits() {
    let games = Tmp::new("rerun");
    games.file("turbo.tzx", &turbo_tzx());
    games.file("tape.tap", &tape_tap());
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("rerun-out");
    run(&games.0, &out.0).expect("first run");
    let path = out.0.join("personal-games.toml");
    let first = std::fs::read_to_string(&path).unwrap();
    let turbo_file = games.0.canonicalize().unwrap().join("turbo.tzx");
    let old = format!("[[game]]\nid = \"turbo\"\nname = \"turbo\"\nfile = \"{}\"\n", turbo_file.display());
    assert!(first.contains(&old), "{first}");
    let edited_entry = format!(
        "[[game]]\nid = \"turbo\"\nname = \"Turbo  Loader\"  # mine\nfile = \"{}\"\n[game.keys]\n\"note.F3\" = \"key.O\"\n",
        turbo_file.display()
    );
    let edited = first.replace(&old, &edited_entry);
    std::fs::write(&path, &edited).unwrap();

    games.file("border.sna", &border_sna());
    std::fs::remove_file(games.0.join("tune.sna")).unwrap();
    let r = run(&games.0, &out.0).expect("second run");
    let second = std::fs::read_to_string(&path).unwrap();
    assert!(second.contains(&edited_entry), "the edited entry is kept byte for byte:\n{second}");
    // every entry of the first run is still there, in order, with border appended at the end
    let at = |s: &str| second.find(s).unwrap_or_else(|| panic!("{s} not in\n{second}"));
    assert!(at("id = \"tape\"") < at("id = \"tune\"") && at("id = \"tune\"") < at("id = \"turbo\""));
    assert!(at("id = \"turbo\"") < at("id = \"border\""));
    assert!(second.trim_end().ends_with(&format!("file = \"{}\"", games.0.canonicalize().unwrap().join("border.sna").display())));
    // tune stays, under a "# missing:" comment
    let tune_file = games.0.canonicalize().unwrap().join("tune.sna");
    let marker = second.find("# missing:").expect("a missing marker");
    assert!(second[marker..].starts_with(&format!("# missing: {}\n[[game]]\nid = \"tune\"\n", tune_file.display())), "{second}");
    assert_eq!(second.matches("# missing:").count(), 1, "{second}");
    assert!(row(&r, "tune.sna").reason.as_deref().is_some_and(|w| w.contains("missing")), "{:?}", r.rows);

    // the pack: the loadable games, the edited name and keys
    let names: Vec<String> = pack_games(&out.0.join("games.bin")).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["tape", "Turbo  Loader", "border"]);
    let pack = std::fs::read(out.0.join("games.bin")).unwrap();
    let keys = pack_game_at(&pack, 1).unwrap().keys();
    assert_eq!(keys[14][0], 26, "note.F3 is key.O");

    // a third run changes nothing; with tune back, its marker goes
    run(&games.0, &out.0).expect("third run");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), second);
    games.file("tune.sna", &tune_sna());
    run(&games.0, &out.0).expect("fourth run");
    let fourth = std::fs::read_to_string(&path).unwrap();
    assert!(!fourth.contains("# missing:") && fourth.contains(&edited_entry), "{fourth}");
    assert_eq!(pack_games(&out.0.join("games.bin")).len(), 4);
}

#[test]
fn personal_skips_failures() {
    let games = Tmp::new("skips");
    games.file("hang.tzx", &hang_tzx());
    games.file("tune.sna", &tune_sna());
    games.file("bad.sna", &[0u8; 100]);
    let out = Tmp::new("skips-out");
    let r = run(&games.0, &out.0).expect("builds without the failures");
    let hang = row(&r, "hang.tzx");
    assert!(hang.packed.is_none() && hang.reason.as_deref().is_some_and(|w| w.contains("loading")), "{hang:?}");
    assert!(row(&r, "bad.sna").reason.is_some());
    assert_eq!(pack_games(&out.0.join("games.bin")).len(), 1);
    let table = fmzx_pack::personal::table(&r);
    assert!(table.contains("tune") && table.contains("hang.tzx") && table.contains("free"), "{table}");

    // nothing loadable: an error, no pack
    let none = Tmp::new("skips-none");
    none.file("hang.tzx", &hang_tzx());
    let out2 = Tmp::new("skips-none-out");
    let e = run(&none.0, &out2.0).expect_err("no game loads");
    assert!(e.contains("hang.tzx"), "{e}");
    assert!(!out2.0.join("games.bin").exists());
}

#[test]
fn personal_from_toml() {
    // a games file is used as written: relative paths from its folder, nothing rewritten
    let games = Tmp::new("toml");
    games.file("tune.sna", &tune_sna());
    games.file("border.sna", &border_sna());
    let text = "[[game]]\nid = \"b\"\nname = \"Border\"\nfile = \"border.sna\"\n\n[[game]]\nid = \"gone\"\nname = \"Gone\"\nfile = \"gone.sna\"\n";
    games.file("mine.toml", text.as_bytes());
    let out = Tmp::new("toml-out");
    let r = run(&games.0.join("mine.toml"), &out.0).expect("builds");
    assert_eq!(std::fs::read_to_string(games.0.join("mine.toml")).unwrap(), text);
    assert!(!out.0.join("personal-games.toml").exists());
    assert!(row(&r, "gone.sna").reason.is_some());
    let names: Vec<String> = pack_games(&out.0.join("games.bin")).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["Border"]);
    // the games folder is never the output; a game file is not a games file
    assert!(run(&games.0, &games.0).is_err());
    assert!(run(&games.0.join("tune.sna"), &out.0).unwrap_err().contains("games file"));
}

#[test]
fn personal_too_big_fails() {
    let games = Tmp::new("big");
    games.file("turbo.tzx", &turbo_tzx());
    games.file("tape.tap", &tape_tap());
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("big-out");
    // a pack from an earlier run, which must not survive the failure
    run(&games.0, &out.0).expect("fits the real slot");
    assert!(out.0.join("games.bin").exists());
    let e = run_with_limit(&games.0, &out.0, Some(1024)).expect_err("does not fit 1 KB");
    assert!(!out.0.join("games.bin").exists(), "no games.bin after the failure");
    assert!(!out.0.join("keymaps.json").exists());
    assert!(e.contains("does not fit"), "{e}");
    // every game, with its packed size, largest first
    let pos: Vec<usize> = ["turbo", "tape", "tune"].iter().map(|n| e.find(&format!("  {n} ")).unwrap_or_else(|| panic!("{n} in {e}"))).collect();
    let sizes: Vec<f64> = pos
        .iter()
        .map(|&p| {
            let line = e[p..].lines().next().unwrap();
            let words: Vec<&str> = line.split_whitespace().collect();
            let kb = words.iter().position(|&w| w == "KB").unwrap_or_else(|| panic!("{line}"));
            words[kb - 1].parse::<f64>().unwrap_or_else(|_| panic!("{line}"))
        })
        .collect();
    let mut order: Vec<usize> = (0..3).collect();
    order.sort_by_key(|&i| pos[i]);
    for w in order.windows(2) {
        assert!(sizes[w[0]] >= sizes[w[1]], "largest first: {e}");
    }
}
