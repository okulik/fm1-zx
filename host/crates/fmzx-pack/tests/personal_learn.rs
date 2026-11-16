// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack learns each game's music: `tune.sna`'s routine gets a learned entry in the
//! pack's sound section, the border tone and the scroller get none, the result lines and
//! `sound-report.txt` say so, and a second run reads the cache (a corrupt cache file is learned
//! again). Games the learner cannot read
//! (a machine halted for good, a speaker loop with an undecoded instruction) get "no music
//! learned" and the pack still builds. A key script beside a game is part of the cache key, and
//! one that does not parse is reported. The tune's one channel gets the lead as its stored role,
//! an override in the games file wins and is reported, and a bad override fails the build with
//! the game's name. Every folder holds only generated test programs.

use fmzx_pack::pack::sound_list;
use fmzx_pack::personal::{run_with_limit, table, Report};
use fmzx_pack::testprogs::{border_sna, scroll_sna, tune_sna};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zx_sys::{pack_game_at, pack_open};

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fmzx-personal-learn-{tag}-{}", std::process::id()));
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

/// The pack's games by name: (name, its learned list's entry count, 0 for none).
fn learned(out: &Path) -> Vec<(String, u8)> {
    let pack = std::fs::read(out.join("games.bin")).expect("games.bin written");
    let n = pack_open(&pack).expect("the pack opens");
    (0..n).map(|i| (pack_game_at(&pack, i).unwrap().name(), sound_list(&pack, i).map_or(0, |l| l[0]))).collect()
}

/// A 48K `.sna` of `ram` (48K from 4000h) starting at `pc` (popped from the stack at `sp`), IM 1,
/// interrupts off.
fn sna_of(ram: Vec<u8>, sp: u16, pc: u16) -> Vec<u8> {
    let mut ram = ram;
    let s = usize::from(sp - 0x4000);
    ram[s..s + 2].copy_from_slice(&pc.to_le_bytes());
    let mut h = vec![0x3Fu8]; // I
    h.extend_from_slice(&[0; 18]); // HL' DE' BC' AF' HL DE BC IY IX
    h.extend_from_slice(&[0, 0, 0, 0]); // IFF2 clear, R, AF
    h.extend_from_slice(&sp.to_le_bytes());
    h.extend_from_slice(&[1, 0]); // IM 1, border 0
    h.extend_from_slice(&ram);
    h
}

/// A RAM full of `HALT` with interrupts off: the machine halts for good, silent.
fn halt_sna() -> Vec<u8> {
    sna_of(vec![0x76; 0xC000], 0xC000, 0x8000)
}

/// A square wave whose loop holds an `ED FF` (an undefined instruction) next to its `OUT`.
fn edff_sna() -> Vec<u8> {
    #[rustfmt::skip]
    let code = [
        0xF3,             // DI
        0xAF,             // XOR A
        0xEE, 0x10,       // loop: XOR 10h
        0xD3, 0xFE,       //       OUT (FEh),A
        0xED, 0xFF,       //       (undefined)
        0x06, 0x40,       //       LD B,40h
        0x10, 0xFE,       // d:    DJNZ d
        0x18, 0xF4,       //       JR loop
    ];
    let mut ram = vec![0u8; 0xC000];
    ram[0x4000..0x4000 + code.len()].copy_from_slice(&code);
    sna_of(ram, 0xC000, 0x8000)
}

/// The result line for `name` among the table's lines.
fn line<'a>(text: &'a str, name: &str) -> &'a str {
    let head = format!("{name}: ");
    text.lines().find(|l| l.starts_with(&head) && l.contains("music learned")).unwrap_or_else(|| panic!("no line for {name} in\n{text}"))
}

fn run(games: &Path, out: &Path) -> (Report, Duration) {
    let t = Instant::now();
    let r = run_with_limit(games, out, None).expect("builds");
    (r, t.elapsed())
}

