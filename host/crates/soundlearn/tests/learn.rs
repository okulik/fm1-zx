// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The learner for one game: `games/test/tune.sna` learned through the whole pipeline from its
//! code (its one channel the lead), routines that mix their channels into the speaker bit
//! learned by observation (and one whose register channels read differently at its `OUT`s
//! refused), a routine the built-in ROM BEEP plays given its roles in a kind-1 record, and a
//! learner that fails is caught.

mod fixtures;

use soundlearn::listen::{run_inputs, schedule, Inputs, RunKind};
use soundlearn::{learn, learn_with, BuiltinRoles, GameSound, LearnOptions, Op, ScoreRecord};
use std::time::Instant;
use zx_sys::{Machine, SndEngines, SND_ROLE_LEAD, SND_SITE_TONAL};

fn tune() -> Machine {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna");
    let mut m = Machine::new();
    m.load_sna(&std::fs::read(path).expect("tune.sna")).expect("loads");
    m
}

#[test]
fn learns_tune() {
    let t = Instant::now();
    let got = learn(&tune, &schedule(1));
    println!("{}\n{}{:?}", got.line, got.details, t.elapsed());
    // its music, which the learned entry plays, is a shadow score too
    assert_eq!(got.line, "music learned (1 routine, heard, 1 channel; 1 shadow score)");
    assert_eq!(got.entries.len(), 1);
    assert!(got.scores.is_empty() && got.shadows.len() == 1 && got.shadows[0].precedence, "{:?}", got.shadows);
    assert_eq!(got.shadow_runs, [None]);
    let key = soundlearn::score_key(&got.shadows[0]);
    assert!(got.details.contains("\n    shadow score: 39.9 s from frame 1 "), "{}", got.details);
    assert!(got.details.contains(&format!(" KB (score {key}; packed only with a chosen take)\n")), "{}", got.details);
    // not in the game's list
    assert_eq!(soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores), soundlearn::encode_list(&got.entries));
    assert_eq!(got.entries[0].sites, vec![(0x14, zx_sys::SND_SITE_TONAL, 0)]);
    assert!(SndEngines::with_learned(&soundlearn::encode_list(&got.entries)).is_ok());
    assert!(got.details.starts_with("  OUT at 8014h, heard in "), "{}", got.details);
    assert!(got.details.contains("\n    learned\n"), "{}", got.details);
    assert_eq!(got.entries[0].instrument, [SND_ROLE_LEAD, 0, 0]);
    assert!(got.details.contains("\n    roles: channel 0 lead (analysis)\n"), "{}", got.details);
    assert!(got.roles.is_empty(), "{:?}", got.roles);
    // learned from its code: no observation
    assert!(!got.entries[0].observed);
    assert!(!got.details.contains("observ"), "{}", got.details);
    // the same game and inputs give the same result
    assert_eq!(learn(&tune, &schedule(1)), got);
}

#[test]
fn learner_errors_are_caught() {
    let none: Inputs = Box::new(|_| ([0; 8], 0));
    let broken = || -> Machine { panic!("no machine") };
    let got = learn(&broken, &none);
    assert_eq!(
        got,
        GameSound {
            entries: Vec::new(),
            line: "no music learned (learner error: no machine)".into(),
            details: "  learner error: no machine\n".into(),
            roles: Vec::new(),
            scores: Vec::new(),
            score_runs: Vec::new(),
            shadows: Vec::new(),
            shadow_runs: Vec::new(),
            override_error: None,
        }
    );
}

#[test]
fn builtin_routine_gets_kind1_roles() {
    let sna = fixtures::rom_beep_sna();
    let got = learn(&|| fixtures::machine(&sna), &schedule(1));
    println!("{}\n{}", got.line, got.details);
    assert!(got.entries.is_empty());
    let table = SndEngines::new().entries();
    let beep = table.iter().find(|e| e.name == "ROM BEEP").expect("ROM BEEP");
    assert_eq!(got.roles, [BuiltinRoles { digest: beep.digest, channels: 1, roles: [SND_ROLE_LEAD, 0, 0] }]);
    // said once, in the block of the BEEP's OUT heard in the most frames
    assert_eq!(got.details.matches("    roles: channel 0 lead (analysis)\n").count(), 1, "{}", got.details);
    // the BEEP plays its tune (long notes): a shadow score, not a score
    assert!(got.scores.is_empty() && got.shadows.len() == 1, "{}", got.details);
    assert!(got.line.ends_with("; 1 shadow score)"), "{}", got.line);
}

fn quiet() -> Inputs {
    Box::new(|_| ([0; 8], 0))
}

/// The replay line's claimed and sounding frames.
fn replay_claims(details: &str) -> (u32, u32) {
    let line = details.lines().find(|l| l.starts_with("    replay: ")).expect("a replay line");
    let n: Vec<u32> = line.split(' ').filter_map(|w| w.parse().ok()).collect();
    (n[0], n[1])
}

/// The two phase accumulators ORed into the speaker bit ([`fixtures::or_mixed_sna`]): the code
/// reader refuses it; observed, its channels are the steps in memory, and method B claims its
/// frames.
#[test]
fn learns_a_mixing_routine_by_observation() {
    let sna = fixtures::or_mixed_sna();
    let got = learn(&|| fixtures::machine(&sna), &schedule(1));
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.line, "music learned (1 routine, heard, observed, 2 channels; 1 shadow score)");
    assert_eq!(got.entries.len(), 1);
    let e = &got.entries[0];
    assert!(e.observed && !e.flips && e.timing.is_none() && e.pass == 0, "{e:?}");
    assert_eq!(e.sites, vec![(fixtures::OR_MIXED_OUT - 0x1B, SND_SITE_TONAL, 0)], "the anchor at the pass loop");
    assert_eq!(e.instrument[..2].iter().filter(|&&r| r != 0).count(), 2, "{e:?}");
    for (p, a) in e.progs.iter().zip(fixtures::OR_MIXED_STEPS) {
        assert_eq!(p[0], Op::Peek16(a), "{p:?}");
    }
    assert!(SndEngines::with_learned(&soundlearn::encode_list(&got.entries)).is_ok());
    let d = &got.details;
    assert!(d.starts_with("  OUT at 803Eh, heard in "), "{d}");
    assert!(d.contains("\n    from its code: "), "{d}");
    assert!(d.contains("\n    observed routine 801Bh-8045h: OUT 803Eh, pass "), "{d}");
    assert!(d.contains("\n    channel 0: word (9004h), 16-bit accumulator, score "), "{d}");
    assert!(d.contains("\n    channel 1: word (9006h), 16-bit accumulator, score "), "{d}");
    assert!(d.contains("; music gate passed\n    learned by observation: 2 channels\n    roles: channel 0 bass (analysis), channel 1 lead (analysis)\n"), "{d}");
    let (claimed, frames) = replay_claims(d);
    assert!(frames > 8000 && claimed * 100 >= frames * 99, "{claimed} of {frames}");
    assert_eq!(learn(&|| fixtures::machine(&sna), &schedule(1)), got);
}

