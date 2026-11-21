// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The learner for one game: `games/test/tune.sna` learned through the whole pipeline from its
//! code (its one channel the lead), routines that mix their channels into the speaker bit
//! learned by observation (and one whose register channels read differently at its `OUT`s
//! refused), a routine the built-in ROM BEEP plays given its roles in a kind-1 record, and a
//! learner that fails is caught.

mod fixtures;

use soundlearn::listen::{schedule, Inputs};
use soundlearn::{learn, BuiltinRoles, GameSound, Op};
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
    assert_eq!(got.line, "music learned (1 routine, heard, 1 channel)");
    assert_eq!(got.entries.len(), 1);
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
    assert_eq!(got.line, "music learned (1 routine, heard, observed, 2 channels)");
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
    assert_eq!(got.line, "music learned (1 routine, heard, observed, 2 channels)");
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
    assert_eq!(got.line, "music learned (1 routine, heard, observed, 2 channels)");
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
    assert_eq!(s.fingerprint.len(), decided - f0 - usize::from(t0 != 0));
    assert!(s.roles[..usize::from(s.voices)].contains(&zx_sys::SND_ROLE_BASS) && s.roles.contains(&SND_ROLE_LEAD), "{s:?}");
    let list = soundlearn::encode_list_scores(&got.entries, &got.roles, &got.scores);
    assert_eq!(SndEngines::with_learned(&list).expect("the list decodes").scores(), 1);
    let d = &got.details;
    assert!(d.contains(&format!("\n    recorded score: {:.1} s from frame {f0}, {} voices, ", s.fingerprint.len() as f64 * 0.019968, s.voices)), "{d}");
    assert!(!d.contains("shortened") && !d.contains("no score"), "{d}");
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
    assert_eq!(s.fingerprint.len(), PRESS as usize - f0 - usize::from(t0 != 0), "the last bucket ends in frame {}", PRESS - 1);
    assert!(got.details.contains(&format!("\n    recorded score: {:.1} s from frame {f0}, ", s.fingerprint.len() as f64 * 0.019968)), "{}", got.details);
}
