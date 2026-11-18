// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack (`personal`): a folder of games in, `personal-games.toml`, the converted
//! files and the game pack out. Every folder holds only our generated test programs.

use fmzx_pack::personal::{run, run_with_limit, Report, Row};
use fmzx_pack::testprogs::{border_sna, hang_tzx, multiload_tap, tape_tap, tune_sna, turbo_tzx, TAPE_DONE_BORDER};
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

    // the pack: the loadable games by name, the edited name and keys
    let names: Vec<String> = pack_games(&out.0.join("games.bin")).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["border", "tape", "Turbo  Loader"]);
    let pack = std::fs::read(out.0.join("games.bin")).unwrap();
    let keys = pack_game_at(&pack, 2).unwrap().keys();
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
    assert!(e.contains("no game could be loaded") && e.contains("hang.tzx"), "{e}");
    assert!(!out2.0.join("games.bin").exists());
}

#[test]
fn personal_multiload_label() {
    // a multi-load tape goes in as the snapshot of its first part, and the table says why
    let games = Tmp::new("multi");
    games.file("multi.tap", &multiload_tap());
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("multi-out");
    let r = run(&games.0, &out.0).expect("builds");
    let how = "loaded (the ROM trap took 4 of 5 blocks)";
    assert_eq!(row(&r, "multi.tap").how, how);
    let table = fmzx_pack::personal::table(&r);
    // the "how" column is as wide as its longest entry
    let col = |l: &str| l.find(" KB").unwrap_or_else(|| panic!("{table}"));
    let lines: Vec<&str> = table.lines().filter(|l| l.contains(" KB  ")).collect();
    assert_eq!(lines.len(), 2, "{table}");
    assert!(lines.iter().any(|l| l.contains(how)) && col(lines[0]) == col(lines[1]), "{table}");
}

