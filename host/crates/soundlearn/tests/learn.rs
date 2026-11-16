// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The learner for one game: `games/test/tune.sna` learned through the whole pipeline (its one
//! channel the lead), a routine the built-in ROM BEEP plays given its roles in a kind-1 record,
//! and a learner that fails is caught.

mod fixtures;

use soundlearn::listen::{schedule, Inputs};
use soundlearn::{learn, BuiltinRoles, GameSound};
use std::time::Instant;
use zx_sys::{Machine, SndEngines, SND_ROLE_LEAD};

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