/// A counter engine whose second channel reloads from IYh ([`fixtures::iyh_counter_engine`]):
/// the program reads IY and shifts.
#[test]
fn observes_a_channel_in_a_byte_of_iy() {
    let (code, l, _) = fixtures::iyh_counter_engine();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert!(got.line.starts_with("music learned (1 routine, heard, observed, 2 channels"), "{}", got.line);
    let e = &got.entries[0];
    assert_eq!(e.sites, vec![(l["out"] - l["loop"], SND_SITE_TONAL, 0)]);
    let iy = e.progs.iter().find(|p| p[0] == Op::Reg(12)).expect("a channel read from IY");
    assert_eq!(iy[1], Op::Shr(8), "{iy:?}");
    assert!(got.details.contains(&format!("\n    channel 0: ({:04X}h), counter x1, score ", l["r1"] + 1)), "{}", got.details);
    assert!(got.details.contains("\n    channel 1: IYh, counter x1, score "), "{}", got.details);
    assert!(got.details.contains("\n    learned by observation: 2 channels\n"), "{}", got.details);
}

/// Method B reads the registers of the `OUT` that made the frame's last edge: a channel in a
/// register is kept only if it reads the same at each of the routine's `OUT`s. The
/// accumulator engine's steps stay in DE and DE' across its two `OUT`s; with the register sets
/// swapped between them, neither does.
#[test]
fn register_channels_read_the_same_at_every_out() {
    let (code, l, _) = fixtures::accumulator_engine();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.line, "music learned (1 routine, heard, observed, 2 channels; 1 shadow score)");
    let e = &got.entries[0];
    assert_eq!(e.sites, vec![(l["out1"] - l["loop"], SND_SITE_TONAL, 0), (l["out2"] - l["loop"], SND_SITE_TONAL, 0)]);
    let regs: Vec<Op> = e.progs.iter().map(|p| p[0]).collect();
    assert!(regs.len() == 2 && regs.contains(&Op::Reg(9)) && regs.contains(&Op::Reg(16)), "{regs:?}");
    // the second OUT is the observed routine's: no block of its own
    assert_eq!(got.details.matches("  OUT at ").count(), 1, "{}", got.details);
    assert!(got.details.contains(": DE, 16-bit accumulator, score ") && got.details.contains(": DE', 16-bit accumulator, score "), "{}", got.details);

    let (code, _, _) = fixtures::swapped_accumulator_engine();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert!(got.entries.is_empty());
    // no routine learned: its music is recorded as a score instead
    assert!(got.line.starts_with("music recorded ("), "{}", got.line);
    let d = &got.details;
    assert!(d.contains("\n    channel dropped: DE, 16-bit accumulator, score "), "{d}");
    assert!(d.contains("\n    channel dropped: DE', 16-bit accumulator, score "), "{d}");
    assert!(d.contains(": reads the same at the routine's OUTs in 0% of the frames two of them sound in (at least 95%)\n"), "{d}");
    assert!(d.contains("\n    observation: no channel reads the same at all the routine's OUTs\n"), "{d}");
    assert!(!d.contains("replay"), "{d}");
}

/// Notes of [`mixing_tune`]: channel 0 and 1's steps.
const TUNE: [(u16, u16); 8] = [(265, 662), (297, 891), (334, 835), (354, 1062), (396, 990), (445, 1112), (499, 1497), (315, 788)];
/// The tune after a key press.
const TUNE2: [(u16, u16); 4] = [(600, 1500), (530, 1590), (700, 1400), (450, 1350)];

/// A tune of two phase accumulators mixed into the speaker bit, its register sets swapped between
/// its two `OUT`s (as [`fixtures::swapped_accumulator_engine`]: neither the code reader nor the
/// observation gives it), after about 70 frames of silence, looping; at each note it reads the
/// SPACE key, and while it is held it plays the second tune from its start, which it loops from
/// then on.
///
/// ```text
/// start: DI ; LD H,3
/// outer: LD BC,0
/// dly:   DEC BC ; LD A,B ; OR C ; JR NZ,dly ; DEC H ; JR NZ,outer
///        LD HL,table ; LD (base),HL
/// first: LD IX,(base)
/// next:  LD A,7Fh ; IN A,(FEh) ; RRA ; JR C,nokey
///        LD HL,table2 ; LD (base),HL ; LD IX,table2
/// nokey: LD E,(IX+0) ; LD D,(IX+1) ; LD A,D ; OR E ; JR Z,first
///        EXX ; LD E,(IX+2) ; LD D,(IX+3) ; EXX ; LD A,(IX+4) ; LD (dur),A ; INC IX x5
/// loop:  ADD HL,DE ; SBC A,A ; AND 16 ; LD B,A ; EXX ; ADD HL,DE ; SBC A,A ; AND 16 ; EXX ; OR B
/// out1:  OUT (254),A ; XOR A ; EXX ; NOP
/// out2:  OUT (254),A ; EXX ; DEC C ; JP NZ,loop
///        LD A,(dur) ; DEC A ; LD (dur),A ; JP NZ,loop ; JP next
/// ```
fn mixing_tune() -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3, 0x26, 0x03]);
    a.label("outer").b(&[0x01, 0x00, 0x00]);
    a.label("dly").b(&[0x0B, 0x78, 0xB1]).r(0x20, "dly").b(&[0x25]).r(0x20, "outer");
    a.w(&[0x21], "table").w(&[0x22], "base");
    a.label("first").w(&[0xDD, 0x2A], "base");
    a.label("next").b(&[0x3E, 0x7F, 0xDB, 0xFE, 0x1F]).r(0x38, "nokey");
    a.w(&[0x21], "table2").w(&[0x22], "base").w(&[0xDD, 0x21], "table2");
    a.label("nokey").b(&[0xDD, 0x5E, 0x00, 0xDD, 0x56, 0x01, 0x7A, 0xB3]).r(0x28, "first");
    a.b(&[0xD9, 0xDD, 0x5E, 0x02, 0xDD, 0x56, 0x03, 0xD9, 0xDD, 0x7E, 0x04]).w(&[0x32], "dur");
    a.b(&[0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23]);
    a.label("loop").b(&[0x19, 0x9F, 0xE6, 0x10, 0x47, 0xD9, 0x19, 0x9F, 0xE6, 0x10, 0xD9, 0xB0]);
    a.label("out1").b(&[0xD3, 0xFE, 0xAF, 0xD9, 0x00]);
    a.label("out2").b(&[0xD3, 0xFE, 0xD9, 0x0D]).w(&[0xC2], "loop");
    a.w(&[0x3A], "dur").b(&[0x3D]).w(&[0x32], "dur").w(&[0xC2], "loop").w(&[0xC3], "next");
    a.label("dur").b(&[0]);
    a.label("base").b(&[0, 0]);
    let units = |len: u32| (len * zx_sys::ZX_FRAME_TSTATES / (112 * 256)) as u8;
    a.label("table");
    for (&(s0, s1), &len) in TUNE.iter().zip(&fixtures::LENGTHS) {
        a.b(&s0.to_le_bytes()).b(&s1.to_le_bytes()).b(&[units(len + 2)]);
    }
    a.b(&[0, 0]);
    a.label("table2");
    for &(s0, s1) in &TUNE2 {
        a.b(&s0.to_le_bytes()).b(&s1.to_le_bytes()).b(&[units(9)]);
    }
    a.b(&[0, 0]);
    a.finish()
}

