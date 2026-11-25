// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack's plan (`personal --plan`): a run up to the pack, which may be over the room,
//! writing `plan.json` and no pack; and packs built from games files into folders whose
//! `sound-cache` and `arrange` are symlinks to one shared pair, each build leaving the others'
//! arranged music alone. Every folder holds only generated test programs.

use fmzx_pack::personal::{run_plan, run_with_limit, table, PLAN};
use fmzx_pack::testprogs::{border_sna, tape_tap};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fmzx-personal-plan-{tag}-{}", std::process::id()));
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

/// `games/test/tune.sna`: a tune its learned entry plays, so it has a shadow score.
fn tune_sna() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna")).expect("tune.sna")
}

/// A `.z80` version 3 header for a 128K (hardware mode 4), with no pages.
fn z80_128k_stub() -> Vec<u8> {
    let mut z = vec![0u8; 32 + 54];
    z[30..32].copy_from_slice(&54u16.to_le_bytes());
    z[34] = 4;
    z
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn plan(out: &Path) -> Value {
    let text = std::fs::read_to_string(out.join(PLAN)).expect("plan.json written");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("plan.json parses: {e}\n{text}"))
}

/// The only score folder of a game in an arrange folder, and its key.
fn score_folder(arrange: &Path, game: &str) -> (PathBuf, String) {
    let mut keys: Vec<PathBuf> = std::fs::read_dir(arrange.join(game)).expect("the game's scores").map(|e| e.unwrap().path()).collect();
    assert_eq!(keys.len(), 1, "{keys:?}");
    let folder = keys.remove(0);
    let key = folder.file_name().unwrap().to_string_lossy().into_owned();
    (folder, key)
}

/// A take of the score in `folder` (`take-1/arranged.toml`): its notes an octave down.
fn write_take(folder: &Path, key: &str) {
    let score: toml::Table = toml::from_str(&std::fs::read_to_string(folder.join("score.toml")).unwrap()).unwrap();
    let mut take = format!("key = \"{key}\"\nintro_t = {}\nloop_t = {}\ntake = 1\ngenerator = \"octave down\"\n", score["intro_t"], score["loop_t"]);
    for n in score["note"].as_array().unwrap() {
        let get = |k: &str| n[k].as_integer().unwrap();
        take += &format!(
            "\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = 200\n",
            get("voice"),
            get("start_t"),
            get("end_t"),
            2 * get("period_t")
        );
    }
    write(&folder.join("take-1/arranged.toml"), &take);
}

#[test]
fn a_plan_over_the_room_is_no_error() {
    let games = Tmp::new("games");
    games.file("tune.sna", &tune_sna());
    games.file("tape.tap", &tape_tap());
    games.file("big.z80", &z80_128k_stub());
    let out = Tmp::new("out");
    // the pack and keymaps of an earlier run, which the plan removes
    run_with_limit(&games.0, &out.0, None).expect("fits the real slot");
    assert!(out.0.join("games.bin").exists() && out.0.join("keymaps.json").exists());

    let r = run_plan(&games.0, &out.0, Some(1024)).expect("a plan over the room is no error");
    assert!(r.used > 1024 && r.room == 1024 && r.free == 0);
    assert!(table(&r).contains(" KB over\n"), "{}", table(&r));
    assert!(!out.0.join("games.bin").exists() && !out.0.join("keymaps.json").exists());
    assert!(out.0.join("sound-report.txt").is_file());
    let p = plan(&out.0);
    println!("{}", std::fs::read_to_string(out.0.join(PLAN)).unwrap());
    assert_eq!(p["room"], 1024);
    assert_eq!(p["used"].as_u64(), Some(u64::from(r.used)));
    let list = p["games"].as_array().unwrap();
    // the packed games in pack order (by name), then the one left out
    let ids: Vec<&str> = list.iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["tape", "tune", "big"]);
    for g in list {
        for k in ["id", "name", "file", "how", "packed", "reason", "music", "scores"] {
            assert!(g.get(k).is_some(), "{k} in {g}");
        }
    }
    let (tape, tune, big) = (&list[0], &list[1], &list[2]);
    assert_eq!((tape["how"].as_str(), tape["reason"].is_null()), (Some("tape"), true));
    assert!(tape["file"].as_str().unwrap().ends_with("/tape.tap"));
    assert!(tape["music"].is_string() && tape["scores"].as_array().unwrap().is_empty(), "{tape}");
    // the bytes of the packed games with their music: the pack's, but its header and tables
    let packed: u64 = list.iter().filter_map(|g| g["packed"].as_u64()).sum();
    assert!(packed > 0 && packed < u64::from(r.used), "{packed} of {}", r.used);
    assert!(tune["packed"].as_u64().unwrap() > u64::from(r.rows.iter().find(|w| w.name == "tune").unwrap().packed.unwrap()), "with its music");
    // tune's shadow score: nothing chosen, the original plays
    let scores = tune["scores"].as_array().unwrap();
    assert_eq!(scores.len(), 1, "{tune}");
    let s = &scores[0];
    let (folder, key) = score_folder(&out.0.join("arrange"), "tune");
    assert_eq!((s["key"].as_str(), s["shadow"].as_bool(), s["status"].as_str()), (Some(key.as_str()), Some(true), Some("original")));
    assert!(s["seconds"].as_f64().unwrap() > 0.5, "{s}");
    assert!(tune["music"].as_str().unwrap().starts_with("music learned ("), "{tune}");
    // the 128K file: left out, why
    assert_eq!((big["how"].as_str(), big["packed"].is_null(), big["music"].is_null()), (Some("left out"), true, true));
    assert_eq!(big["reason"].as_str(), Some("128K"));
    assert!(big["scores"].as_array().unwrap().is_empty());

    // a take chosen: the plan says it plays, and the game's line counts it
    write_take(&folder, &key);
    write(&out.0.join("arrange/choices.toml"), &format!("[choices]\n\"tune/{key}\" = 1\n"));
    run_plan(&games.0, &out.0, Some(1024)).expect("plans");
    let p = plan(&out.0);
    let tune = &p["games"][1];
    assert_eq!(tune["scores"][0]["status"].as_str(), Some("take 1 (octave down)"), "{tune}");
    assert!(tune["music"].as_str().unwrap().ends_with(", 1 arranged)"), "{tune}");
    // a take that does not fit: refused, why
    write(&out.0.join("arrange/choices.toml"), &format!("[choices]\n\"tune/{key}\" = 2\n"));
    run_plan(&games.0, &out.0, Some(1024)).expect("plans");
    let status = plan(&out.0)["games"][1]["scores"][0]["status"].as_str().unwrap().to_string();
    assert!(status.starts_with("refused: "), "{status}");

    // a build removes the plan; the pack that fits is written
    run_with_limit(&games.0, &out.0, None).expect("builds");
    assert!(!out.0.join(PLAN).exists() && out.0.join("games.bin").exists());
}

