// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The corpus tool (`music::corpus`) on the pack's own files copied to a temp folder: TZX
//! conversion, key scripts, report rows and the engine recognition.

use music::corpus::{parse_keys, report_csv, report_txt, run_folder, Options, Step};
use music::repo;
use music::truth::Engine;
use music::tzx::to_tap;

/// A fresh temp folder.
fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("music-corpus-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn castaway_tap() -> Vec<u8> {
    std::fs::read(repo().join("third_party/games/castaway/castawy.tap")).expect("castawy.tap")
}

/// The blocks of a `.tap` image.
fn tap_blocks(tap: &[u8]) -> Vec<&[u8]> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < tap.len() {
        let n = u16::from_le_bytes([tap[i], tap[i + 1]]) as usize;
        v.push(&tap[i + 2..i + 2 + n]);
        i += 2 + n;
    }
    v
}

fn tzx_header() -> Vec<u8> {
    let mut t = b"ZXTape!\x1a".to_vec();
    t.extend_from_slice(&[1, 20]);
    t
}

#[test]
fn tzx_standard_converts() {
    let tap = castaway_tap();
    let mut t = tzx_header();
    t.extend_from_slice(&[0x30, 5]);
    t.extend_from_slice(b"hello");
    for b in tap_blocks(&tap) {
        t.push(0x10);
        t.extend_from_slice(&1000u16.to_le_bytes());
        t.extend_from_slice(&(b.len() as u16).to_le_bytes());
        t.extend_from_slice(b);
        t.extend_from_slice(&[0x20, 0x10, 0x00]); // a pause between blocks
    }
    t.extend_from_slice(&[0x32, 3, 0, 9, 9, 9]); // archive info
    assert_eq!(to_tap(&t).expect("converts"), tap);
}

#[test]
fn tzx_turbo_rejected() {
    let mut t = tzx_header();
    t.push(0x11);
    t.extend_from_slice(&[0; 0x12]);
    let e = to_tap(&t).unwrap_err();
    assert!(e.contains("0x11"), "{e}");
    // truncated and foreign files are refused, not panicked on
    assert!(to_tap(b"nope").is_err());
    let mut cut = tzx_header();
    cut.extend_from_slice(&[0x10, 0, 0, 0xff, 0xff, 1, 2]);
    assert!(to_tap(&cut).is_err());
}

#[test]
fn keys_parse() {
    let s = parse_keys("# title\nkey.ENTER@8000, key.SPACE@9000+200\nkempston.fire@100").unwrap();
    assert_eq!(s.len(), 3);
    assert_eq!(s[1], Step { action: 35, at: 9000, hold: 200 });
    assert_eq!(s[0].hold, 100);
    assert!(parse_keys("key.NOPE@1").is_err());
    assert!(parse_keys("key.ENTER").is_err());
}

#[test]
fn corpus_runs_on_pack() {
    let dir = temp("pack");
    std::fs::copy(repo().join("third_party/games/castaway/castawy.tap"), dir.join("castawy.tap")).unwrap();
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("tune.sna")).unwrap();
    // a Castaway as a .tzx, a turbo tape and a stray file: the first runs, the second is reported
    let mut tz = tzx_header();
    for b in tap_blocks(&castaway_tap()) {
        tz.push(0x10);
        tz.extend_from_slice(&[0, 0]);
        tz.extend_from_slice(&(b.len() as u16).to_le_bytes());
        tz.extend_from_slice(b);
    }
    std::fs::write(dir.join("castaway_tzx.tzx"), &tz).unwrap();
    let mut turbo = tzx_header();
    turbo.extend_from_slice(&[0x12, 0, 0, 0, 0]);
    std::fs::write(dir.join("turbo.tzx"), &turbo).unwrap();
    std::fs::write(dir.join("readme.txt"), "ignored").unwrap();
    let before: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();

    let out = temp("out");
    let opts = Options { seconds: 30, wav: true, out: out.join("report") };
    let rows = run_folder(&dir, &opts).expect("runs");
    assert_eq!(rows.len(), 4, "readme.txt is not a game");
    let by = |f: &str| rows.iter().find(|r| r.file == f).unwrap_or_else(|| panic!("no row {f}"));

    let c = by("castawy.tap");
    assert_eq!(c.status, "ok");
    assert_eq!(c.engine, Some(Engine::Castaway));
    let t = c.truth.as_ref().expect("scored");
    println!("{}", report_txt(&rows, &opts));
    assert!(c.fm_pct > 50.0, "FM {:.1}%", c.fm_pct);
    assert!(t.precision() >= 0.9, "precision {:.3}", t.precision());
    let z = by("castaway_tzx.tzx");
    assert_eq!(z.status, "ok");
    assert!((z.fm_pct - c.fm_pct).abs() < 1.0, "the .tzx plays as the .tap");
    assert!(by("tune.sna").status == "ok" && by("tune.sna").sound_frames > 0);
    assert!(by("turbo.tzx").status.starts_with("not loadable"), "{}", by("turbo.tzx").status);

    // reports and WAVs under the output dir only; the input folder is untouched
    let txt = std::fs::read_to_string(opts.out.join("report.txt")).unwrap();
    let csv = std::fs::read_to_string(opts.out.join("report.csv")).unwrap();
    assert_eq!(txt, report_txt(&rows, &opts));
    assert_eq!(csv, report_csv(&rows));
    assert_eq!(csv.lines().count(), 5);
    assert!(csv.lines().all(|l| l.matches(',').count() == 13), "{csv}");
    for f in ["castawy_beeper.wav", "castawy_fm.wav", "tune_fm.wav"] {
        let w = std::fs::read(opts.out.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(&w[..4], b"RIFF");
        assert!(w.len() > 44 + 30 * 44_000);
    }
    let after: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before.len(), after.len(), "nothing written beside the games");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn keys_file_applies() {
    // tune.sna with a key script runs the same sound; a bad script is reported, not fatal
    let dir = temp("keys");
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("tune.sna")).unwrap();
    std::fs::write(dir.join("tune.keys"), "key.SPACE@500+200").unwrap();
    std::fs::copy(repo().join("games/test/border.sna"), dir.join("border.sna")).unwrap();
    std::fs::write(dir.join("border.keys"), "key.BOGUS@1").unwrap();
    let out = temp("keysout");
    let rows = run_folder(&dir, &Options { seconds: 3, wav: false, out: out.clone() }).unwrap();
    assert_eq!(rows.iter().find(|r| r.file == "tune.sna").unwrap().status, "ok");
    assert!(rows.iter().find(|r| r.file == "border.sna").unwrap().status.contains("key script"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}