#[test]
fn personal_all_commented_out() {
    // every entry commented out: the error says so and names the games file, not "no game
    // could be loaded"
    let games = Tmp::new("allcomment");
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("allcomment-out");
    run(&games.0, &out.0).expect("first run");
    let path = out.0.join("personal-games.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let commented: String =
        text.lines().map(|l| if l.starts_with('#') || l.is_empty() { format!("{l}\n") } else { format!("# {l}\n") }).collect();
    std::fs::write(&path, &commented).unwrap();
    let e = run(&games.0, &out.0).expect_err("nothing to build");
    let file = path.canonicalize().unwrap();
    assert!(e.contains("every game in") && e.contains("commented out or missing") && e.contains(&file.display().to_string()), "{e}");
    assert!(!e.contains("no game could be loaded"), "{e}");
    assert!(!out.0.join("games.bin").exists());
    // from a games file whose only game is missing: the same, with the game listed
    let mine = games.0.join("mine.toml");
    std::fs::write(&mine, "[[game]]\nid = \"g\"\nname = \"Gone\"\nfile = \"gone.sna\"\n").unwrap();
    let e = run(&mine, &out.0).expect_err("nothing to build");
    assert!(e.contains("commented out or missing") && e.contains("mine.toml") && e.contains("gone.sna"), "{e}");
    // an empty folder has no games file to name
    let empty = Tmp::new("allcomment-empty");
    let out2 = Tmp::new("allcomment-empty-out");
    let e = run(&empty.0, &out2.0).expect_err("no games");
    assert!(e.contains("no game files"), "{e}");
}

#[test]
fn personal_cli_needs_app() {
    // fmzx-pack personal without --app: a usage error, nothing built
    let games = Tmp::new("cli");
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("cli-out");
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_fmzx-pack"))
        .args(["personal", games.0.to_str().unwrap(), "-o", out.0.to_str().unwrap()])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(err.contains("--app") && err.contains("usage:"), "{err}");
    assert!(!out.0.join("games.bin").exists() && !out.0.join("personal-games.toml").exists());
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
fn personal_pack_is_alphabetical() {
    // the picker shows the pack's order: games by name, whatever order the games file has (a
    // re-run adds new files at the end); the games file and the table keep their own order
    let games = Tmp::new("abc");
    games.file("tune.sna", &tune_sna());
    games.file("border.sna", &border_sna());
    games.file("tape.tap", &tape_tap());
    let text = "[[game]]\nid = \"z\"\nname = \"Zeta\"\nfile = \"tune.sna\"\n\n\
                [[game]]\nid = \"a\"\nname = \"alpha\"\nfile = \"border.sna\"\n\n\
                [[game]]\nid = \"b\"\nname = \"Beta\"\nfile = \"tape.tap\"\n";
    games.file("mine.toml", text.as_bytes());
    let out = Tmp::new("abc-out");
    let r = run(&games.0.join("mine.toml"), &out.0).expect("builds");
    let names: Vec<String> = pack_games(&out.0.join("games.bin")).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["alpha", "Beta", "Zeta"]);
    let table: Vec<&str> = r.rows.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(table, ["Zeta", "alpha", "Beta"]);
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

#[test]
fn personal_rerun_symlink() {
    // a symlinked game file is the same file on every run: one entry, not one more each time
    let store = Tmp::new("link-store");
    store.file("tune.sna", &tune_sna());
    let games = Tmp::new("link");
    std::os::unix::fs::symlink(store.0.join("tune.sna"), games.0.join("my-tune.sna")).unwrap();
    let out = Tmp::new("link-out");
    run(&games.0, &out.0).expect("first run");
    let path = out.0.join("personal-games.toml");
    let first = std::fs::read_to_string(&path).unwrap();
    assert_eq!(first.matches("[[game]]").count(), 1, "{first}");
    for _ in 0..2 {
        run(&games.0, &out.0).expect("re-run");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first, "the re-run added the symlink again");
    }
    assert_eq!(pack_games(&out.0.join("games.bin")).len(), 1);
}

#[test]
fn personal_rerun_keeps_commented_out() {
    let games = Tmp::new("comment");
    games.file("turbo.tzx", &turbo_tzx());
    games.file("tape.tap", &tape_tap());
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("comment-out");
    run(&games.0, &out.0).expect("first run");
    let path = out.0.join("personal-games.toml");
    let first = std::fs::read_to_string(&path).unwrap();
    let tune_file = games.0.canonicalize().unwrap().join("tune.sna");
    let entry = format!("[[game]]\nid = \"tune\"\nname = \"tune\"\nfile = \"{}\"\n", tune_file.display());
    let commented: String = entry.lines().map(|l| format!("# {l}\n")).collect();
    assert!(first.contains(&entry), "{first}");
    std::fs::write(&path, first.replace(&entry, &commented)).unwrap();

    games.file("border.sna", &border_sna());
    let r = run(&games.0, &out.0).expect("second run");
    let second = std::fs::read_to_string(&path).unwrap();
    assert!(second.contains(&commented), "the commented-out entry stays as it was:\n{second}");
    assert!(!second.contains(&entry), "tune was added back:\n{second}");
    assert_eq!(second.matches(&format!("\"{}\"", tune_file.display())).count(), 1, "{second}");
    assert!(second.trim_end().ends_with(&format!("file = \"{}\"", games.0.canonicalize().unwrap().join("border.sna").display())));
    assert!(r.rows.iter().all(|w| !w.file.ends_with("tune.sna")), "tune is not a game of the run: {:?}", r.rows);
    let names: Vec<String> = pack_games(&out.0.join("games.bin")).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["border", "tape", "turbo"]);
}

#[test]
fn personal_refuses_foreign_output() {
    // an output folder with packages of its own (build/fm1, say) is never cleared
    let games = Tmp::new("foreign");
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("foreign-out");
    out.file("games.bin", b"built-in pack");
    out.file("fmzx.fwsc", b"built-in package");
    let e = run(&games.0, &out.0).expect_err("refuses a folder of other packages");
    assert!(e.contains(&out.0.canonicalize().unwrap().display().to_string()) && e.contains("games.bin"), "{e}");
    assert_eq!(std::fs::read(out.0.join("games.bin")).unwrap(), b"built-in pack");
    assert_eq!(std::fs::read(out.0.join("fmzx.fwsc")).unwrap(), b"built-in package");
    assert!(!out.0.join("personal-games.toml").exists());

    // an output folder of an earlier run from a games file is ours: a second run goes ahead
    games.file("mine.toml", b"[[game]]\nid = \"t\"\nname = \"Tune\"\nfile = \"tune.sna\"\n");
    let out2 = Tmp::new("foreign-out2");
    run(&games.0.join("mine.toml"), &out2.0).expect("first run from the games file");
    run(&games.0.join("mine.toml"), &out2.0).expect("second run into the same folder");
}
