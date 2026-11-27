// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `music-preview` on a personal pack's learning workspace made by `fmzx-pack personal --plan`
//! from `games/test/tune.sna` (a tune its learned entry plays, so it has a shadow score): the
//! game's music for 40 s; its shadow score from its start with the live version, an
//! AI take, a C64 take and a Spectrum 128 take, each heard differently, for the score and 2 s; the beeper over the
//! same frames; `--seconds`; and the errors: no workspace, an unknown game, score or take (exit 1)
//! and a take the pack refuses (exit 3, `refused: <why>`). Every folder holds only generated
//! test programs.

use fmzx_pack::arrange::{timeline, CLOCK_HZ};
use fmzx_pack::personal::{cache_decode, run_plan};
use music::preview::{preview, rate, Error, Request, Take};
use soundlearn::{score_key, ScoreRecord};
use std::path::{Path, PathBuf};
use std::process::Command;
use zx_sys::ZX_FRAME_TSTATES;

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("music-preview-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// An AI take of `rec` (`take-1`): its notes an octave down; `key` the key it says it is of.
fn octave_down(rec: &ScoreRecord, key: &str) -> String {
    let mut t = format!("key = \"{key}\"\nintro_t = {}\nloop_t = {}\ntake = 1\ngenerator = \"octave down\"\n", rec.intro_t, rec.loop_t);
    for n in timeline(rec) {
        t += &format!("\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = 200\n", n.voice, n.start_t, n.end_t, 2 * n.period_t);
    }
    t
}

/// An engine take of `rec` (`engine-1`) on the score's timeline: its notes a fifth down.
fn engine(rec: &ScoreRecord, key: &str) -> String {
    let mut t = format!("key = \"{key}\"\nintro_t = {}\nloop_t = {}\ntake = \"engine-1\"\ngenerator = \"engine\"\nsource = \"ntropic: an invented reading\"\n", rec.intro_t, rec.loop_t);
    for n in timeline(rec) {
        t += &format!("\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = 250\n", n.voice, n.start_t, n.end_t, n.period_t * 3 / 2);
    }
    t
}

/// A C64 take of `rec` (`chip-1`) on its own timeline: two notes a fifth apart, looped.
fn chip(rec: &ScoreRecord) -> String {
    let f = u64::from(ZX_FRAME_TSTATES);
    format!(
        "key = \"{}\"\nintro_t = {}\nloop_t = {}\ntake = \"chip-1\"\ngenerator = \"chip sid\"\ntimeline = \"own\"\n\
         own_intro_t = {}\nown_loop_t = {}\nsource = \"C64: an invented tune\"\n\n\
         [[note]]\nvoice = 0\nstart_t = 0\nend_t = {}\nperiod_t = 5000\nlevel = 220\n\n\
         [[note]]\nvoice = 0\nstart_t = {}\nend_t = {}\nperiod_t = 3337\nlevel = 220\n",
        score_key(rec),
        rec.intro_t,
        rec.loop_t,
        10 * f,
        40 * f,
        10 * f,
        10 * f,
        50 * f
    )
}

/// A Spectrum 128 take of `rec` (`ay-1`): [`chip`]'s, a fourth apart.
fn ay(rec: &ScoreRecord) -> String {
    chip(rec)
        .replace("take = \"chip-1\"", "take = \"ay-1\"")
        .replace("generator = \"chip sid\"", "generator = \"chip ay\"")
        .replace("C64: an invented tune", "Spectrum 128: an invented tune")
        .replace("period_t = 3337", "period_t = 3746")
}

/// The samples of a WAV that `music::wav` wrote, and its rate.
fn wav(path: &Path) -> (Vec<i16>, u32) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!((&b[..4], &b[8..16]), (&b"RIFF"[..], &b"WAVEfmt "[..]));
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    assert_eq!(u16::from_le_bytes([b[34], b[35]]), 16, "16-bit");
    (b[44..].as_chunks::<2>().0.iter().map(|&c| i16::from_le_bytes(c)).collect(), rate)
}