/// The first speaker edge from `site` of a run of `code` under `inputs`: (frame, T-state).
fn first_edge_from(code: &[u8], inputs: &Inputs, site: u16) -> (usize, u32) {
    let mut m = fixtures::bare(code);
    for f in 0..9000 {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let level = m.edges_start_level();
        if let Some(e) = soundlearn::entry::ear_edges(level, m.edges()).find(|e| e.pc == site) {
            return (f as usize, e.tstate);
        }
    }
    panic!("no edge from {site:04X}h");
}

/// The mixing tune nothing reads: its music is recorded as a score, from its first edge (after
/// its silence) to the end of the frames method B decides, verified by the replay.
#[test]
fn records_a_score_for_music_nothing_reads() {
    let (code, l) = mixing_tune();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert!(got.entries.is_empty());
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    let s = &got.scores[0];
    assert_eq!(got.line, format!("music recorded (1 score, {} voices)", s.voices));
    assert!(s.voices >= 2, "{s:?}");
    let (f0, t0) = first_edge_from(&code, &quiet(), l["out1"]);
    assert!(f0 > 60, "after the silence: frame {f0}");
    assert_eq!((s.site, s.quiet), (l["out1"], 25));
    assert_eq!(s.code, soundlearn::entry::score_code(|a| fixtures::bare(&code).peek(a), l["out1"]));
    // to the frames method B decides: 9000 - its delay
    let decided = 9000 - zx_sys::SND_DELAY_FRAMES as usize;
    assert_eq!(s.intro_fp.len(), decided - f0 - usize::from(t0 != 0));
    assert!(s.roles[..usize::from(s.voices)].contains(&zx_sys::SND_ROLE_BASS) && s.roles.contains(&SND_ROLE_LEAD), "{s:?}");
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    assert_eq!(SndEngines::with_learned(&list).expect("the list decodes").scores(), 1);
    let d = &got.details;
    assert!(d.contains(&format!("\n    recorded score: {:.1} s from frame {f0}, {} voices, ", s.intro_fp.len() as f64 * 0.019968, s.voices)), "{d}");
    assert!(!d.contains("shortened") && !d.contains("no score"), "{d}");
    assert_eq!(got.score_runs, [None]);
    assert!(d.contains(&format!(" KB (score {})\n", soundlearn::score_key(s))), "{d}");
    // the same game and inputs give the same score
    assert_eq!(learn(&|| fixtures::bare(&code), &quiet()), got);
}

/// The mixing tune with SPACE pressed in the middle: the score's stretch ends with the frame
/// before the press (the tune that follows has no trigger of its own).
#[test]
fn a_key_press_ends_the_stretch() {
    let (code, l) = mixing_tune();
    const PRESS: u32 = 3000;
    let space: Inputs = Box::new(|f| {
        let mut rows = [0u8; 8];
        if (PRESS..PRESS + 5).contains(&f) {
            rows[7] = 1;
        }
        (rows, 0)
    });
    let got = learn(&|| fixtures::bare(&code), &space);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    let s = &got.scores[0];
    let (f0, t0) = first_edge_from(&code, &space, l["out1"]);
    assert_eq!(s.intro_fp.len(), PRESS as usize - f0 - usize::from(t0 != 0), "the last bucket ends in frame {}", PRESS - 1);
    assert!(got.details.contains(&format!("\n    recorded score: {:.1} s from frame {f0}, ", s.intro_fp.len() as f64 * 0.019968)), "{}", got.details);
}

/// The second tune of [`jukebox`] and [`medley`].
const TUNE_B: [(u16, u16); 8] = [(600, 1500), (530, 1590), (700, 1400), (450, 1350), (400, 1200), (500, 1000), (560, 1680), (640, 1280)];
/// Each note's length in [`jukebox`] and [`medley`], frames: a tune is 320 frames (6.4 s).
const NOTE_FRAMES: u32 = 40;

/// The labels of a [`player`]: its entry, loop, `OUT`s, length and tune.
struct Names {
    play: &'static str,
    looped: &'static str,
    out1: &'static str,
    out2: &'static str,
    dur: &'static str,
    tune: &'static str,
}

const PLAYER: Names = Names { play: "play", looped: "loop", out1: "out1", out2: "out2", dur: "dur", tune: "tune_a" };