#[test]
fn personal_learns_tune() {
    let games = Tmp::new("tune");
    games.file("tune.sna", &tune_sna());
    games.file("border.sna", &border_sna());
    games.file("scroll.sna", &scroll_sna());
    let out = Tmp::new("tune-out");
    let (r, first) = run(&games.0, &out.0);
    let shown = table(&r);
    println!("{shown}first run {first:?}");

    // the pack: the tune has one learned entry, the border tone and the scroller none
    let mut got = learned(&out.0);
    got.sort();
    assert_eq!(got, [("border".to_string(), 0), ("scroll".to_string(), 0), ("tune".to_string(), 1)]);

    // the lines after the size table, and the details
    assert!(shown.find("used ").unwrap() < shown.find("tune: music learned").unwrap(), "{shown}");
    assert!(line(&shown, "tune").starts_with("tune: music learned (1 routine, heard, 1 channel)"), "{shown}");
    assert!(line(&shown, "border").starts_with("border: no music learned ("), "{shown}");
    assert!(line(&shown, "scroll").starts_with("scroll: no music learned ("), "{shown}");
    let report = std::fs::read_to_string(out.0.join("sound-report.txt")).expect("sound-report.txt");
    println!("{report}");
    let tune = &report[report.find("tune: ").expect("the tune's block")..];
    let tune = &tune[..tune.find("\n\n").unwrap_or(tune.len())];
    assert!(tune.contains("8014h") && tune.contains("heard") && tune.contains("learned"), "{tune}");
    assert!(report.contains("border: no music learned") && report.contains("scroll: no music learned"), "{report}");

    // the second run reads the cache: the same pack and lines, in well under a second
    let cache = out.0.join("sound-cache");
    assert_eq!(std::fs::read_dir(&cache).expect("sound-cache/").count(), 3);
    let pack = std::fs::read(out.0.join("games.bin")).unwrap();
    let (again, second) = run(&games.0, &out.0);
    println!("second run {second:?}");
    assert_eq!(table(&again), shown);
    assert_eq!(std::fs::read(out.0.join("games.bin")).unwrap(), pack);
    assert_eq!(std::fs::read_to_string(out.0.join("sound-report.txt")).unwrap(), report);
    assert!(second < Duration::from_secs(1), "the cached run took {second:?}");
}

#[test]
fn learn_never_fails_the_pack() {
    let games = Tmp::new("never");
    games.file("halt.sna", &halt_sna());
    games.file("edff.sna", &edff_sna());
    let out = Tmp::new("never-out");
    let (r, _) = run(&games.0, &out.0);
    let shown = table(&r);
    println!("{shown}");
    assert!(line(&shown, "halt").starts_with("halt: no music learned ("), "{shown}");
    let edff = line(&shown, "edff");
    assert!(edff.starts_with("edff: no music learned (") && edff.contains("not decoded"), "{shown}");
    let mut got = learned(&out.0);
    got.sort();
    assert_eq!(got, [("edff".to_string(), 0), ("halt".to_string(), 0)]);
}

#[test]
fn keys_file_guides_the_run() {
    let games = Tmp::new("keys");
    games.file("tune.sna", &tune_sna());
    games.file("tune.keys", b"key.ENTER@1000, key.SPACE@2000+40\n");
    let out = Tmp::new("keys-out");
    let (r, _) = run(&games.0, &out.0);
    assert!(line(&table(&r), "tune").starts_with("tune: music learned ("), "{}", table(&r));
    let cache = out.0.join("sound-cache");
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);

    // a key script that does not parse: said in the report, the generic run used, and a cache
    // entry of its own (the script is part of the key)
    games.file("tune.keys", b"key.NOPE@1000\n");
    let (r, _) = run(&games.0, &out.0);
    assert!(line(&table(&r), "tune").starts_with("tune: music learned ("), "{}", table(&r));
    let report = std::fs::read_to_string(out.0.join("sound-report.txt")).unwrap();
    assert!(report.contains("tune.keys: ") && report.contains("the generic run was used"), "{report}");
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 2);
}

#[test]
fn corrupt_cache_is_relearned() {
    let games = Tmp::new("corrupt");
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("corrupt-out");
    run(&games.0, &out.0);
    let pack = std::fs::read(out.0.join("games.bin")).unwrap();
    let cache = std::fs::read_dir(out.0.join("sound-cache")).unwrap().next().expect("one cache file").unwrap().path();
    let good = std::fs::read(&cache).unwrap();
    // a byte of the learned entry's digest (26 bytes in: past the cache file's head and the
    // list's and entry's first bytes), so the list's structure stays intact
    let mut bad = good.clone();
    bad[26] ^= 0x5A;
    std::fs::write(&cache, &bad).unwrap();
    let (r, _) = run(&games.0, &out.0);
    assert!(line(&table(&r), "tune").starts_with("tune: music learned ("), "{}", table(&r));
    assert!(std::fs::read(out.0.join("games.bin")).unwrap() == pack, "the corrupt list went into the pack");
    assert!(std::fs::read(&cache).unwrap() == good, "the cache file is not learned again and rewritten");
}

