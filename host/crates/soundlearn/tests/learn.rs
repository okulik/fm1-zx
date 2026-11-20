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
    assert!(got.line.starts_with("no music learned ("), "{}", got.line);
    let d = &got.details;
    assert!(d.contains("\n    channel dropped: DE, 16-bit accumulator, score "), "{d}");
    assert!(d.contains("\n    channel dropped: DE', 16-bit accumulator, score "), "{d}");
    assert!(d.contains(": reads the same at the routine's OUTs in 0% of the frames two of them sound in (at least 95%)\n"), "{d}");
    assert!(d.contains("\n    observation: no channel reads the same at all the routine's OUTs\n"), "{d}");
    assert!(!d.contains("replay"), "{d}");
}