/// The engine of [`mixing_tune`] (nothing reads it) as a subroutine, `play`: it plays the tune
/// at IX (step 0, step 1 and a length in 256 passes per note; 0, 0 ends it) and returns, the
/// speaker left low; `tune` follows it, and `tune_b` too when given.
///
/// ```text
/// play:  LD E,(IX+0) ; LD D,(IX+1) ; LD A,D ; OR E ; RET Z
///        EXX ; LD E,(IX+2) ; LD D,(IX+3) ; EXX ; LD A,(IX+4) ; LD (dur),A ; INC IX x5
/// loop:  ADD HL,DE ; SBC A,A ; AND 16 ; LD B,A ; EXX ; ADD HL,DE ; SBC A,A ; AND 16 ; EXX ; OR B
/// out1:  OUT (254),A ; XOR A ; EXX ; NOP
/// out2:  OUT (254),A ; EXX ; DEC C ; JP NZ,loop
///        LD A,(dur) ; DEC A ; LD (dur),A ; JP NZ,loop ; JP play
/// ```
fn player(a: &mut fixtures::Asm, n: &Names, tune: &[(u16, u16)], second: Option<&[(u16, u16)]>) {
    player_of(a, n, tune, second, NOTE_FRAMES);
}

/// [`player`] with notes of `note_frames` frames each (to the nearest 256 passes).
fn player_of(a: &mut fixtures::Asm, n: &Names, tune: &[(u16, u16)], second: Option<&[(u16, u16)]>, note_frames: u32) {
    a.label(n.play).b(&[0xDD, 0x5E, 0x00, 0xDD, 0x56, 0x01, 0x7A, 0xB3, 0xC8]);
    a.b(&[0xD9, 0xDD, 0x5E, 0x02, 0xDD, 0x56, 0x03, 0xD9, 0xDD, 0x7E, 0x04]).w(&[0x32], n.dur);
    a.b(&[0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23]);
    a.label(n.looped).b(&[0x19, 0x9F, 0xE6, 0x10, 0x47, 0xD9, 0x19, 0x9F, 0xE6, 0x10, 0xD9, 0xB0]);
    a.label(n.out1).b(&[0xD3, 0xFE, 0xAF, 0xD9, 0x00]);
    a.label(n.out2).b(&[0xD3, 0xFE, 0xD9, 0x0D]).w(&[0xC2], n.looped);
    a.w(&[0x3A], n.dur).b(&[0x3D]).w(&[0x32], n.dur).w(&[0xC2], n.looped).w(&[0xC3], n.play);
    a.label(n.dur).b(&[0]);
    let units = (note_frames * zx_sys::ZX_FRAME_TSTATES / (112 * 256)) as u8;
    for (name, notes) in [(n.tune, Some(tune)), ("tune_b", second)] {
        let Some(notes) = notes else { continue };
        a.label(name);
        for &(s0, s1) in notes {
            a.b(&s0.to_le_bytes()).b(&s1.to_le_bytes()).b(&[units]);
        }
        a.b(&[0, 0]);
    }
}

/// A menu of tunes: it waits for key 1 or 2 (and for its release), plays [`TUNE`] on key 1 and
/// `second` on key 2 through [`player`], then waits again; silence otherwise.
///
/// ```text
/// start: DI
/// wait:  LD BC,F7FEh ; IN A,(C) ; CPL ; AND 3 ; JR Z,wait ; LD E,A
/// rel:   IN A,(C) ; CPL ; AND 1Fh ; JR NZ,rel
///        LD IX,tune_a ; RR E ; JR C,go ; LD IX,tune_b
/// go:    CALL play ; JR wait
/// ```
fn jukebox(second: &[(u16, u16)]) -> (Vec<u8>, fixtures::Labels) {
    jukebox_of(second, NOTE_FRAMES)
}

/// [`jukebox`] with notes of `note_frames` frames each.
fn jukebox_of(second: &[(u16, u16)], note_frames: u32) -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3]);
    a.label("wait").b(&[0x01, 0xFE, 0xF7, 0xED, 0x78, 0x2F, 0xE6, 0x03]).r(0x28, "wait").b(&[0x5F]);
    a.label("rel").b(&[0xED, 0x78, 0x2F, 0xE6, 0x1F]).r(0x20, "rel");
    a.w(&[0xDD, 0x21], "tune_a").b(&[0xCB, 0x1B]).r(0x38, "go").w(&[0xDD, 0x21], "tune_b");
    a.label("go").w(&[0xCD], "play").r(0x18, "wait");
    player_of(&mut a, &PLAYER, &TUNE, Some(second), note_frames);
    a.finish()
}

/// [`jukebox`] with a player of its own for each tune: key 2's [`TUNE_B`] plays through a copy
/// of the engine (`out1_b`), never heard in the learner's run (key 1 once).
///
/// ```text
/// start: DI
/// wait:  LD BC,F7FEh ; IN A,(C) ; CPL ; AND 3 ; JR Z,wait ; LD E,A
/// rel:   IN A,(C) ; CPL ; AND 1Fh ; JR NZ,rel
///        RR E ; JR NC,two ; CALL play ; JR wait
/// two:   CALL play_b ; JR wait
/// ```
fn two_players() -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3]);
    a.label("wait").b(&[0x01, 0xFE, 0xF7, 0xED, 0x78, 0x2F, 0xE6, 0x03]).r(0x28, "wait").b(&[0x5F]);
    a.label("rel").b(&[0xED, 0x78, 0x2F, 0xE6, 0x1F]).r(0x20, "rel");
    a.b(&[0xCB, 0x1B]).r(0x30, "two").w(&[0xDD, 0x21], "tune_a").w(&[0xCD], "play").r(0x18, "wait");
    a.label("two").w(&[0xDD, 0x21], "tune_b2").w(&[0xCD], "play_b").r(0x18, "wait");
    player(&mut a, &PLAYER, &TUNE, None);
    let b = Names { play: "play_b", looped: "loop_b", out1: "out1_b", out2: "out2_b", dur: "dur_b", tune: "tune_b2" };
    player(&mut a, &b, &TUNE_B, None);
    a.finish()
}

/// Two tunes one after the other: about 70 frames of silence, [`TUNE`], about 146 frames of
/// silence, [`TUNE_B`], then silence.
///
/// ```text
/// start: DI ; LD H,3 ; CALL quiet ; LD IX,tune_a ; CALL play
///        LD H,6 ; CALL quiet ; LD IX,tune_b ; CALL play
/// end:   JR end
/// quiet: LD BC,0
/// dly:   DEC BC ; LD A,B ; OR C ; JR NZ,dly ; DEC H ; JR NZ,quiet ; RET
/// ```
fn medley() -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3, 0x26, 0x03]).w(&[0xCD], "quiet").w(&[0xDD, 0x21], "tune_a").w(&[0xCD], "play");
    a.b(&[0x26, 0x06]).w(&[0xCD], "quiet").w(&[0xDD, 0x21], "tune_b").w(&[0xCD], "play");
    a.label("end").r(0x18, "end");
    a.label("quiet").b(&[0x01, 0x00, 0x00]);
    a.label("dly").b(&[0x0B, 0x78, 0xB1]).r(0x20, "dly").b(&[0x25]).r(0x20, "quiet").b(&[0xC9]);
    player(&mut a, &PLAYER, &TUNE, Some(&TUNE_B));
    a.finish()
}

