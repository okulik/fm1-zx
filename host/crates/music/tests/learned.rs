// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Learned entries (`core/sound/learned.c`) in method B: the "Test burst" entry of
//! `core/sound/engines_test.c` re-expressed as data decides every frame of `burst_sna` as the
//! built-in reader does, the learned entry follows its code to another address but not into
//! changed code, and the mixer plays FM: engines from learned entries.

use fmzx_pack::testprogs::burst_sna;
use soundlearn::{encode_list, LearnedEntry, Op};
use music::RATE;
use zx_sys::{Beeper, Machine, SndEngines, SndFrame, SndMixer, SND_MODE_FM_B, SND_SITE_TONAL};

/// The "Test burst" entry as data: period 2 (13n + 54) x 256 from the operand n at +13h, silent
/// for n = 0, as `read_burst`.
fn burst_entry() -> LearnedEntry {
    LearnedEntry {
        kind: 0,
        digest: 0xd4b5_51f4_6da9_de4c,
        len: 0x2C,
        wild: vec![0x13],
        reloc: vec![0x02, 0x09],
        sites: vec![(0x1A, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        drum: 0,
        stale: None,
        level: vec![200],
        progs: vec![vec![Op::PeekRel(0x13), Op::ZeroSilent, Op::PeekRel(0x13), Op::K(13), Op::Mul, Op::K(54), Op::Add, Op::K(512), Op::Mul]],
        timing: None,
    }
}

/// Method B's decision about each of `frames` frames of `m` (the last SND_DELAY_FRAMES are still
/// to come when the run ends).
fn decisions(mut m: Machine, mut b: SndEngines, frames: usize) -> Vec<SndFrame> {
    (0..frames)
        .map(|_| {
            m.run_frame();
            b.frame(&m, m.edges(), m.edges_start_level())
        })
        .collect()
}

fn burst(base: u16) -> Machine {
    let mut m = Machine::new();
    m.load_sna(&burst_sna(base)).expect("loads");
    m
}

#[test]
fn learned_burst_equals_builtin() {
    let list = encode_list(&[burst_entry()]);
    let builtin = decisions(burst(0x8000), SndEngines::new_test(), 600);
    let learned = decisions(burst(0x8000), SndEngines::with_learned(&list).expect("decodes"), 600);
    let mut confident = 0;
    for (i, (a, b)) in builtin.iter().zip(&learned).enumerate() {
        assert_eq!((a.confident, a.tone, a.drum), (b.confident, b.tone, b.drum), "decision {i}");
        assert_eq!((a.engine, a.events()), (b.engine, b.events()), "decision {i}");
        confident += usize::from(a.confident != 0);
    }
    println!("{confident} confident decisions of 600");
    assert!(confident >= 100, "{confident} confident decisions");
    let b = SndEngines::with_learned(&list).unwrap();
    assert_eq!(b.name(learned.iter().find(|d| d.confident != 0).unwrap().engine), "learned");
}

/// burst_sna's code and melody (at 8000h) copied to `to`, its two absolute words re-relocated
/// (+02h: the melody at +2Ch, +09h: the operand at +13h), and run from there.
fn moved(to: u16) -> Machine {
    const LEN: u16 = 0x2C + 8 * 5 + 1;
    let mut m = burst(0x8000);
    for i in 0..LEN {
        m.poke(to + i, m.peek(0x8000 + i));
        m.poke(0x8000 + i, 0);
    }
    for (at, word) in [(0x02u16, to + 0x2C), (0x09, to + 0x13)] {
        m.poke(to + at, word as u8);
        m.poke(to + at + 1, (word >> 8) as u8);
    }
    let mut r = m.regs();
    r.pc = to;
    m.set_regs(&r);
    m
}

fn claimed(d: &[SndFrame]) -> usize {
    d.iter().filter(|d| d.confident != 0).count()
}

#[test]
fn learned_entry_moves_with_code() {
    const TO: u16 = 0xA123;
    let entry = burst_entry();
    let list = encode_list(std::slice::from_ref(&entry));
    let n = claimed(&decisions(moved(TO), SndEngines::with_learned(&list).unwrap(), 300));
    println!("moved to {TO:04X}: {n} frames claimed");
    assert!(n >= 50, "moved: {n} frames claimed");

    // the same code with the LD C,0 operand at +11h changed (the speaker starts high: the same
    // tones): a signature byte, unless the entry leaves it out
    let changed = || {
        let mut m = moved(TO);
        m.poke(TO + 0x11, 0x10);
        m
    };
    let mut wider = LearnedEntry { wild: vec![0x11, 0x13], digest: 0, ..entry.clone() };
    let probe = SndEngines::with_learned(&encode_list(std::slice::from_ref(&wider))).unwrap();
    let index = probe.entries().len() - 1;
    wider.digest = probe.digest(&changed(), index, TO);
    assert_eq!(probe.digest(&moved(TO), index, TO), wider.digest, "the byte at +11h is left out");
    let n = claimed(&decisions(changed(), SndEngines::with_learned(&encode_list(&[wider])).unwrap(), 300));
    println!("+11h changed, left out: {n} frames claimed");
    assert!(n >= 50, "a wildcard byte changed: {n} frames claimed");

    let mut m = changed();
    let mut edges = 0;
    let mut b = SndEngines::with_learned(&list).unwrap();
    let mut n = 0;
    for _ in 0..300 {
        m.run_frame();
        edges += m.edges().len();
        n += usize::from(b.frame(&m, m.edges(), m.edges_start_level()).confident != 0);
    }
    println!("+11h changed, in the signature: {n} frames claimed ({edges} edges)");
    assert!(edges > 500, "the changed code still plays ({edges} edges)");
    assert_eq!(n, 0, "a signature byte changed: {n} frames claimed");
}

#[test]
fn mixer_plays_learned_entries() {
    // frames the mixer plays FM for in FM: engines, on burst_sna, with the built-in table and then
    // with the burst entry learned
    let fm_frames = |mixer: &mut SndMixer| {
        let mut m = burst(0x8000);
        let mut beeper = Beeper::new(RATE.0, RATE.1);
        (0..200)
            .filter(|_| {
                m.run_frame();
                beeper.set_level(m.edges_start_level());
                let samples = beeper.render(m.edges());
                mixer.frame(SND_MODE_FM_B, Some(&m), m.edges(), m.edges_start_level(), &samples);
                mixer.fm_on()
            })
            .count()
    };
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    assert_eq!(fm_frames(&mut mixer), 0, "the built-in table does not know burst_sna");
    assert!(mixer.set_learned(&[9, 0, 0, 0]).is_err(), "a count of 9");
    assert_eq!(fm_frames(&mut mixer), 0, "a refused list leaves the engines as they were");
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_learned(&encode_list(&[burst_entry()])).expect("decodes");
    let n = fm_frames(&mut mixer);
    println!("FM for {n} of 200 frames");
    assert!(n >= 150, "FM for {n} of 200 frames");
}
