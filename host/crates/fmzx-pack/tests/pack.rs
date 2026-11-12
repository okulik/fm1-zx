// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::manifest::{encode_keymap, parse_manifest};
use fmzx_pack::pack::build_pack;
use std::path::{Path, PathBuf};
use zx_sys::*;
use zx_tests::*;

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("fmzx-pack-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(name), bytes).unwrap();
    }
    fn manifest(&self, text: &str) -> PathBuf {
        let p = self.0.join("games.toml");
        std::fs::write(&p, text).unwrap();
        p
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tap_fixture() -> Vec<u8> {
    let line = basic_line(10, &[0xEA]); // 10 REM
    let mut tap = tap_header(0, "demo", line.len() as u16, 10, line.len() as u16);
    tap.extend(tap_block(0xFF, &line));
    tap
}

fn game(id: &str, name: &str, file: &str, extra: &str) -> String {
    format!("[[game]]\nid = \"{id}\"\nname = \"{name}\"\nfile = \"{file}\"\n{extra}\n")
}

#[test]
fn build_and_read_back() {
    let d = Dir::new("readback");
    let (regs, ram) = (sample_regs(), sample_ram());
    let sna = make_sna(&regs, &ram, 2);
    let z80 = make_z80_v3(&regs, &ram, 0);
    let tap = tap_fixture();
    d.file("a.sna", &sna);
    d.file("b.Z80", &z80);
    d.file("c.tap", &tap);
    let text = game("a", "Alpha Game", "a.sna", "crop = 4\n[game.keys]\n\"note.F3\" = \"key.O\"\n\"enc.SELECT.cw\" = \"kempston.fire + key.P\"")
        + &game("b", "Beta", "b.Z80", "")
        + &game("c", "Gamma", "c.tap", "crop = 16");
    let manifest = d.manifest(&text);
    let pack = build_pack(&manifest).unwrap();

    assert_eq!(pack_open(&pack), Ok(3));
    let m = parse_manifest(&text).unwrap();
    let inputs: [(&[u8], u8); 3] = [(&sna, PACK_SNA), (&z80, PACK_Z80), (&tap, PACK_TAP)];
    for (i, (bytes, kind)) in inputs.iter().enumerate() {
        let g = pack_game_at(&pack, i as u32).unwrap();
        assert_eq!(g.name(), m.game[i].name);
        assert_eq!(g.kind(), *kind);
        assert_eq!(g.crop_x(), m.game[i].crop);
        assert_eq!(g.keys(), encode_keymap(&m.game[i].keys).unwrap());
        assert_eq!(g.raw_len() as usize, bytes.len());
        assert_eq!(g.extract(PACK_MAX_RAW as usize).unwrap(), *bytes);
    }
    // the snapshots are compressible
    assert_eq!(pack_game_at(&pack, 0).unwrap().codec(), PACK_LZ4);
    assert!(pack_game_at(&pack, 0).unwrap().stored_len() < sna.len() as u32);
    assert_eq!(pack_game_at(&pack, 0).unwrap().keys()[14], [26, KM_NONE]); // note.F3 -> key.O
}

#[test]
fn validation_loads_each_game() {
    let (regs, ram) = (sample_regs(), sample_ram());
    let cases: [(&str, Vec<u8>, &str); 4] = [
        ("bad.sna", make_sna(&regs, &ram, 0)[..1000].to_vec(), "not a loadable .sna"),
        ("bad.z80", vec![0u8; 20], "not a loadable .z80"),
        ("bad.tap", vec![0x20, 0x00, 0x00], "not a valid .tap"), // a block that overruns the data
        ("bad.SNA", b"garbage".to_vec(), "not a loadable .sna"),
    ];
    for (file, bytes, cause) in cases {
        let d = Dir::new("invalid");
        d.file("ok.sna", &make_sna(&regs, &ram, 0));
        d.file(file, &bytes);
        let text = game("ok", "Good", "ok.sna", "") + &game("bad", "Bad", file, "");
        let e = build_pack(&d.manifest(&text)).unwrap_err();
        assert!(e.starts_with(&format!("{file}: {cause}")), "{e}");
    }
}

#[test]
fn rejects_unknown_extension_and_missing_file() {
    let d = Dir::new("ext");
    d.file("a.rom", &[0u8; 100]);
    let e = build_pack(&d.manifest(&game("a", "A", "a.rom", ""))).unwrap_err();
    assert!(e.contains("a.rom"), "{e}");
    let e = build_pack(&d.manifest(&game("a", "A", "nope.sna", ""))).unwrap_err();
    assert!(e.contains("nope.sna"), "{e}");
}

#[test]
fn rejects_oversize() {
    let d = Dir::new("big");
    let mut tap = tap_fixture();
    tap.resize(PACK_MAX_RAW as usize + 1, 0);
    d.file("big.tap", &tap);
    let e = build_pack(&d.manifest(&game("big", "Big", "big.tap", ""))).unwrap_err();
    assert!(e.contains("big.tap") && e.contains("131072"), "{e}");
}

#[test]
fn stored_when_lz4_doesnt_help() {
    let d = Dir::new("stored");
    // a valid tap whose data block is incompressible
    let mut x = 0x2545_F491u32;
    let noise: Vec<u8> = (0..3000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect();
    let mut tap = tap_header(3, "noise", noise.len() as u16, 32768, 0);
    tap.extend(tap_block(0xFF, &noise));
    d.file("n.tap", &tap);
    let pack = build_pack(&d.manifest(&game("n", "Noise", "n.tap", ""))).unwrap();
    let g = pack_game_at(&pack, 0).unwrap();
    assert_eq!(g.codec(), PACK_STORED);
    assert_eq!(g.stored_len(), g.raw_len());
    assert_eq!(g.extract(tap.len()).unwrap(), tap);
}

#[test]
fn cli_writes_the_pack() {
    let d = Dir::new("cli");
    d.file("a.tap", &tap_fixture());
    let manifest = d.manifest(&game("a", "A", "a.tap", ""));
    let out = d.0.join("pack.bin");
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_fmzx-pack"))
        .args(["games", manifest.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(std::fs::read(&out).unwrap(), build_pack(Path::new(&manifest)).unwrap());
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_fmzx-pack")).args(["games"]).status().unwrap();
    assert!(!st.success());
}