/// The frames method B gives each score of `list` in a run of `code` under `inputs` (the
/// frames it decides, from the run's first).
fn score_frames(code: &[u8], inputs: &Inputs, list: &[u8]) -> Vec<Vec<usize>> {
    let mut b = SndEngines::with_learned(list).expect("the list decodes");
    let (table, n) = (b.entries().len(), b.scores());
    let mut m = fixtures::bare(code);
    let mut got = vec![Vec::new(); n];
    let delay = zx_sys::SND_DELAY_FRAMES as usize;
    for f in 0..9000 {
        let (rows, kemp) = inputs(f as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let d = b.frame(&m, m.edges(), m.edges_start_level());
        if f >= delay && d.confident != 0 && usize::from(d.engine) > table {
            got[usize::from(d.engine) - table - 1].push(f - delay);
        }
    }
    got
}

/// Whether score `s` plays its whole stretch, from its trigger's frame `f0`, in `frames` (and
/// nowhere else).
fn plays_through(s: &soundlearn::ScoreRecord, f0: usize, frames: &[usize]) -> bool {
    let n = frames.len();
    n >= s.intro_fp.len() && n <= s.intro_fp.len() + 1 && frames.first() == Some(&f0) && frames.windows(2).all(|w| w[1] == w[0] + 1)
}

/// Key 1 learned: key 1 once is the learner's run (as [`choose_run`] picks it for a menu of
/// tunes).
fn key1() -> (Inputs, LearnOptions) {
    let kind = RunKind::Once(3);
    (run_inputs(kind, 0), LearnOptions { run: Some(kind), ..LearnOptions::default() })
}

/// A menu that plays one tune on key 1 and another on key 2: the learner's run (key 1 once)
/// gives the first score, the extra run key 2 once the second, through the same `OUT` with the
/// same code around it (the two share a trigger); each plays in its own run with the whole list.
#[test]
fn records_the_tune_behind_each_key() {
    assert_eq!(zx_sys::SND_SCORES_MAX, 8, "method B follows several scores that share a trigger");
    let (code, l) = jukebox(&TUNE_B);
    let (inputs, opts) = key1();
    let got = learn_with(&|| fixtures::bare(&code), &inputs, &opts);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 2, "{}", got.details);
    assert!(got.scores.iter().all(|s| s.site == l["out1"]) && got.scores[0].code == got.scores[1].code, "one trigger");
    assert_eq!(got.line, format!("music recorded (2 scores, {} voices)", got.scores.iter().map(|s| s.voices).max().unwrap()));
    let d = &got.details;
    assert_eq!(d.matches("\n    recorded score: ").count(), 1, "{d}");
    assert_eq!(d.matches("\n    recorded score (key 2 once): ").count(), 1, "{d}");
    assert!(!d.contains("no score"), "{d}");
    // each score's run kept, its key on its line
    let mut runs = got.score_runs.clone();
    runs.sort_by_key(|r| r.is_some());
    assert_eq!(runs, [None, Some(RunKind::Key(1))]);
    for s in &got.scores {
        assert_eq!(d.matches(&format!(" KB (score {})\n", soundlearn::score_key(s))).count(), 1, "{d}");
    }
    // each in its run: the learner's, and key 2 once
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let key2 = run_inputs(RunKind::Key(1), 0);
    let mut played = Vec::new();
    for run in [&inputs, &key2] {
        let (f0, _) = first_edge_from(&code, run, l["out1"]);
        assert!((505..510).contains(&f0), "after the key's release: frame {f0}");
        let frames = score_frames(&code, run, &list);
        let who: Vec<usize> = (0..2).filter(|&i| !frames[i].is_empty()).collect();
        assert_eq!(who.len(), 1, "one score plays: {frames:?}");
        assert!(plays_through(&got.scores[who[0]], f0, &frames[who[0]]), "{f0}: {:?}", frames[who[0]]);
        played.push(who[0]);
    }
    assert_ne!(played[0], played[1], "each run its own score");
    assert_eq!(learn_with(&|| fixtures::bare(&code), &inputs, &opts), got);
}

/// The same tune behind keys 1 and 2: one score (key 2's stretch is the same music).
#[test]
fn the_same_tune_behind_two_keys_is_one_score() {
    let (code, _) = jukebox(&TUNE);
    let (inputs, opts) = key1();
    let got = learn_with(&|| fixtures::bare(&code), &inputs, &opts);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    assert!(got.details.contains("\n    recorded score: "), "{}", got.details);
    assert!(!got.details.contains("key 2") && !got.details.contains("no score"), "{}", got.details);
}

/// Two tunes one after the other, silence between: a score each, from the learner's run, both
/// from the same `OUT` with the same code around it; each plays its own stretch.
#[test]
fn records_each_tune_of_a_run() {
    assert_eq!(zx_sys::SND_SCORES_MAX, 8, "method B follows several scores that share a trigger");
    let (code, l) = medley();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 2, "{}", got.details);
    assert!(got.scores.iter().all(|s| s.site == l["out1"]) && got.scores[0].code == got.scores[1].code, "one trigger");
    assert!(got.scores.iter().all(|s| s.loop_t == 0 && s.loop_fp.is_empty() && s.loop_events.is_empty()), "tunes that do not repeat: no loop");
    assert_eq!(got.details.matches("\n    recorded score: ").count(), 2, "{}", got.details);
    assert!(!got.details.contains("once)") && !got.details.contains("no score"), "{}", got.details);
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let frames = score_frames(&code, &quiet(), &list);
    // the triggers: the first edge, and the first after the silence between the tunes
    let (a0, _) = first_edge_from(&code, &quiet(), l["out1"]);
    let b0 = frames.iter().flatten().copied().filter(|&f| f > a0 + 400).min().expect("the second tune plays");
    let starts: Vec<usize> = frames.iter().map(|f| f[0]).collect();
    let (first, second) = if starts[0] == a0 { (0, 1) } else { (1, 0) };
    assert!(plays_through(&got.scores[first], a0, &frames[first]), "{:?}", frames[first]);
    assert!(plays_through(&got.scores[second], b0, &frames[second]), "{:?}", frames[second]);
    assert!(b0 > a0 + 320 + 100, "the second after the silence: {a0}, {b0}");
}

