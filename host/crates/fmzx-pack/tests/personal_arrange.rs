// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Arranged music in the personal pack: a generated looping tune that nothing reads gets a
//! recorded score, which the run writes to `arrange/<game>/<key>/score.toml`; with no choices
//! the pack is as learned (takes but no choice say `arranged: original`); a take chosen in
//! `choices.toml` (four voices, drums among them) goes into the pack in place of the score's
//! notes and the replay verifies it; takes that do not fit (the wrong key, overlapping notes, a
//! note past the score's end) leave the original with the reason; a choice naming no score is a
//! note, and a choices file that does not read fails the run. The music of a routine the learner
//! learned (`games/test/tune.sna`) is a shadow score: written out with `shadow = true`, packed
//! only once a take is chosen for it, and then with precedence, so that the replay verifies it
//! over the learned entry. Every folder holds only generated test programs.

use fmzx_pack::arrange::{score_toml, timeline, Of};
use fmzx_pack::pack::sound_list;
use fmzx_pack::personal::{cache_decode, run_with_limit, table, LearnerRun, Report, ScoreRuns};
use soundlearn::listen::{schedule, RunKind};
use soundlearn::{decode_scores, score_key, ScoreEvent, ScoreRecord, LEARNER_VERSION, SCORE_DRUM};
use std::path::{Path, PathBuf};
use zx_sys::{Machine, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD};

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fmzx-personal-arrange-{tag}-{}", std::process::id()));
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