#[test]
fn plan_from_the_command_line() {
    // an image that fills the app slot leaves no room: the plan is still made, exit 0
    let games = Tmp::new("cli-games");
    games.file("border.sna", &border_sna());
    let out = Tmp::new("cli-out");
    let app = out.0.join("app.bin");
    std::fs::File::create(&app).unwrap().set_len(u64::from(fmzx_pack::layout::APP_SLOT)).unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_fmzx-pack"))
        .args(["personal", games.0.to_str().unwrap(), "-o", out.0.to_str().unwrap(), "--app", app.to_str().unwrap(), "--plan"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{said}{}", String::from_utf8_lossy(&o.stderr));
    assert!(said.contains("plan.json: a pack of 1 games") && said.contains("it does not fit"), "{said}");
    assert!(!out.0.join("games.bin").exists());
    let p = plan(&out.0);
    assert_eq!(p["room"], 0);
    assert_eq!(p["games"][0]["name"].as_str(), Some("border"));
}

/// `dir/name` a symlink to `target` (relative to `dir`).
fn link(target: &str, dir: &Path, name: &str) {
    std::os::unix::fs::symlink(target, dir.join(name)).unwrap();
}

#[test]
fn packs_share_their_arranged_music_through_symlinks() {
    // W/all holds the shared sound-cache and arrange; W/pack-1 and W/pack-2 link to them, each
    // built from its own games file
    let games = Tmp::new("share-games");
    games.file("tune.sna", &tune_sna());
    games.file("border.sna", &border_sna());
    let w = Tmp::new("share-w");
    let all = w.0.join("all");
    std::fs::create_dir_all(all.join("sound-cache")).unwrap();
    std::fs::create_dir_all(all.join("arrange")).unwrap();
    let entry = |id: &str, name: &str, file: &str| {
        format!("[[game]]\nid = \"{id}\"\nname = \"{name}\"\nfile = \"{}\"\n", games.0.join(file).display())
    };
    let mut packs = Vec::new();
    for (n, text) in [(1, entry("tune", "tune", "tune.sna")), (2, entry("border", "border", "border.sna"))] {
        let dir = w.0.join(format!("pack-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        link("../all/sound-cache", &dir, "sound-cache");
        link("../all/arrange", &dir, "arrange");
        write(&dir.join("personal-games.toml"), &text);
        packs.push(dir);
    }
    let build = |dir: &Path| run_with_limit(&dir.join("personal-games.toml"), dir, None).expect("builds");

    // pack 1's score goes to the shared folder; a take chosen for it plays
    build(&packs[0]);
    let (folder, key) = score_folder(&all.join("arrange"), "tune");
    write_take(&folder, &key);
    let choices = all.join("arrange/choices.toml");
    write(&choices, &format!("[choices]\n\"tune/{key}\" = 1\n"));
    let r = build(&packs[0]);
    assert!(table(&r).contains(&format!("tune: shadow score {key} plays take 1 (octave down)\n")), "{}", table(&r));
    let score = std::fs::read(folder.join("score.toml")).unwrap();
    let take = std::fs::read(folder.join("take-1/arranged.toml")).unwrap();
    let chosen = std::fs::read(&choices).unwrap();

    // pack 2: pack 1's choice is no score of its pack, and pack 1's music stays as it was
    let r = build(&packs[1]);
    assert!(r.arranged.iter().any(|l| l.contains(&format!("\"tune/{key}\" names no score of the pack; ignored"))), "{:?}", r.arranged);
    assert!(packs[1].join("games.bin").is_file());
    assert_eq!(std::fs::read(folder.join("score.toml")).unwrap(), score);
    assert_eq!(std::fs::read(folder.join("take-1/arranged.toml")).unwrap(), take);
    assert_eq!(std::fs::read(&choices).unwrap(), chosen);
    for dir in &packs {
        for name in ["sound-cache", "arrange"] {
            assert!(std::fs::symlink_metadata(dir.join(name)).unwrap().file_type().is_symlink(), "{name} still a link");
        }
    }
    // both games learned into the one cache; pack 1 builds again from it with its take
    assert_eq!(std::fs::read_dir(all.join("sound-cache")).unwrap().count(), 2);
    let r = build(&packs[0]);
    assert!(table(&r).contains("plays take 1 (octave down)"), "{}", table(&r));
}