/// A tune behind key 2 played by code never heard in the learner's run (key 1 once): the block
/// of its `OUT` (found in the code only) gives the score from the extra run.
#[test]
fn records_a_tune_heard_only_in_an_extra_run() {
    let (code, l) = two_players();
    let (inputs, opts) = key1();
    let got = learn_with(&|| fixtures::bare(&code), &inputs, &opts);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 2, "{}", got.details);
    let sites: Vec<u16> = got.scores.iter().map(|s| s.site).collect();
    assert!(sites.contains(&l["out1"]) && sites.contains(&l["out1_b"]), "{sites:04X?}");
    let d = &got.details;
    let block = d.split("\n  OUT at ").find(|b| b.starts_with(&format!("{:04X}h, code only", l["out1_b"]))).expect("its block");
    assert!(block.contains("\n    recorded score (key 2 once): 6.4 s from frame "), "{d}");
    assert!(!d.contains("heard only in other runs") && d.matches("(key 2 once)").count() == 1, "its second OUT is the same music: {d}");
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let key2 = run_inputs(RunKind::Key(1), 0);
    let b = sites.iter().position(|&s| s == l["out1_b"]).unwrap();
    let (f0, _) = first_edge_from(&code, &key2, l["out1_b"]);
    assert!(plays_through(&got.scores[b], f0, &score_frames(&code, &key2, &list)[b]));
}

/// [`looping_tune`]'s intro: steps of channels 0 and 1 and a length in frames each; the last a
/// rest (steps too small to carry within it).
const INTRO: [(u16, u16, u8); 4] = [(265, 662, 40), (334, 835, 30), (445, 1112, 50), (1, 1, 20)];
/// The phrase [`looping_tune`] loops after its intro.
const PHRASE: [(u16, u16, u8); 6] = [(297, 891, 40), (354, 1062, 30), (396, 990, 40), (499, 1497, 30), (315, 788, 50), (600, 1500, 40)];
/// The phrase it loops once SPACE is pressed.
const PHRASE2: [(u16, u16, u8); 3] = [(530, 1590, 30), (700, 1400, 30), (450, 1350, 30)];

/// A tune paced by the frame interrupt (IM 2, an `EI ; RET` at 9191h): about 70 frames of
/// silence, then an intro and a phrase it loops, each note's accumulators from 0, so that each
/// pass of the phrase makes the same speaker edges at the same T-states in its frames. Each
/// note is a number of frames, each frame 512 passes of [`mixing_tune`]'s loop after the
/// interrupt (the speaker left low). Each frame it reads SPACE: while held, it plays
/// [`PHRASE2`] from its start, which it loops from then on.
///
/// ```text
/// start: DI ; LD A,90h ; LD I,A ; IM 2 ; EI ; LD H,70
/// quiet: HALT ; DEC H ; JR NZ,quiet
///        LD IX,intro ; CALL play
/// again: LD IX,(cur) ; CALL play ; JR again
/// play:  LD E,(IX+0) ; LD D,(IX+1) ; LD A,D ; OR E ; RET Z
///        EXX ; LD E,(IX+2) ; LD D,(IX+3) ; LD HL,0 ; EXX ; LD HL,0
///        LD A,(IX+4) ; LD (dur),A ; INC IX x5
/// frame: HALT ; LD A,7Fh ; IN A,(FEh) ; RRA ; JR NC,other ; LD A,2 ; LD (reps),A
/// loop:  ADD HL,DE ; SBC A,A ; AND 16 ; LD B,A ; EXX ; ADD HL,DE ; SBC A,A ; AND 16 ; EXX ; OR B
/// out1:  OUT (254),A ; XOR A ; EXX ; NOP
/// out2:  OUT (254),A ; EXX ; DEC C ; JP NZ,loop
///        LD A,(reps) ; DEC A ; LD (reps),A ; JP NZ,loop
///        LD A,(dur) ; DEC A ; LD (dur),A ; JP NZ,frame ; JP play
/// other: LD IX,phrase2 ; LD (cur),IX ; JP play
/// ```
fn looping_tune() -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3, 0x3E, 0x90, 0xED, 0x47, 0xED, 0x5E, 0xFB, 0x26, 70]);
    a.label("quiet").b(&[0x76, 0x25]).r(0x20, "quiet");
    a.w(&[0xDD, 0x21], "intro").w(&[0xCD], "play");
    a.label("again").w(&[0xDD, 0x2A], "cur").w(&[0xCD], "play").r(0x18, "again");
    a.label("play").b(&[0xDD, 0x5E, 0x00, 0xDD, 0x56, 0x01, 0x7A, 0xB3, 0xC8]);
    a.b(&[0xD9, 0xDD, 0x5E, 0x02, 0xDD, 0x56, 0x03, 0x21, 0, 0, 0xD9, 0x21, 0, 0]);
    a.b(&[0xDD, 0x7E, 0x04]).w(&[0x32], "dur").b(&[0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23, 0xDD, 0x23]);
    a.label("frame").b(&[0x76, 0x3E, 0x7F, 0xDB, 0xFE, 0x1F]).r(0x30, "other").b(&[0x3E, 0x02]).w(&[0x32], "reps");
    a.label("loop").b(&[0x19, 0x9F, 0xE6, 0x10, 0x47, 0xD9, 0x19, 0x9F, 0xE6, 0x10, 0xD9, 0xB0]);
    a.label("out1").b(&[0xD3, 0xFE, 0xAF, 0xD9, 0x00]);
    a.label("out2").b(&[0xD3, 0xFE, 0xD9, 0x0D]).w(&[0xC2], "loop");
    a.w(&[0x3A], "reps").b(&[0x3D]).w(&[0x32], "reps").w(&[0xC2], "loop");
    a.w(&[0x3A], "dur").b(&[0x3D]).w(&[0x32], "dur").w(&[0xC2], "frame").w(&[0xC3], "play");
    a.label("other").w(&[0xDD, 0x21], "phrase2").w(&[0xDD, 0x22], "cur").w(&[0xC3], "play");
    a.label("dur").b(&[0]);
    a.label("reps").b(&[0]);
    a.label("cur").w(&[], "phrase");
    for (name, notes) in [("intro", &INTRO[..]), ("phrase", &PHRASE[..]), ("phrase2", &PHRASE2[..])] {
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
    a.b(&vec![0; pad]).label("isr").b(&[0xFB, 0xC9]);
    a.finish()
}