/// Code from 8000h with its labels resolved: `(op, label)` pieces are an opcode and a label's
/// address.
#[derive(Default)]
struct Asm {
    code: Vec<u8>,
    labels: std::collections::HashMap<&'static str, u16>,
    fixups: Vec<(usize, &'static str, bool)>,
}

impl Asm {
    fn pc(&self) -> u16 {
        0x8000 + self.code.len() as u16
    }
    fn label(&mut self, name: &'static str) -> &mut Self {
        let pc = self.pc();
        self.labels.insert(name, pc);
        self
    }
    fn b(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }
    /// An opcode and a label's address.
    fn w(&mut self, op: &[u8], name: &'static str) -> &mut Self {
        self.code.extend_from_slice(op);
        self.fixups.push((self.code.len(), name, false));
        self.code.extend_from_slice(&[0, 0]);
        self
    }
    /// An opcode and a label's displacement from the next instruction.
    fn r(&mut self, op: u8, name: &'static str) -> &mut Self {
        self.code.push(op);
        self.fixups.push((self.code.len(), name, true));
        self.code.push(0);
        self
    }
    fn finish(mut self) -> Vec<u8> {
        for &(off, name, rel) in &self.fixups {
            let target = self.labels[name];
            if rel {
                let d = i32::from(target) - (0x8000 + off as i32 + 1);
                self.code[off] = i8::try_from(d).expect("JR in reach") as u8;
            } else {
                self.code[off..off + 2].copy_from_slice(&target.to_le_bytes());
            }
        }
        self.code
    }
}

/// The tune's intro and the phrase it loops: each note the two channels' steps and its frames.
const INTRO: [(u16, u16, u8); 4] = [(265, 662, 40), (334, 835, 30), (445, 1112, 50), (1, 1, 20)];
const PHRASE: [(u16, u16, u8); 6] = [(297, 891, 40), (354, 1062, 30), (396, 990, 40), (499, 1497, 30), (315, 788, 50), (600, 1500, 40)];

/// A tune of two phase accumulators mixed into the speaker bit, its register sets swapped
/// between its two `OUT`s (so that neither the code reader nor the observation gives it: it is
/// recorded as a score), paced by the frame interrupt (IM 2, an `EI ; RET` at 9191h): about 70
/// frames of silence, then the intro and the phrase, looped. Each frame of a note is 512 passes
/// of its loop after the interrupt.
///
/// ```text
/// start: DI ; LD A,90h ; LD I,A ; IM 2 ; EI ; LD H,70
/// quiet: HALT ; DEC H ; JR NZ,quiet
///        LD IX,intro ; CALL play
/// again: LD IX,phrase ; CALL play ; JR again
/// play:  LD E,(IX+0) ; LD D,(IX+1) ; LD A,D ; OR E ; RET Z
///        EXX ; LD E,(IX+2) ; LD D,(IX+3) ; LD HL,0 ; EXX ; LD HL,0
///        LD A,(IX+4) ; LD (dur),A ; INC IX x5
/// frame: HALT ; LD A,2 ; LD (reps),A
/// loop:  ADD HL,DE ; SBC A,A ; AND 16 ; LD B,A ; EXX ; ADD HL,DE ; SBC A,A ; AND 16 ; EXX ; OR B
/// out1:  OUT (254),A ; XOR A ; EXX ; NOP
/// out2:  OUT (254),A ; EXX ; DEC C ; JP NZ,loop
///        LD A,(reps) ; DEC A ; LD (reps),A ; JP NZ,loop
///        LD A,(dur) ; DEC A ; LD (dur),A ; JP NZ,frame ; JP play
/// ```
fn looping_sna() -> Vec<u8> {
    let mut a = Asm::default();
    a.label("start").b(&[0xF3, 0x3E, 0x90, 0xED, 0x47, 0xED, 0x5E, 0xFB, 0x26, 70]);
    a.label("quiet").b(&[0x76, 0x25]).r(0x20, "quiet");
    a.w(&[0xDD, 0x21], "intro").w(&[0xCD], "play");
    a.label("again").w(&[0xDD, 0x21], "phrase").w(&[0xCD], "play").r(0x18, "again");
    a.label("play").b(&[0xDD, 0x5E, 0x00, 0xDD, 0x56, 0x01, 0x7A, 0xB3, 0xC8]);
    a.b(&[0xD9, 0xDD, 0x5E, 0x02, 0xDD, 0x56, 0x03, 0x21, 0, 0, 0xD9, 0x21, 0, 0]);
    a.b(&[0xDD, 0x7E, 0x04]).w(&[0x32], "dur").b(&[0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23]);
    a.label("frame").b(&[0x76, 0x3E, 0x02]).w(&[0x32], "reps");
    a.label("loop").b(&[0x19, 0x9F, 0xE6, 0x10, 0x47, 0xD9, 0x19, 0x9F, 0xE6, 0x10, 0xD9, 0xB0]);
    a.label("out1").b(&[0xD3, 0xFE, 0xAF, 0xD9, 0x00]);
    a.label("out2").b(&[0xD3, 0xFE, 0xD9, 0x0D]).w(&[0xC2], "loop");
    a.w(&[0x3A], "reps").b(&[0x3D]).w(&[0x32], "reps").w(&[0xC2], "loop");
    a.w(&[0x3A], "dur").b(&[0x3D]).w(&[0x32], "dur").w(&[0xC2], "frame").w(&[0xC3], "play");
    a.label("dur").b(&[0]);
    a.label("reps").b(&[0]);
    for (name, notes) in [("intro", &INTRO[..]), ("phrase", &PHRASE[..])] {
        a.label(name);
        for &(s0, s1, len) in notes {
            a.b(&s0.to_le_bytes()).b(&s1.to_le_bytes()).b(&[len]);
        }
        a.b(&[0, 0]);
    }
    // the interrupt's table at 9000h (257 bytes of 91h) and its routine at 9191h
    let pad = 0x9000 - usize::from(a.pc());
    a.b(&vec![0; pad]).b(&[0x91; 257]);
    let pad = 0x9191 - usize::from(a.pc());
    a.b(&vec![0; pad]).b(&[0xFB, 0xC9]);
    let code = a.finish();
    // a 48K .sna: RAM from 4000h, the start address on the stack at FF00h, interrupts off
    let mut ram = vec![0u8; 0xC000];
    ram[0x4000..0x4000 + code.len()].copy_from_slice(&code);
    ram[0xBF00..0xBF02].copy_from_slice(&0x8000u16.to_le_bytes());
    let mut h = vec![0x3Fu8];
    h.extend_from_slice(&[0; 22]);
    h.extend_from_slice(&0xFF00u16.to_le_bytes());
    h.extend_from_slice(&[1, 0]);
    h.extend_from_slice(&ram);
    h
}

fn run(games: &Path, out: &Path) -> Report {
    run_with_limit(games, out, None).expect("builds")
}

/// The game's list in the pack.
fn list(out: &Path) -> Vec<u8> {
    let pack = std::fs::read(out.join("games.bin")).expect("games.bin written");
    sound_list(&pack, 0).expect("the game's list").to_vec()
}

fn report(out: &Path) -> String {
    std::fs::read_to_string(out.join("sound-report.txt")).expect("sound-report.txt")
}

/// An event at a time in 64-T units of its section.
fn ev(u: u32, voice: u8, on: Option<(u32, u8)>) -> ScoreEvent {
    ScoreEvent { t: u * 64, voice, on }
}

/// A take of the score: four voices, the drum's first off shortened (the next hit comes where
/// it ends), a lead note across the intro's end (restated at the loop's start), and the events
/// it gives (intro, loop).
fn take(rec: &ScoreRecord, n: u32) -> (String, Vec<ScoreEvent>, Vec<ScoreEvent>) {
    let (i, f) = (u64::from(rec.intro_t), 1092u64);
    let at = |u: u64| u * 64;
    // (voice, start, end, period, level), times in T
    let notes: Vec<(u8, u64, u64, u32, u8)> = vec![
        (0, 0, at(2 * f), 4000, 200),
        (0, at(2 * f), at(4 * f), 3000, 200),
        (0, i - 3200, i + at(100), 5000, 120),
        (1, at(1100), at(5000), 20000, 100),
        (1, i + at(2000), i + at(3000), 18000, 100),
        (2, at(1200), at(2300), 8000, 80),
        (2, i + at(50), i + at(150), 7000, 80),
        (3, 0, at(5), 0, 200),
        (3, at(5), at(30), 0, 160),
        (3, i, i + at(3), 0, 252),
        (3, i + at(5000), i + at(5010), 0, 100),
    ];
    let mut text = format!(
        "game = \"looper\"\nname = \"looper\"\nkey = \"{}\"\nclock_hz = 3500000\nintro_t = {}\nloop_t = {}\nvoices = 4\n\
         roles = [\"lead\", \"bass\", \"pad\", \"drums\"]\ntake = {n}\ngenerator = \"test generator\"\n",
        score_key(rec),
        rec.intro_t,
        rec.loop_t
    );
    for (v, s, e, p, l) in notes {
        text += &format!("\n[[note]]\nvoice = {v}\nstart_t = {s}\nend_t = {e}\nperiod_t = {p}\nlevel = {l}\n");
    }
    let c = ((i - 3200 + 32) / 64) as u32;
    let d = SCORE_DRUM;
    let intro = vec![
        ev(0, 0, Some((4000, 200))),
        ev(0, d, Some((0, 200))),
        ev(4, d, None),
        ev(5, d, Some((0, 160))),
        ev(30, d, None),
        ev(1100, 1, Some((20000, 100))),
        ev(1200, 2, Some((8000, 80))),
        ev(2184, 0, Some((3000, 200))),
        ev(2300, 2, None),
        ev(4368, 0, None),
        ev(5000, 1, None),
        ev(c, 0, Some((5000, 120))),
    ];
    let looped = vec![
        ev(0, 0, Some((5000, 120))),
        ev(0, d, Some((0, 252))),
        ev(3, d, None),
        ev(50, 2, Some((7000, 80))),
        ev(100, 0, None),
        ev(150, 2, None),
        ev(2000, 1, Some((18000, 100))),
        ev(3000, 1, None),
        ev(5000, d, Some((0, 100))),
        ev(5010, d, None),
    ];
    (text, intro, looped)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn chosen_takes_go_into_the_pack() {
    let games = Tmp::new("games");
    std::fs::write(games.0.join("looper.sna"), looping_sna()).unwrap();
    let out = Tmp::new("out");
    let r = run(&games.0, &out.0);
    let shown = table(&r);
    println!("{shown}\n{}", report(&out.0));

    // the score: recorded with its loop
    let original = list(&out.0);
    let scores = decode_scores(&original).expect("the scores read back");
    assert_eq!(scores.len(), 1, "{}", report(&out.0));
    let rec = scores[0].clone();
    assert!(rec.loop_t > 0 && rec.loop_t.is_multiple_of(64), "{rec:?}");
    let key = score_key(&rec);
    assert!(report(&out.0).contains(&format!(" KB (score {key})\n")), "{}", report(&out.0));

    // the cache keeps the run that recorded it: the learner's, the generic run seeded by the
    // file's CRC-32 (the tune is the same whatever is pressed)
    let cached = std::fs::read_dir(out.0.join("sound-cache")).unwrap().next().expect("a cache file").unwrap().path();
    assert!(cached.to_string_lossy().ends_with(&format!("-{LEARNER_VERSION}.bin")));
    let (list_c, runs, ..) = cache_decode(&std::fs::read(&cached).unwrap()).expect("the cache file reads");
    assert_eq!(list_c.as_deref(), Some(&original[..]));
    let seed = u64::from(crc32fast::hash(&looping_sna()));
    assert_eq!(runs, Some(ScoreRuns { learner: LearnerRun::Kind(RunKind::Generic, seed), scores: vec![None], shadows: vec![] }));

    // written out: the record's notes on one timeline
    let folder = out.0.join("arrange/looper").join(&key);
    let path = folder.join("score.toml");
    let text = std::fs::read_to_string(&path).expect("score.toml written");
    assert_eq!(text, score_toml("looper", "looper", &rec));
    let t: toml::Table = toml::from_str(&text).expect("score.toml reads");
    assert_eq!(t["game"].as_str(), Some("looper"));
    assert_eq!(t["key"].as_str(), Some(key.as_str()));
    assert_eq!(t["clock_hz"].as_integer(), Some(3_500_000));
    assert_eq!((t["intro_t"].as_integer(), t["loop_t"].as_integer()), (Some(i64::from(rec.intro_t)), Some(i64::from(rec.loop_t))));
    assert_eq!(t["voices"].as_integer(), Some(i64::from(rec.voices)));
    let roles: Vec<&str> = t["roles"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    assert_eq!(roles.len(), usize::from(rec.voices));
    let notes = t["note"].as_array().expect("notes");
    let want = timeline(&rec);
    assert_eq!(notes.len(), want.len());
    let ons = rec.intro_events.iter().chain(&rec.loop_events).filter(|e| e.on.is_some()).count();
    assert_eq!(want.len(), ons, "a note per note on");
    for (n, w) in notes.iter().zip(&want) {
        let get = |k: &str| n[k].as_integer().unwrap();
        assert_eq!(
            (get("voice"), get("start_t"), get("end_t"), get("period_t"), get("level")),
            (i64::from(w.voice), w.start_t as i64, w.end_t as i64, i64::from(w.period_t), i64::from(w.level))
        );
        assert!(w.end_t <= u64::from(rec.intro_t + rec.loop_t) && w.start_t < w.end_t);
    }
    // the loop's notes at intro_t + their time
    let first_loop = rec.loop_events.first().expect("loop events");
    assert!(want.iter().any(|n| n.start_t == u64::from(rec.intro_t + first_loop.t)));
    // not rewritten when unchanged; the folder's other files left alone
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    write(&folder.join("original.wav"), "x");
    std::thread::sleep(std::time::Duration::from_millis(20));

    // no choices: the pack as learned, and nothing said
    run(&games.0, &out.0);
    assert_eq!(list(&out.0), original);
    assert!(!report(&out.0).contains("      arranged:"), "{}", report(&out.0));
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp, "score.toml rewritten");
    assert!(folder.join("original.wav").is_file());

    // takes and no choice: the original plays, and the report says so
    let (good, intro, looped) = take(&rec, 1);
    write(&folder.join("take-1/arranged.toml"), &good);
    let r = run(&games.0, &out.0);
    assert_eq!(list(&out.0), original);
    let rep = report(&out.0);
    assert!(rep.contains("\n      arranged: original\n"), "{rep}");
    assert!(r.arranged.is_empty() && !table(&r).contains(", 1 arranged"), "{}", table(&r));

    // take 1 chosen: in the pack in place of the score's notes, the trigger and fingerprints kept
    let choices = out.0.join("arrange/choices.toml");
    write(&choices, &format!("[choices]\n\"looper/{key}\" = 1\n"));
    let r = run(&games.0, &out.0);
    let shown = table(&r);
    let rep = report(&out.0);
    println!("{shown}\n{rep}");
    let arranged = list(&out.0);
    let got = decode_scores(&arranged).unwrap();
    assert_eq!(got.len(), 1);
    let a = &got[0];
    assert_eq!((a.intro_events.clone(), a.loop_events.clone()), (intro, looped));
    assert_eq!((a.voices, a.roles), (3, [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_PAD]));
    assert_eq!(
        (a.site, a.quiet, a.code, a.intro_t, a.loop_t, &a.intro_fp, &a.loop_fp),
        (rec.site, rec.quiet, rec.code, rec.intro_t, rec.loop_t, &rec.intro_fp, &rec.loop_fp)
    );
    assert_eq!(score_key(a), key);
    assert!(rep.contains("\n      arranged: take 1 (test generator)\n"), "{rep}");
    assert!(shown.contains("looper: music recorded (1 score, ") && shown.contains(", 1 arranged)\n"), "{shown}");
    assert!(shown.contains(&format!("looper: score {key} plays take 1 (test generator)\n")), "{shown}");
    // the replay verifies it in the learner's run (the generic run, seeded by the file's CRC)
    let sna = looping_sna();
    let load = || {
        let mut m = Machine::new();
        m.load_sna(&sna).unwrap();
        m
    };
    soundlearn::scores::verify(&load, &schedule(u64::from(crc32fast::hash(&sna))), &arranged, 0, true).expect("verified");
    // the score file stays the learned score's
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

    // takes that do not fit: the original, with the reason
    let i = rec.intro_t;
    let bad = [
        (good.replace(&key, "0000-00000000"), format!("its key is 0000-00000000, not the score's {key}")),
        (good.clone() + "\n[[note]]\nvoice = 1\nstart_t = 100000\nend_t = 100100\nperiod_t = 9000\nlevel = 50\n", "notes 4 and 12 overlap on voice 1".into()),
        (
            good.clone() + &format!("\n[[note]]\nvoice = 2\nstart_t = {i}\nend_t = {}\nperiod_t = 9000\nlevel = 50\n", i + rec.loop_t + 1),
            format!("note 12: it ends at {} T, past the score's end at {} T", i + rec.loop_t + 1, i + rec.loop_t),
        ),
    ];
    for (n, (text, why)) in bad.iter().enumerate() {
        let n = n + 2;
        write(&folder.join(format!("take-{n}/arranged.toml")), &text.replace("take = 1", &format!("take = {n}")));
        write(&choices, &format!("[choices]\n\"looper/{key}\" = {n}\n"));
        let r = run(&games.0, &out.0);
        assert_eq!(list(&out.0), original, "take {n}");
        let rep = report(&out.0);
        assert!(rep.contains(&format!("\n      arranged: take {n} refused: {why}\n")), "{rep}");
        assert!(table(&r).contains(&format!("looper: score {key}: take {n} refused, the original plays: {why}\n")), "{}", table(&r));
        assert!(!table(&r).contains(", 1 arranged"));
    }

    // a choice that names no score: a note; the take chosen still plays
    write(&choices, &format!("[choices]\n\"looper/{key}\" = 1\n\"looper/0000-00000000\" = 2\n\"gone/1234-12345678\" = 1\n"));
    let r = run(&games.0, &out.0);
    assert_eq!(list(&out.0), arranged);
    let shown = table(&r);
    assert!(shown.contains("\"looper/0000-00000000\" names no score of the pack; ignored\n"), "{shown}");
    assert!(shown.contains("\"gone/1234-12345678\" names no score of the pack; ignored\n"), "{shown}");
    assert!(report(&out.0).contains("\"gone/1234-12345678\" names no score of the pack; ignored\n"));

    // choosing the original again: the learned list
    write(&choices, &format!("[choices]\n\"looper/{key}\" = 0\n"));
    run(&games.0, &out.0);
    assert_eq!(list(&out.0), original);

    // a choices file that does not read fails the run, naming it
    write(&choices, "[choices]\n\"looper/x\" = \"two\"\n");
    let e = run_with_limit(&games.0, &out.0, None).expect_err("a bad choices file");
    assert!(e.contains("choices.toml: line 2: ") && e.contains(&choices.canonicalize().unwrap().display().to_string()), "{e}");
    write(&choices, "[choices]\n\"looper/x\" = -1\n");
    let e = run_with_limit(&games.0, &out.0, None).expect_err("a bad take number");
    assert!(e.contains("\"looper/x\" = -1: a take is 1 or more, 0 the original"), "{e}");
}

/// `games/test/tune.sna`: a tune its learned entry plays.
fn tune_sna() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna")).expect("tune.sna")
}

#[test]
fn a_shadow_score_is_packed_only_with_a_take() {
    let games = Tmp::new("shadow-games");
    std::fs::write(games.0.join("tune.sna"), tune_sna()).unwrap();
    let out = Tmp::new("shadow-out");
    let r = run(&games.0, &out.0);
    let rep = report(&out.0);
    println!("{}\n{rep}", table(&r));

    // the learned entry alone in the pack; the shadow score written out, its line in the report
    let learned = list(&out.0);
    assert_eq!((learned[0], decode_scores(&learned).unwrap().len()), (1, 0));
    let cached = std::fs::read_dir(out.0.join("sound-cache")).unwrap().next().expect("a cache file").unwrap().path();
    let (_, runs, _, _, shadows) = cache_decode(&std::fs::read(&cached).unwrap()).expect("the cache file reads");
    assert_eq!(shadows.len(), 1);
    let rec = shadows[0].clone();
    assert!(rec.precedence);
    let key = score_key(&rec);
    assert!(rep.contains("\n    shadow score: ") && rep.contains(&format!(" (score {key}; packed only with a chosen take)\n")), "{rep}");
    let folder = out.0.join("arrange/tune").join(&key);
    let text = std::fs::read_to_string(folder.join("score.toml")).expect("score.toml written");
    assert_eq!(text, score_toml("tune", "tune", &rec));
    assert!(text.contains("\nshadow = true\n"), "{text}");

    // a take and no choice: still not packed; the live version plays
    let mut take = format!(
        "key = \"{key}\"\nintro_t = {}\nloop_t = {}\ntake = 1\ngenerator = \"octave down\"\n",
        rec.intro_t, rec.loop_t
    );
    for n in timeline(&rec) {
        take += &format!("\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = 200\n", n.voice, n.start_t, n.end_t, 2 * n.period_t);
    }
    write(&folder.join("take-1/arranged.toml"), &take);
    run(&games.0, &out.0);
    assert_eq!(list(&out.0), learned);
    assert!(report(&out.0).contains("\n      arranged: the live version (no take chosen)\n"), "{}", report(&out.0));

    // chosen: packed after the entry with precedence, the trigger and fingerprints kept
    write(&out.0.join("arrange/choices.toml"), &format!("[choices]\n\"tune/{key}\" = 1\n"));
    let r = run(&games.0, &out.0);
    let shown = table(&r);
    let rep = report(&out.0);
    println!("{shown}\n{rep}");
    let packed = list(&out.0);
    let got = decode_scores(&packed).unwrap();
    assert_eq!((packed[0], got.len()), (2, 1));
    let a = &got[0];
    assert!(a.precedence);
    assert_eq!((a.site, a.code, a.intro_fp.clone(), a.loop_fp.clone()), (rec.site, rec.code, rec.intro_fp.clone(), rec.loop_fp.clone()));
    assert!(a.intro_events.iter().chain(&a.loop_events).filter_map(|e| e.on).all(|(p, _)| timeline(&rec).iter().any(|n| n.period_t * 2 == p)));
    assert!(rep.contains("\n      arranged: take 1 (octave down)\n"), "{rep}");
    assert!(shown.contains(&format!("tune: shadow score {key} plays take 1 (octave down)\n")), "{shown}");
    assert!(shown.contains("tune: music learned (1 routine, heard, 1 channel; 1 shadow score, 1 arranged)\n"), "{shown}");
    // the replay verifies it in the run that recorded it: it plays over the learned entry
    let sna = tune_sna();
    let load = || {
        let mut m = Machine::new();
        m.load_sna(&sna).unwrap();
        m
    };
    let inputs = runs.expect("the runs").inputs(Of::Shadow(0)).expect("its run");
    soundlearn::scores::verify(&load, &inputs, &packed, 0, true).expect("verified");
    // without precedence the learned entry keeps its frames and the replay refuses it
    let plain = soundlearn::replace_scores(&packed, &[ScoreRecord { precedence: false, ..a.clone() }]).unwrap();
    assert!(soundlearn::scores::verify(&load, &inputs, &plain, 0, true).is_err());

    // the original chosen again: the learned list
    write(&out.0.join("arrange/choices.toml"), &format!("[choices]\n\"tune/{key}\" = 0\n"));
    run(&games.0, &out.0);
    assert_eq!(list(&out.0), learned);
}