/// The game `name`'s learned entry's `instrument[]` bytes in the pack (its list holds that one
/// entry), and its block of `sound-report.txt`.
fn stored_roles(out: &Path, name: &str) -> ([u8; 3], String) {
    let pack = std::fs::read(out.join("games.bin")).expect("games.bin written");
    let n = pack_open(&pack).expect("the pack opens");
    let i = (0..n).find(|&i| pack_game_at(&pack, i).unwrap().name() == name).expect("the game");
    let l = sound_list(&pack, i).expect("a sound list");
    assert_eq!(l[0], 1, "one entry");
    let report = std::fs::read_to_string(out.join("sound-report.txt")).expect("sound-report.txt");
    let block = &report[report.find(&format!("\n{name}: ")).expect("the game's block")..];
    let block = &block[..block[1..].find("\n\n").map_or(block.len(), |e| e + 1)];
    (l[4 + 29..4 + 32].try_into().unwrap(), block.to_string())
}

#[test]
fn tune_gets_lead_role() {
    let games = Tmp::new("roles");
    games.file("tune.sna", &tune_sna());
    let out = Tmp::new("roles-out");
    run(&games.0, &out.0);
    let (roles, block) = stored_roles(&out.0, "tune");
    println!("{block}");
    assert_eq!(roles, [zx_sys::SND_ROLE_LEAD, 0, 0]);
    assert!(block.contains("\n    roles: channel 0 lead (analysis)\n"), "{block}");
}

/// A games file holding `tune.sna` as "Tune Game", with `instruments` as its `[game.instruments]`
/// table's lines.
fn games_file(dir: &Tmp, instruments: &str) -> PathBuf {
    dir.file("tune.sna", &tune_sna());
    let text = format!("[[game]]\nid = \"tune\"\nname = \"Tune Game\"\nfile = \"tune.sna\"\n[game.instruments]\n{instruments}\n");
    dir.file("games.toml", text.as_bytes());
    dir.0.join("games.toml")
}

#[test]
fn override_wins_and_is_reported() {
    let games = Tmp::new("override");
    let out = Tmp::new("override-out");
    run(&games_file(&games, "\"8014\" = [\"bass\"]"), &out.0);
    let (roles, block) = stored_roles(&out.0, "Tune Game");
    println!("{block}");
    assert_eq!(roles, [zx_sys::SND_ROLE_BASS, 0, 0]);
    assert!(block.contains("\n    roles: channel 0 bass (override)\n"), "{block}");

    // the overrides are part of the cache key: another one is learned anew, not read back
    run(&games_file(&games, "\"8014\" = [\"pad\"]"), &out.0);
    let (roles, block) = stored_roles(&out.0, "Tune Game");
    assert_eq!(roles, [zx_sys::SND_ROLE_PAD, 0, 0]);
    assert!(block.contains("\n    roles: channel 0 pad (override)\n"), "{block}");
    assert_eq!(std::fs::read_dir(out.0.join("sound-cache")).unwrap().count(), 2);
}

#[test]
fn bad_override_fails_with_game_name() {
    let games = Tmp::new("bad-override");
    let out = Tmp::new("bad-override-out");
    run(&games_file(&games, ""), &out.0);
    assert!(out.0.join("games.bin").is_file());
    for (table, key) in [
        ("\"9999\" = [\"lead\"]", "\"9999\""),     // no routine's OUT
        ("\"8014\" = [\"drums\"]", "\"drums\""),   // no such role
        ("\"801\" = [\"lead\"]", "\"801\""),       // not 4 hex digits
        ("\"8014\" = [\"lead\", \"bass\"]", "\"8014\""), // 2 roles, 1 channel
    ] {
        let err = run_with_limit(&games_file(&games, table), &out.0, None).expect_err(table);
        println!("{err}");
        assert!(err.contains("Tune Game") && err.contains(key), "{table}: {err}");
        assert!(!out.0.join("games.bin").exists(), "{table}: a pack was written");
    }
}