/// The first speaker edge of frame `frame` of a run of `code` under `inputs`: its T-state.
fn first_edge_in(code: &[u8], inputs: &Inputs, frame: usize) -> u32 {
    let mut m = fixtures::bare(code);
    for f in 0..=frame {
        let (rows, kemp) = inputs(f as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
    }
    let level = m.edges_start_level();
    let t = soundlearn::entry::ear_edges(level, m.edges()).next().expect("an edge").tstate;
    t
}

/// The frames of the notes of a tune.
fn frames_of(notes: &[(u16, u16, u8)]) -> usize {
    notes.iter().map(|n| usize::from(n.2)).sum()
}

/// A tune that plays an intro and then loops a phrase: its score stores the intro and one pass
/// of the loop, the loop's start the phrase's first edge (the intro ends in a rest) and its
/// length the phrase's frames exactly (the tune is paced by the frame interrupt); method B
/// follows it round the loop to the end of the frames it decides.
#[test]
fn records_a_tune_that_loops_once_with_its_loop() {
    let (code, l) = looping_tune();
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    let s = &got.scores[0];
    let (f0, t0) = first_edge_from(&code, &quiet(), l["out1"]);
    let frame = zx_sys::ZX_FRAME_TSTATES;
    let p0 = f0 + frames_of(&INTRO);
    let intro_t = frames_of(&INTRO) as u32 * frame + first_edge_in(&code, &quiet(), p0) - t0;
    assert_eq!((s.intro_t, s.loop_t), (intro_t, frames_of(&PHRASE) as u32 * frame), "{s:?}");
    assert_eq!((s.intro_fp.len(), s.loop_fp.len()), ((intro_t / frame) as usize, frames_of(&PHRASE)));
    // the intro's notes and one pass's (each frame's burst of sound splits a note into more)
    let ons = |ev: &[soundlearn::ScoreEvent]| ev.iter().filter(|e| e.on.is_some()).count();
    assert!(ons(&s.intro_events) >= 3 && ons(&s.loop_events) >= 6, "{s:?}");
    assert!(s.intro_events.iter().all(|e| e.t < s.intro_t) && s.loop_events.iter().all(|e| e.t < s.loop_t));
    assert!(s.loop_events.iter().take_while(|e| e.t == 0).all(|e| e.on.is_some()), "the loop's start restates its voices");
    let d = &got.details;
    let decided = 9000 - zx_sys::SND_DELAY_FRAMES as usize;
    let seconds = |t: u32| f64::from(t) / 3_500_000.0;
    let line = format!(
        "\n    recorded score: {:.1} s from frame {f0} (intro {:.1} s, loop {:.1} s), {} voices, {} notes, ",
        (decided - f0 - usize::from(t0 != 0)) as f64 * 0.019968,
        seconds(s.intro_t),
        seconds(s.loop_t),
        s.voices,
        ons(&s.intro_events) + ons(&s.loop_events)
    );
    assert!(d.contains(&line), "{line}\n{d}");
    // method B plays it from its trigger to the end of the frames it decides, round the loop
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let frames = &score_frames(&code, &quiet(), &list)[0];
    assert_eq!(frames.first(), Some(&f0));
    assert_eq!(frames.len(), decided - f0, "{frames:?}");
    assert!(frames.windows(2).all(|w| w[1] == w[0] + 1));
    assert_eq!(learn(&|| fixtures::bare(&code), &quiet()), got);
}

/// [`looping_tune`] with SPACE pressed in its loop: the stretch ends with the frame before the
/// press, and the score keeps its loop (the music changes only after the stretch): method B
/// follows it round the loop to the press (the frames within its delay of the third bucket that
/// disagrees not the score's).
#[test]
fn a_loop_that_changes_after_its_stretch() {
    let (code, l) = looping_tune();
    const PRESS: u32 = 4000;
    let space: Inputs = Box::new(|f| {
        let mut rows = [0u8; 8];
        if (PRESS..PRESS + 5).contains(&f) {
            rows[7] = 1;
        }
        (rows, 0)
    });
    let got = learn(&|| fixtures::bare(&code), &space);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    let s = &got.scores[0];
    let (f0, _) = first_edge_from(&code, &space, l["out1"]);
    assert_eq!(s.loop_t, frames_of(&PHRASE) as u32 * zx_sys::ZX_FRAME_TSTATES, "{s:?}");
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let frames = &score_frames(&code, &space, &list)[0];
    assert_eq!(frames.first(), Some(&f0));
    assert!(frames.windows(2).all(|w| w[1] == w[0] + 1));
    let end = *frames.last().unwrap() as u32;
    assert!((PRESS - zx_sys::SND_DELAY_FRAMES..PRESS).contains(&end), "to the press: {end}");
}


/// [`TUNE`] once through [`player`], its notes `note_frames` frames each, after about 70 frames
/// of silence; then silence.
///
/// ```text
/// start: DI ; LD H,3 ; CALL quiet ; LD IX,tune_a ; CALL play
/// end:   JR end
/// quiet: LD BC,0
/// dly:   DEC BC ; LD A,B ; OR C ; JR NZ,dly ; DEC H ; JR NZ,quiet ; RET
/// ```
fn one_tune(note_frames: u32) -> (Vec<u8>, fixtures::Labels) {
    let mut a = fixtures::Asm::default();
    a.label("start").b(&[0xF3, 0x26, 0x03]).w(&[0xCD], "quiet").w(&[0xDD, 0x21], "tune_a").w(&[0xCD], "play");
    a.label("end").r(0x18, "end");
    a.label("quiet").b(&[0x01, 0x00, 0x00]);
    a.label("dly").b(&[0x0B, 0x78, 0xB1]).r(0x20, "dly").b(&[0x25]).r(0x20, "quiet").b(&[0xC9]);
    player_of(&mut a, &PLAYER, &TUNE, None, note_frames);
    a.finish()
}

/// A tune of about 3 s (8 notes of 19 frames) gets a score, which method B arms, chooses (after
/// its delay's buckets) and plays through; one of about 1.5 s does not (the shortest is 2 s, and
/// its `OUT` must sound in 100 frames).
#[test]
fn records_a_three_second_tune() {
    assert_eq!(soundlearn::scores::SCORE_FRAMES, 100);
    let (code, l) = one_tune(19);
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 1, "{}", got.details);
    let s = &got.scores[0];
    assert!((140..160).contains(&s.intro_fp.len()) && s.loop_t == 0, "{s:?}");
    assert!(got.details.contains("\n    recorded score: 3.0 s from frame "), "{}", got.details);
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let (f0, _) = first_edge_from(&code, &quiet(), l["out1"]);
    assert!(plays_through(s, f0, &score_frames(&code, &quiet(), &list)[0]));

    let (code, _) = one_tune(9);
    let got = learn(&|| fixtures::bare(&code), &quiet());
    println!("{}\n{}", got.line, got.details);
    // its OUT sounds in under 100 frames: no score is looked for
    assert!(got.scores.is_empty() && got.shadows.is_empty() && !got.details.contains("score"), "{}", got.details);
}

