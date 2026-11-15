// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The learner for one game: `games/test/tune.sna` learned through the whole pipeline, and a
//! learner that fails is caught.

use soundlearn::listen::{schedule, Inputs};
use soundlearn::{learn, GameSound};
use std::time::Instant;
use zx_sys::{Machine, SndEngines};

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
            details: "  learner error: no machine\n".into()
        }
    );
}