/// Runs the binary; its exit code, stdout and stderr.
fn cli(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_music-preview")).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn loud(s: &[i16]) -> bool {
    s.iter().filter(|&&v| v.unsigned_abs() > 500).count() > s.len() / 20
}

#[test]
fn previews_of_a_workspace() {
    let games = Tmp::new("games");
    std::fs::write(games.0.join("tune.sna"), std::fs::read(music::repo().join("games/test/tune.sna")).unwrap()).unwrap();
    let w = Tmp::new("w");
    let all = w.0.join("all");
    run_plan(&games.0, &all, None).expect("plans");
    let cached = std::fs::read_dir(all.join("sound-cache")).unwrap().next().expect("a cache file").unwrap().path();
    let (_, _, _, details, shadows) = cache_decode(&std::fs::read(&cached).unwrap()).expect("the cache file reads");
    assert_eq!(shadows.len(), 1);
    let rec = shadows[0].clone();
    let key = score_key(&rec);
    let folder = all.join("arrange/tune").join(&key);
    write(&folder.join("take-1/arranged.toml"), &octave_down(&rec, &key));
    write(&folder.join("chip-1/arranged.toml"), &chip(&rec));
    write(&folder.join("ay-1/arranged.toml"), &ay(&rec));
    write(&folder.join("engine-1/arranged.toml"), &engine(&rec, &key));
    write(&folder.join("take-2/arranged.toml"), &octave_down(&rec, "0000-00000000").replace("take = 1", "take = 2"));
    let rate = rate();
    let samples = |s: f64| (s * f64::from(rate)).round() as usize;
    let frame_s = f64::from(ZX_FRAME_TSTATES) / f64::from(CLOCK_HZ);

    // the game's music: the learner's run from its start, 40 s
    let out = w.0.join("previews");
    let (code, said, err) = cli(&[all.to_str().unwrap(), "--game", "tune", "-o", out.join("tune.wav").to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(said.contains("tune.wav: 40.0 s of tune (its music, FM: engines), from 0.0 s into its run"), "{said}");
    let (music, r) = wav(&out.join("tune.wav"));
    assert_eq!((r, music.len()), (rate, samples(40.0)));
    assert!(loud(&music), "the tune plays");

    // the shadow score: from the frame the learner says it starts in, for its length and 2 s; the live version, each take and the beeper heard apart
    let start = details.lines().find(|l| l.contains(&format!("(score {key}"))).expect("its line");
    let frame: usize = start.split(" from frame ").nth(1).unwrap().split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap();
    let len = (f64::from(rec.intro_t) + f64::from(rec.loop_t)) / f64::from(CLOCK_HZ) + 2.0;
    let len = len.min(40.0);
    let req = |take: Take| Request { game: "tune".into(), score: Some((key.clone(), take)), seconds: None, beeper: false };
    let live = preview(&all, &req(Take::Original)).expect("the live version");
    assert_eq!(live.plays.as_deref(), Some("the live version"));
    assert!(live.cached, "the plan cached the game's music");
    assert!((live.from - frame as f64 * frame_s).abs() < 1e-9, "{} for frame {frame}", live.from);
    let mut heard = vec![live.samples.clone()];
    for (take, plays) in [
        (Take::Of(fmzx_pack::arrange::Choice::Take(1)), "take 1 (octave down)"),
        (Take::Of(fmzx_pack::arrange::Choice::Chip(1)), "C64 take chip-1 (C64: an invented tune)"),
        (Take::parse("ay-1").expect("ay-1 names a take"), "Spectrum 128 take ay-1 (Spectrum 128: an invented tune)"),
        (Take::parse("engine-1").expect("engine-1 names a take"), "engine take engine-1 (ntropic: an invented reading)"),
    ] {
        let p = preview(&all, &req(take)).unwrap_or_else(|e| panic!("{take:?}: {e}"));
        assert_eq!((p.plays.as_deref(), p.from), (Some(plays), live.from));
        heard.push(p.samples);
    }
    let b = preview(&all, &Request { beeper: true, ..req(Take::Original) }).expect("the beeper");
    heard.push(b.samples);
    for (k, s) in heard.iter().enumerate() {
        assert_eq!(s.len(), samples(len), "render {k}");
        assert!(loud(s), "render {k} plays");
        for (j, t) in heard.iter().enumerate().take(k) {
            assert_ne!(s, t, "renders {j} and {k} differ");
        }
    }

    // the same through the command line, take 0 and take 1, and the beeper; --seconds
    let path = |n: &str| out.join(n).to_str().unwrap().to_string();
    for (take, name) in [("0", "t0.wav"), ("original", "to.wav"), ("1", "t1.wav")] {
        let (code, said, err) = cli(&[all.to_str().unwrap(), "--game", "tune", "--score", &key, "--take", take, "-o", &path(name)]);
        assert_eq!(code, 0, "{err}");
        assert!(said.contains(&format!("score {key} playing ")), "{said}");
    }
    assert_eq!(wav(&out.join("t0.wav")).0, live.samples);
    assert_eq!(wav(&out.join("to.wav")).0, live.samples);
    assert_eq!(wav(&out.join("t1.wav")).0, heard[1]);
    let (code, said, err) = cli(&[all.to_str().unwrap(), "--game", "tune", "--score", &key, "--take", "1", "--beeper", "--seconds", "5", "-o", &path("b.wav")]);
    assert_eq!(code, 0, "{err}");
    assert!(said.contains(", the beeper), from "), "{said}");
    let (bp, _) = wav(&out.join("b.wav"));
    assert_eq!(bp.len(), samples(5.0));
    assert_eq!(bp[..], heard[5][..bp.len()], "the beeper whatever the take");
    let (code, said, err) = cli(&[all.to_str().unwrap(), "--game", "tune", "--score", &key, "--take", "ay-1", "-o", &path("ay.wav")]);
    assert_eq!(code, 0, "{err}");
    assert!(said.contains(&format!("score {key} playing Spectrum 128 take ay-1")), "{said}");
    assert_eq!(wav(&out.join("ay.wav")).0, heard[3]);

    // the errors: one line on stderr, exit 1; a take the pack refuses, exit 3
    let a = all.to_str().unwrap();
    let o = path("x.wav");
    let nowhere = w.0.join("nowhere");
    for (args, says) in [
        (vec![nowhere.to_str().unwrap(), "--game", "tune", "-o", &o], "no such folder".to_string()),
        (vec![a, "--game", "other", "-o", &o], "no game \"other\" in ".to_string()),
        (vec![a, "--game", "tune", "--score", "0000-00000000", "--take", "1", "-o", &o], format!("game \"tune\" has no score 0000-00000000 (its scores: {key})")),
        (vec![a, "--game", "tune", "--score", &key, "--take", "5", "-o", &o], format!("score {key} has no take take-5 (its takes: ay-1, chip-1, engine-1, take-1, take-2)")),
        (vec![a, "--game", "tune", "--score", &key, "--take", "chip-2", "-o", &o], format!("score {key} has no take chip-2 (")),
        (vec![a, "--game", "tune", "--score", &key, "--take", "ay-2", "-o", &o], format!("score {key} has no take ay-2 (")),
        (vec![a, "--game", "tune", "--score", &key, "--take", "ay-0", "-o", &o], "unknown take \"ay-0\"".to_string()),
    ] {
        let (code, _, err) = cli(&args);
        assert_eq!((code, err.lines().count()), (1, 1), "{args:?}: {err}");
        assert!(err.contains(&says), "{args:?}: {err}");
    }
    let (code, _, err) = cli(&[a, "--game", "tune", "--score", &key, "--take", "2", "-o", &o]);
    assert_eq!((code, err.trim_end()), (3, format!("refused: its key is 0000-00000000, not the score's {key}").as_str()));
    assert!(matches!(preview(&all, &req(Take::Of(fmzx_pack::arrange::Choice::Take(2)))), Err(Error::Refused(_))));
    // usage errors: exit 2
    for args in [vec![a, "--game", "tune"], vec![a, "--game", "tune", "--take", "1", "-o", &o]] {
        assert_eq!(cli(&args).0, 2, "{args:?}");
    }
    assert!(!out.join("x.wav").exists());
}
