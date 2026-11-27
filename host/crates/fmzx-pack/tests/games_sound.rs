// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The built-in games' learned music (`games/sound.toml`, `fmzx_pack::games_sound`): the built-in
//! pack carries its lists, each at its game, and a sound file that does not fit the games is
//! refused. (`tools/games-sound --check` checks that the file is what the learning gives now.)

use fmzx_pack::games_sound::lists;
use fmzx_pack::manifest::parse_manifest;
use fmzx_pack::pack::{build_pack, sound_list};
use std::path::{Path, PathBuf};
use zx_sys::{pack_sound_at, SndEngines};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn built_in_pack_carries_the_games_music() {
    let path = repo().join("games/games.toml");
    let m = parse_manifest(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(m.sound.as_deref(), Some("sound.toml"));
    let want = lists(&path, &m).expect("games/sound.toml fits games/games.toml");
    let pack = build_pack(&path).unwrap();
    for (i, g) in m.game.iter().enumerate() {
        assert_eq!(sound_list(&pack, i as u32), want[i].as_deref(), "{}", g.id);
        assert_eq!(pack_sound_at(&pack, i as u32), Ok(want[i].clone()), "{}: the firmware's reader", g.id);
        if g.file.starts_with("test/") {
            assert!(want[i].is_none(), "{}: a test program has no learned music", g.id);
        }
    }
    // the two games whose music no built-in entry plays carry their tune (a recorded score each,
    // its notes read from the game's engine); the scores reach method B
    for id in ["blackstar", "magictokens"] {
        let i = m.game.iter().position(|g| g.id == id).expect(id);
        let list = want[i].as_ref().unwrap_or_else(|| panic!("{id}: no learned music"));
        assert_eq!(SndEngines::with_learned(list).expect(id).scores(), 1, "{id}");
    }
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("fmzx-games-sound-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::copy(repo().join("games/test/border.sna"), p.join("border.sna")).unwrap();
        Dir(p)
    }
    /// The games file (one game, `border.sna`, with the sound file `sound.toml`) and that sound
    /// file's `text`.
    fn with_sound(&self, text: &str) -> PathBuf {
        std::fs::write(self.0.join("sound.toml"), text).unwrap();
        let p = self.0.join("games.toml");
        std::fs::write(&p, "sound = \"sound.toml\"\n\n[[game]]\nid = \"border\"\nname = \"Border\"\nfile = \"border.sna\"\n").unwrap();
        p
    }
    fn crc(&self) -> String {
        format!("{:08x}", crc32fast::hash(&std::fs::read(self.0.join("border.sna")).unwrap()))
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A list of one kind-1 record (roles bass, lead, pad for a built-in entry), in hex.
const ROLES: &str = "01000000 10000103 0000000000000000 01020300";

fn entry(id: &str, crc: &str, list: &str) -> String {
    format!("[[game]]\nid = \"{id}\"\ncrc = \"{crc}\"\nlist = \"\"\"\n{list}\n\"\"\"\n")
}

#[test]
fn sound_file_is_attached() {
    let d = Dir::new("ok");
    let pack = build_pack(&d.with_sound(&entry("border", &d.crc(), ROLES))).unwrap();
    let list: Vec<u8> = (0..ROLES.replace(' ', "").len() / 2).map(|i| u8::from_str_radix(&ROLES.replace(' ', "")[2 * i..2 * i + 2], 16).unwrap()).collect();
    assert_eq!(sound_list(&pack, 0), Some(&list[..]));
    // a sound file with no games: no section
    let pack = build_pack(&d.with_sound("")).unwrap();
    assert_eq!(sound_list(&pack, 0), None);
    assert_eq!(pack_sound_at(&pack, 0), Ok(None));
}

#[test]
fn sound_file_that_does_not_fit_is_refused() {
    let d = Dir::new("bad");
    let err = |text: &str| build_pack(&d.with_sound(text)).unwrap_err();
    let e = err(&entry("other", &d.crc(), ROLES));
    assert!(e.contains("game \"other\" is not in the games file"), "{e}");
    let e = err(&entry("border", "00000000", ROLES));
    assert!(e.contains("its music was learned from another file") && e.contains("run tools/games-sound"), "{e}");
    let e = err(&entry("border", &d.crc(), "01000000 10000703 0000000000000000 01020300"));
    assert!(e.contains("the firmware's decoder rejects its list"), "{e}");
    let e = err(&entry("border", &d.crc(), "0100"));
    assert!(e.contains("not a multiple of 4"), "{e}");
    let e = err(&entry("border", &d.crc(), "01zz"));
    assert!(e.contains("list is not hex bytes"), "{e}");
    let e = err(&(entry("border", &d.crc(), ROLES) + &entry("border", &d.crc(), ROLES)));
    assert!(e.contains("given twice"), "{e}");
    std::fs::remove_file(d.0.join("sound.toml")).unwrap();
    let e = build_pack(&d.0.join("games.toml")).unwrap_err();
    assert!(e.contains("sound.toml"), "{e}");
}

#[test]
fn sound_key_is_relative() {
    let game = "[[game]]\nid = \"a\"\nname = \"A\"\nfile = \"a.sna\"\n";
    assert!(parse_manifest(&format!("sound = \"/abs/sound.toml\"\n{game}")).unwrap_err().contains("relative to the manifest"));
    assert!(parse_manifest(&format!("sound = \"\"\n{game}")).is_err());
    assert_eq!(parse_manifest(game).unwrap().sound, None);
}
