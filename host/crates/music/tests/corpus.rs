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
    assert!(csv.lines().all(|l| l.matches(',').count() == 14), "{csv}");
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

#[test]
fn tzx_skips_the_non_data_blocks() {
    let tap = castaway_tap();
    let blocks = tap_blocks(&tap);
    let skipped: [&[u8]; 16] = [
        &[0x20, 1, 0],
        &[0x21, 2, b'h', b'i'],
        &[0x22],
        &[0x23, 5, 0],
        &[0x24, 3, 0],
        &[0x25],
        &[0x26, 2, 0, 1, 0, 2, 0],
        &[0x27],
        &[0x28, 3, 0, 1, 2, 3],
        &[0x2a, 0, 0, 0, 0],
        &[0x2b, 1, 0, 0, 0, 1],
        &[0x30, 2, b'h', b'i'],
        &[0x31, 4, 2, b'h', b'i'],
        &[0x32, 4, 0, 1, 1, 2, 3],
        &[0x33, 2, 0, 0, 1, 0, 1, 2],
        &[0x34, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    let mut t = tzx_header();
    for b in skipped {
        t.extend_from_slice(b);
    }
    t.extend_from_slice(&[0x35, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o', b'p', 2, 0, 0, 0, 9, 9]);
    t.extend_from_slice(&[0x5a, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for b in &blocks {
        t.push(0x10);
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&(b.len() as u16).to_le_bytes());
        t.extend_from_slice(b);
    }
    assert_eq!(to_tap(&t).expect("converts"), tap);
    // each skipped block cut short is a clean refusal
    for b in skipped.iter().copied().chain([&[0x35u8, 1, 2, 3][..], &[0x5a, 0, 0][..]]) {
        for cut in 1..b.len() {
            let mut c = tzx_header();
            c.extend_from_slice(&b[..cut]);
                        assert!(to_tap(&c).is_err(), "block 0x{:02x} cut at {cut}", b[0]);
        }
    }
}

#[test]
fn engines_recognised_by_content() {
    use music::corpus::detect_engine;
    let run = |mut m: zx_sys::Machine| {
        for _ in 0..1500 {
            m.run_frame();
        }
        detect_engine(&m)
    };
    assert_eq!(run(music::castaway()), Some(Engine::Castaway));
    assert_eq!(run(music::colony8()), Some(Engine::Colony8));
    assert_eq!(run(music::test_sna("tune.sna")), None);
}

#[test]
fn engine_not_taken_from_the_name() {
    // Castaway under another name is still Castaway and scored; tune.sna named like Cave Runner
    // is not, and gets no truth
    let dir = temp("names");
    std::fs::copy(repo().join("third_party/games/castaway/castawy.tap"), dir.join("renamed_game.tap")).unwrap();
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("cave_runner_x.sna")).unwrap();
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("cave.tap.sna")).unwrap();
    let out = temp("namesout");
    let rows = run_folder(&dir, &Options { seconds: 20, wav: false, out: out.clone() }).unwrap();
    let r = rows.iter().find(|r| r.file == "renamed_game.tap").unwrap();
    assert_eq!(r.engine, Some(Engine::Castaway));
    assert!(r.truth.is_some());
    for f in ["cave_runner_x.sna", "cave.tap.sna"] {
        let r = rows.iter().find(|r| r.file == f).unwrap();
        assert!(r.engine.is_none() && r.truth.is_none(), "{f}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn wav_failure_keeps_the_numbers() {
    // the output dir exists but `tune_fm.wav` is a directory: the write fails, the row stays
    let dir = temp("wavfail");
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("tune.sna")).unwrap();
    let out = temp("wavfailout");
    std::fs::create_dir_all(out.join("tune_fm.wav")).unwrap();
    let opts = Options { seconds: 5, wav: true, out: out.clone() };
    let rows = run_folder(&dir, &opts).unwrap();
    let r = &rows[0];
    assert_eq!(r.status, "ok");
    assert!(r.note.starts_with("wav not written"), "{}", r.note);
    assert!(r.fm_pct > 50.0);
    let txt = std::fs::read_to_string(out.join("report.txt")).unwrap();
    let csv = std::fs::read_to_string(out.join("report.csv")).unwrap();
    assert!(txt.contains("wav not written") && txt.contains("tune.sna") && !txt.contains("not loadable"), "{txt}");
    let line = csv.lines().nth(1).unwrap();
    assert!(line.starts_with("tune.sna,ok,") && line.contains("wav not written"), "{line}");
    assert!(csv.lines().all(|l| l.matches(',').count() >= 14));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}
