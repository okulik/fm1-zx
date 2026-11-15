// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The corpus tool (`music::corpus`) on the pack's own files copied to a temp folder: TZX
//! conversion, key scripts, report rows and the engine recognition.

use fmzx_pack::testprogs::{rom_copy_payload, rom_copy_tap, tape_tap, turbo_tzx, ROM_COPY_PAYLOAD_AT, TAPE_DONE_BORDER};
use music::corpus::{load, parse_keys, report_csv, report_txt, run_folder, Options, Step};
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
    // (named as the .tap: each keeps its own WAVs)
    std::fs::write(dir.join("castawy.tzx"), &tz).unwrap();
    let mut turbo = tzx_header();
    turbo.extend_from_slice(&[0x12, 0, 0, 0, 0]);
    std::fs::write(dir.join("turbo.tzx"), &turbo).unwrap();
    std::fs::write(dir.join("readme.txt"), "ignored").unwrap();
    let before: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();

    let out = temp("out");
    let opts = Options { seconds: 30, wav: true, out: out.join("report"), learn: true };
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
    let z = by("castawy.tzx");
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
    // fields: the commas outside quotes (a learned line holds commas)
    let fields = |l: &str| l.chars().fold((0, false), |(n, q), ch| (n + usize::from(ch == ',' && !q), q ^ (ch == '"'))).0;
    assert!(csv.lines().all(|l| fields(l) == 27), "{csv}");
    // the learned column: the tune's routine learned and claimed, Castaway's left to its
    // built-in entry; each game's line and details in sound-report.txt, the results cached
    let tune = by("tune.sna");
    assert!(tune.learned_line.starts_with("music learned (1 routine, heard"), "{}", tune.learned_line);
    assert!(tune.learned_frames > 0 && tune.learned_fm_pct > 90.0 && tune.b_fm_pct < 1.0, "{tune:?}");
    assert!(c.learned_line.contains("built-in entry"), "{}", c.learned_line);
    assert_eq!((c.learned_frames, c.learned_fm_pct), (0, c.b_fm_pct));
    assert!(by("turbo.tzx").learned_line.is_empty());
    let sound = std::fs::read_to_string(opts.out.join("sound-report.txt")).unwrap();
    assert!(sound.contains(&format!("tune.sna: {}\n", tune.learned_line)) && sound.contains("OUT at 8014h"), "{sound}");
    assert!(txt.contains("L FM %") && txt.contains(&format!("tune.sna: {}", tune.learned_line)), "{txt}");
    assert!(csv.lines().next().is_some_and(|h| h.contains("learned_fm_percent,learned_frames,learned_line")), "{csv}");
    let cached = std::fs::read_dir(opts.out.join("sound-cache")).unwrap().count();
    assert_eq!(cached, 3, "one cache file per loadable game");
    for f in ["castawy_tap_beeper.wav", "castawy_tap_fm.wav", "castawy_tap_fm_b.wav", "castawy_tzx_beeper.wav", "castawy_tzx_fm.wav", "tune_sna_fm.wav"] {
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
fn corpus_reports_b() {
    // method B beside A: Castaway's and Colony 8's music recognised by B, named, its FM share
    // through the mixer's FM: engines path, scored against the truth with no wrong note
    let dir = temp("b");
    std::fs::copy(repo().join("third_party/games/castaway/castawy.tap"), dir.join("castawy.tap")).unwrap();
    std::fs::copy(repo().join("third_party/games/colony8/colony8.tap"), dir.join("colony8.tap")).unwrap();
    let out = temp("bout");
    let opts = Options { seconds: 30, wav: true, out: out.clone(), learn: false };
    let rows = run_folder(&dir, &opts).expect("runs");
    let txt = report_txt(&rows, &opts);
    println!("{txt}");
    for (file, name, engine) in [("castawy.tap", "Castaway", Engine::Castaway), ("colony8.tap", "Colony 8", Engine::Colony8)] {
        let r = rows.iter().find(|r| r.file == file).unwrap();
        assert_eq!(r.engine, Some(engine), "{file}");
        assert_eq!(r.b_engines, [name], "{file}");
        assert!(r.b_fm_pct > 50.0, "{file}: B's FM {:.1}%", r.b_fm_pct);
        let t = r.b_truth.as_ref().unwrap_or_else(|| panic!("{file}: B scored"));
        assert!(t.wrong.is_empty(), "{file}: B's wrong notes {t}");
        assert!(t.precision() >= 0.95, "{file}: B's precision {t}");
        assert!(std::fs::read(out.join(format!("{}_fm_b.wav", file.replace('.', "_")))).is_ok_and(|w| &w[..4] == b"RIFF"), "{file}");
    }
    assert!(txt.contains("B: Castaway") && txt.contains("B: Colony 8"), "{txt}");
    let csv = std::fs::read_to_string(out.join("report.csv")).unwrap();
    assert!(csv.lines().next().unwrap().contains(",b_fm_percent,b_engines,"), "{csv}");
    assert!(csv.lines().any(|l| l.starts_with("castawy.tap,ok,") && l.contains(",Castaway,")), "{csv}");
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
    let rows = run_folder(&dir, &Options { seconds: 3, wav: false, out: out.clone(), learn: false }).unwrap();
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
    let rows = run_folder(&dir, &Options { seconds: 20, wav: false, out: out.clone(), learn: false }).unwrap();
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
fn unison_truth_digest() {
    // Unison's melody loop joins the truth by its digest, not by the file name: tune.sna named
    // like Unison is not Unison; a test-built image with the PC in the loop and the operands set
    // reads N and N - 1 (no byte of Unison here), and is not recognised either
    use music::corpus::detect_engine;
    use music::truth::Engine::Unison;
    let dir = temp("unison");
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("unison.sna")).unwrap();
    let out = temp("unisonout");
    let rows = run_folder(&dir, &Options { seconds: 5, wav: false, out: out.clone(), learn: false }).unwrap();
    assert!(rows[0].engine.is_none() && rows[0].truth.is_none() && rows[0].b_engines.is_empty(), "{:?}", rows[0]);
    let mut m = music::test_sna("tune.sna");
    assert_eq!(Unison.read(&m), None, "the PC is not in the loop");
    for (a, v) in [(0x81cdu16, 184u16), (0x81df, 183)] {
        m.poke(a, v as u8);
        m.poke(a + 1, (v >> 8) as u8);
    }
    let mut r = m.regs();
    r.pc = 0x81c6;
    m.set_regs(&r);
    // the half-period: 48 T a pass, 47.5 for each reload (its own, and n / n' of the other's)
    let hz = |n: f64, o: f64| 3_500_000.0 / (2.0 * (48.0 * n + 47.5 * (1.0 + n / o)));
    let got = Unison.read_channels(&m).expect("in the loop");
    let want = [(0, hz(184.0, 183.0)), (1, hz(183.0, 184.0))];
    assert!(got.len() == 2 && got.iter().zip(want).all(|(g, w)| g.0 == w.0 && (g.1 / w.1 - 1.0).abs() < 1e-12), "{got:?}");
    assert_eq!(detect_engine(&m), None);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn wav_failure_keeps_the_numbers() {
    // the output dir exists but `tune_sna_fm.wav` is a directory: the write fails, the row stays
    let dir = temp("wavfail");
    std::fs::copy(repo().join("games/test/tune.sna"), dir.join("tune.sna")).unwrap();
    let out = temp("wavfailout");
    std::fs::create_dir_all(out.join("tune_sna_fm.wav")).unwrap();
    let opts = Options { seconds: 5, wav: true, out: out.clone(), learn: false };
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
    assert!(csv.lines().all(|l| l.matches(',').count() >= 20));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn corpus_loads_turbo() {
    // a tape with its own turbo loader runs from the snapshot after its load (border green at
    // frame 0 of the run); a standard tape beside it still runs
    let dir = temp("turbo");
    std::fs::write(dir.join("turbo.tzx"), turbo_tzx()).unwrap();
    std::fs::write(dir.join("tape.tap"), tape_tap()).unwrap();
    let out = temp("turboout");
    let rows = run_folder(&dir, &Options { seconds: 5, wav: false, out: out.clone(), learn: false }).expect("runs");
    for f in ["turbo.tzx", "tape.tap"] {
        let r = rows.iter().find(|r| r.file == f).unwrap_or_else(|| panic!("no row {f}"));
        assert_eq!(r.status, "ok", "{f}");
    }
    let m = load(&dir.join("turbo.tzx")).expect("loads");
    assert_eq!(m.border(), TAPE_DONE_BORDER, "frame 0 of the run is the loaded machine");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn corpus_standard_tape_routes() {
    // a standard tape the trap's trial rejects (its last block read by a loader of its own) runs
    // from the snapshot after its signal load: frames already run, the payload in RAM, no tape
    let dir = temp("routes");
    std::fs::write(dir.join("copy.tap"), rom_copy_tap()).unwrap();
    std::fs::write(dir.join("tape.tap"), tape_tap()).unwrap();
    let m = load(&dir.join("copy.tap")).expect("loads through the signal");
    assert!(m.frame_count() > 0, "time 0 is after the load and its settle time");
    assert!(!m.has_tape_buffer(), "no tape for the trap");
    assert_eq!(m.border(), TAPE_DONE_BORDER);
    let payload = rom_copy_payload();
    let got: Vec<u8> = (0..payload.len()).map(|i| m.peek(ROM_COPY_PAYLOAD_AT + i as u16)).collect();
    assert!(got == payload, "the headerless block is in RAM");
    // the plain standard tape takes the trap: time 0 is the autostart, the tape inserted, and
    // LD-BYTES takes its blocks as the run goes on
    let mut m = load(&dir.join("tape.tap")).expect("loads through the trap");
    assert_eq!(m.frame_count(), 0, "time 0 is the autostart");
    assert!(m.has_tape_buffer(), "the tape is in for the trap");
    assert_eq!(m.tape_pos(), 0);
    for _ in 0..200 {
        m.run_frame();
    }
    assert!(m.tape_pos() > 0, "the trap took the first blocks");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The schedule's hard-coded key positions are the manifest's action numbers.
#[test]
fn schedule_keys_match_parse_action() {
    for (name, row, mask) in soundlearn::listen::PRESSES {
        let a = fmzx_pack::manifest::parse_action(name).expect(name);
        if a & 0x40 != 0 {
            assert_eq!((row, mask), (8, 1 << (a & 0x3f)), "{name}");
        } else {
            assert_eq!((row, mask), (a as usize / 5, 1 << (a % 5)), "{name}");
        }
    }
}

/// The sounding pitches of a decision (period_q8 of its tonal voices at the frame's start), in
/// order.
fn voices(d: &zx_sys::SndFrame) -> Vec<u32> {
    let mut v: Vec<u32> = d.tone.iter().filter(|t| t.level != 0 && t.period_q8 != 0).map(|t| t.period_q8).collect();
    v.sort_unstable();
    v
}

/// The learner with the built-in table disabled, on the repo's two games that have built-in
/// entries (the spec's built-in agreement): each routine it learns plays the pitches the
/// built-in entry plays, within 0.2%, on every frame both claim; Castaway's sliding tune is left
/// out with a reason.
#[test]
fn builtins_agree() {
    use soundlearn::listen::schedule;
    use soundlearn::{learn_with, LearnOptions};
    use zx_sys::SndEngines;
    for (game, machine) in [("castaway", music::castaway as fn() -> zx_sys::Machine), ("colony8", music::colony8)] {
        let got = learn_with(&machine, &schedule(1), &LearnOptions { ignore_builtins: true });
        eprintln!("{game}: {}\n{}", got.line, got.details);
        if got.entries.is_empty() {
            // left out by the code reader: a shape it does not model (not a gate's figures)
            let why = got.line.strip_prefix("no music learned (").unwrap_or_else(|| panic!("{game}: {}", got.line));
            assert!(!["fit", "replay", "code only", "built-in", "learner error", "no speaker"].iter().any(|g| why.starts_with(g)), "{game}: {}", got.line);
            continue;
        }
        let list = soundlearn::encode_list(&got.entries);
        let (mut builtin, mut learned) = (SndEngines::new(), SndEngines::with_learned_only(&list).expect("decodes"));
        let mut m = machine();
        let inputs = schedule(1);
        let (mut both, mut off) = (0, Vec::new());
        for f in 0..3000u32 {
            let (rows, kemp) = inputs(f);
            m.set_keys(rows);
            m.set_kempston(kemp);
            m.run_frame();
            let (edges, level) = (m.edges().to_vec(), m.edges_start_level());
            let (b, l) = (builtin.frame(&m, &edges, level), learned.frame(&m, &edges, level));
            if b.confident == 0 || b.engine == 0 || l.confident == 0 || l.engine == 0 {
                continue;
            }
            both += 1;
            let (vb, vl) = (voices(&b), voices(&l));
            let agree = vb.len() == vl.len() && vb.iter().zip(&vl).all(|(&x, &y)| (f64::from(x) - f64::from(y)).abs() <= 0.002 * f64::from(x));
            if !agree {
                off.push((f, vb, vl));
            }
        }
        assert!(both > 0, "{game}: no frame both claim");
        assert!(off.is_empty(), "{game}: {} of {both} frames differ, first {:?}", off.len(), off.first());
    }
}