/// Two tunes of about 3 s behind keys 1 and 2, through one `OUT` with the same code around it:
/// a score each, and in each run method B follows both from the shared trigger and plays the
/// one whose fingerprint the beeper keeps to, through its stretch.
#[test]
fn short_tunes_that_share_a_trigger() {
    let (code, l) = jukebox_of(&TUNE_B, 19);
    let (inputs, opts) = key1();
    let got = learn_with(&|| fixtures::bare(&code), &inputs, &opts);
    println!("{}\n{}", got.line, got.details);
    assert_eq!(got.scores.len(), 2, "{}", got.details);
    assert!(got.scores.iter().all(|s| s.site == l["out1"] && s.intro_fp.len() < 160) && got.scores[0].code == got.scores[1].code);
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    let key2 = run_inputs(RunKind::Key(1), 0);
    let mut played = Vec::new();
    for run in [&inputs, &key2] {
        let (f0, _) = first_edge_from(&code, run, l["out1"]);
        let frames = score_frames(&code, run, &list);
        let who: Vec<usize> = (0..2).filter(|&i| !frames[i].is_empty()).collect();
        assert_eq!(who.len(), 1, "one score plays: {frames:?}");
        assert!(plays_through(&got.scores[who[0]], f0, &frames[who[0]]), "{f0}: {:?}", frames[who[0]]);
        played.push(who[0]);
    }
    assert_ne!(played[0], played[1], "each run its own score");
}

/// A tune of the ROM's BEEP routine whose every call is 12 ms, shorter than the built-in BEEP
/// entry takes (a frame): each tone of the tune twelve calls at its pitch (to the ear a note of
/// about 150 ms). The table claims none of it, so the routine inside the built-in entry gets a
/// score, not a shadow score, which plays in the replay.
#[test]
fn a_beep_tune_of_short_notes_gets_a_score() {
    let hz = [262.0, 392.0, 523.0, 330.0, 659.0, 440.0, 784.0, 349.0];
    let notes: Vec<(u16, u16)> = hz.iter().flat_map(|&f| std::iter::repeat_n(fixtures::beep_note(f, 0.012), 12)).collect();
    let sna = fixtures::rom_beep_tune_sna(&notes, 1);
    let load = || fixtures::machine(&sna);
    let got = learn(&load, &schedule(1));
    println!("{}\n{}", got.line, got.details);
    assert!(got.entries.is_empty());
    assert_eq!((got.scores.len(), got.shadows.len()), (1, 0), "{}", got.details);
    assert!(got.line.starts_with("music recorded (1 score, "), "{}", got.line);
    let d = &got.details;
    assert!(d.contains("\n    left out: built-in entry at 03B5h\n") && d.contains("\n    recorded score: "), "{d}");
    assert!(!got.scores[0].precedence);
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    soundlearn::scores::verify(&load, &schedule(1), &list, 0, true).expect("it plays");
}

/// The frames method B gives each score of `list` in a run of the game `load` gives under
/// `inputs`, and the frames the table claims (the frames it decides, from the run's first).
fn decided(load: &dyn Fn() -> Machine, inputs: &Inputs, list: &[u8]) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut b = SndEngines::with_learned(list).expect("the list decodes");
    let (table, n) = (b.entries().len(), b.scores());
    let mut m = load();
    let (mut got, mut tabled) = (vec![Vec::new(); n], Vec::new());
    let delay = zx_sys::SND_DELAY_FRAMES as usize;
    for f in 0..9000 {
        let (rows, kemp) = inputs(f as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let d = b.frame(&m, m.edges(), m.edges_start_level());
        if f >= delay && d.confident != 0 {
            match usize::from(d.engine) {
                e if e > table => got[e - table - 1].push(f - delay),
                e if e > 0 => tabled.push(f - delay),
                _ => {}
            }
        }
    }
    (got, tabled)
}

/// `tune.sna`'s music, which its learned entry plays, is a shadow score: kept beside the list,
/// not in it. Put in the list it plays over the learned entry only with precedence (the
/// replay verifies it, and method B gives it the frames the entry claimed); without, the table
/// keeps them and the replay refuses it.
#[test]
fn a_shadow_score_plays_over_the_learned_entry_only_with_precedence() {
    let got = learn(&tune, &schedule(1));
    assert_eq!((got.entries.len(), got.scores.len(), got.shadows.len()), (1, 0, 1), "{}", got.details);
    let shadow = got.shadows[0].clone();
    assert!(shadow.precedence);
    let alone = soundlearn::encode_list(&got.entries);
    assert_eq!(soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores), alone);
    let (_, tabled) = decided(&tune, &schedule(1), &alone);
    assert!(tabled.len() > 8900, "the learned entry plays it: {}", tabled.len());

    let first = soundlearn::encode_list_scores(&got.entries, &got.roles, std::slice::from_ref(&shadow));
    soundlearn::scores::verify(&tune, &schedule(1), &first, 0, true).expect("it plays with precedence");
    let (scored, tabled) = decided(&tune, &schedule(1), &first);
    assert!(scored[0].len() > 8900 && tabled.len() < 10, "{} {}", scored[0].len(), tabled.len());

    let plain = ScoreRecord { precedence: false, ..shadow };
    let last = soundlearn::encode_list_scores(&got.entries, &got.roles, std::slice::from_ref(&plain));
    let why = soundlearn::scores::verify(&tune, &schedule(1), &last, 0, true).expect_err("the table wins");
    assert!(why.contains("method B"), "{why}");
    let (scored, tabled) = decided(&tune, &schedule(1), &last);
    assert!(scored[0].len() < 100 && tabled.len() > 8900, "{} {}", scored[0].len(), tabled.len());
}
