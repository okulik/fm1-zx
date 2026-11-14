// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Load to snapshot (`tapeload`): a turbo tape loaded through the tape signal, a loader that
//! never finishes, the `.z80` round trip, the trap trial and the routing, the success test, and
//! 128K detection. Every tape is generated.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use fmzx_pack::tapeload::{is_128k, load_signal, prepare, save_z80, trap_trial, Kind};
use fmzx_pack::testprogs::{
    halt_tap, hang_tzx, input_tap, ok_tap, pause_tap, reset_tap, rom_copy_payload, rom_copy_tap, stop_tap, tape_tap, tune_sna,
    turbo_tzx, tzx_header, wrap_tap_as_tzx, ROM_COPY_PAYLOAD_AT, TAPE_DONE_BORDER, TURBO_PAYLOAD, TURBO_PAYLOAD_AT,
};
use zx_sys::{zx_regs, Machine};

fn ram(m: &Machine, at: u16, len: usize) -> Vec<u8> {
    (0..len).map(|i| m.peek(at.wrapping_add(i as u16))).collect()
}

/// A fresh temporary folder for one test.
fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fmzx-tapeload-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn turbo_load_reaches_code() {
    let m = load_signal(&turbo_tzx()).expect("the turbo tape loads");
    assert!(ram(&m, TURBO_PAYLOAD_AT, TURBO_PAYLOAD.len()) == TURBO_PAYLOAD, "the payload is in RAM");
    assert_eq!(m.border(), TAPE_DONE_BORDER);
}

#[test]
fn hang_reported() {
    let t0 = Instant::now();
    let e = load_signal(&hang_tzx()).err().expect("a loader that never finishes fails");
    assert!(e.contains("still loading") || e.contains("never finished"), "{e}");
    assert!(t0.elapsed() < Duration::from_secs(30), "{:?}", t0.elapsed());
}

/// Every field of the registers, R as `LD A,R` reads it (the core's raw counter keeps another
/// bit 7).
fn regs_of(m: &Machine) -> [u16; 17] {
    let r: zx_regs = m.regs();
    [
        r.af, r.bc, r.de, r.hl, r.af_, r.bc_, r.de_, r.hl_, r.ix, r.iy, r.sp, r.pc,
        r.i as u16, m.r_register() as u16, r.iff1 as u16, r.iff2 as u16, r.im as u16,
    ]
}

#[test]
fn snapshot_round_trip() {
    let mut m = load_signal(&turbo_tzx()).expect("loads");
    let s = save_z80(&m);
    let mut m2 = Machine::new();
    m2.load_z80(&s).expect("our .z80 loads");
    assert_eq!(regs_of(&m2), regs_of(&m), "registers");
    assert_eq!(m2.regs().r & 0x7F, m.regs().r & 0x7F);
    assert_eq!(m2.border(), m.border());
    assert!(ram(&m2, 0x4000, 0xC000) == ram(&m, 0x4000, 0xC000), "48K of RAM");
    assert!(save_z80(&m2) == s, "saving again gives the same bytes");
    assert!(s.len() < 0xC000, "compressed: {} bytes", s.len());
    // the T-state counter just after the interrupt: low 17471 (counting down), high 3
    assert_eq!(u16::from_le_bytes([s[55], s[56]]), 17_471, "low T-state counter");
    assert_eq!(s[57], 3, "high T-state counter");

    let screen = |m: &Machine| ram(m, 0x4000, 0x1B00);
    let b0 = m.peek(0x4000);
    for f in 0..200 {
        m.run_frame();
        m2.run_frame();
        assert!(screen(&m) == screen(&m2), "screen differs at frame {f}");
    }
    assert_eq!(m.peek(0x4000), b0.wrapping_add(200), "the payload counts the frames on the screen");
}

#[test]
fn z80_compression_round_trips() {
    // runs, lone EDs, ED pairs and an ED before a run: the core's loader reads them all back
    let mut m = Machine::new();
    let patterns: [&[u8]; 6] =
        [&[0xED, 0, 0, 0, 0, 0, 0], &[0xED, 0xED], &[0xED], &[7; 300], &[1, 2, 3], &[0xED, 0xED, 0xED, 5]];
    let mut a = 0x8000u16;
    for _ in 0..40 {
        for p in patterns {
            for &b in p {
                m.poke(a, b);
                a += 1;
            }
        }
    }
    m.poke(0xFFFF, 0xED);
    let s = save_z80(&m);
    let mut m2 = Machine::new();
    m2.load_z80(&s).expect("loads");
    assert!(ram(&m2, 0x4000, 0xC000) == ram(&m, 0x4000, 0xC000));
    // a page of noise is stored, not compressed
    let mut x = 1u32;
    for a in 0xC000..=0xFFFFu16 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        m.poke(a, (x >> 16) as u8);
    }
    let s = save_z80(&m);
    let mut m2 = Machine::new();
    m2.load_z80(&s).expect("loads");
    assert!(ram(&m2, 0x4000, 0xC000) == ram(&m, 0x4000, 0xC000));
}

#[test]
fn trap_trial_ok_and_not() {
    trap_trial(&tape_tap()).expect("the multi-load tape loads through the trap");

    let tap = rom_copy_tap();
    let e = trap_trial(&tap).expect_err("the trap cannot serve a loader of its own");
    assert!(e.contains("reads the rest itself"), "{e}");

    let dir = tmp("copy");
    let path = dir.join("copy.tap");
    std::fs::write(&path, &tap).unwrap();
    let got = prepare(&path).expect("loads through the signal");
    assert_eq!(got.how, "loaded (own loader)");
    assert_eq!(got.kind, Kind::Snapshot);
    let mut m = Machine::new();
    m.load_z80(&got.data).expect("a .z80");
    let payload = rom_copy_payload();
    assert!(ram(&m, ROM_COPY_PAYLOAD_AT, payload.len()) == payload, "the headerless block is in RAM");
    assert_eq!(m.border(), TAPE_DONE_BORDER);

    // the standard tape goes in as a tape, a standard .tzx as its .tap, a snapshot as it is
    let p = dir.join("tape.tap");
    std::fs::write(&p, tape_tap()).unwrap();
    let got = prepare(&p).unwrap();
    assert_eq!((got.kind, got.how), (Kind::Tape, "tape"));
    assert!(got.data == tape_tap());
    let p = dir.join("tape.TZX");
    std::fs::write(&p, wrap_tap_as_tzx(&tape_tap())).unwrap();
    let got = prepare(&p).unwrap();
    assert_eq!((got.kind, got.how), (Kind::Tape, "tape"));
    assert!(got.data == tape_tap());
    let p = dir.join("tune.sna");
    std::fs::write(&p, tune_sna()).unwrap();
    let got = prepare(&p).unwrap();
    assert_eq!((got.kind, got.how, got.frames), (Kind::Snapshot, "as is", 0));
    assert!(got.data == tune_sna());
    let p = dir.join("turbo.tzx");
    std::fs::write(&p, turbo_tzx()).unwrap();
    let got = prepare(&p).unwrap();
    assert_eq!((got.kind, got.how), (Kind::Snapshot, "loaded"));
    let p = dir.join("hang.tzx");
    std::fs::write(&p, hang_tzx()).unwrap();
    assert!(prepare(&p).unwrap_err().contains("still loading"));
    let p = dir.join("notes.txt");
    std::fs::write(&p, b"hello").unwrap();
    assert!(prepare(&p).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn success_rules() {
    let e = trap_trial(&stop_tap()).expect_err("a program that stops at a report fails");
    assert!(e.contains("error report") && e.contains("9 STOP statement"), "{e}");
    let e = load_signal(&stop_tap()).err().expect("so it does through the signal");
    assert!(e.contains("error report") && e.contains("9 STOP statement"), "{e}");
    let e = trap_trial(&reset_tap()).expect_err("a reset fails");
    assert!(e.contains("copyright"), "{e}");
    let e = trap_trial(&halt_tap()).expect_err("DI; HALT fails");
    assert!(e.contains("halted with interrupts off"), "{e}");
    let e = trap_trial(&ok_tap()).expect_err("a program that ends at the editor fails");
    assert!(e.contains("BASIC editor"), "{e}");
    let e = load_signal(&ok_tap()).err().expect("so it does through the signal");
    assert!(e.contains("BASIC editor"), "{e}");
    // a BASIC program still running in the ROM has started: PAUSE 0, INPUT
    trap_trial(&pause_tap()).expect("PAUSE 0 runs");
    trap_trial(&input_tap()).expect("INPUT runs");
    load_signal(&pause_tap()).expect("PAUSE 0 runs through the signal");
}

#[test]
fn bad_files_refused() {
    // a cut tape, a bad TZX block and a bad snapshot: errors, not panics or hangs
    let tap = tape_tap();
    assert!(trap_trial(&tap[..tap.len() - 1]).is_err());
    assert!(trap_trial(&[]).is_err());
    assert!(load_signal(&tap[..tap.len() - 1]).is_err());
    let mut t = tzx_header();
    t.extend_from_slice(&[0x7F, 0, 0, 0, 0]);
    let e = load_signal(&t).err().unwrap();
    assert!(e.contains("0x7F"), "{e}");
    let dir = tmp("bad");
    let p = dir.join("bad.z80");
    std::fs::write(&p, [0u8; 31]).unwrap();
    assert!(prepare(&p).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.z80` version 3 header (no pages) with hardware mode `hw`.
fn z80_v3_header(hw: u8) -> Vec<u8> {
    let mut z = vec![0u8; 32 + 54];
    z[30..32].copy_from_slice(&54u16.to_le_bytes());
    z[34] = hw;
    z
}

#[test]
fn is_128k_detects() {
    assert!(is_128k(&z80_v3_header(4), "z80"));
    assert!(!is_128k(&z80_v3_header(0), "z80"));
    assert!(is_128k(&vec![0u8; 131_103], "sna"));
    assert!(is_128k(&vec![0u8; 131_103], "SNA"));
    assert!(!is_128k(&tune_sna(), "sna"));
    // a version 2 header: hardware mode 3 is a 128K
    let mut v2 = vec![0u8; 32 + 23];
    v2[30..32].copy_from_slice(&23u16.to_le_bytes());
    v2[34] = 3;
    assert!(is_128k(&v2, "z80"));
    // a TZX hardware type block: "does not run on a 48K", or "uses the 128K" with no 48K entry
    let tzx_hw = |entries: &[[u8; 3]]| {
        let mut t = tzx_header();
        t.extend_from_slice(&[0x33, entries.len() as u8]);
        for e in entries {
            t.extend_from_slice(e);
        }
        t.extend_from_slice(&wrap_tap_as_tzx(&tape_tap())[10..]);
        t
    };
    assert!(is_128k(&tzx_hw(&[[0, 1, 3]]), "tzx"));
    assert!(is_128k(&tzx_hw(&[[0, 3, 1]]), "tzx"));
    assert!(!is_128k(&tzx_hw(&[[0, 1, 0], [0, 3, 1]]), "tzx"));
    assert!(!is_128k(&tzx_hw(&[[0, 0, 3], [0, 1, 2]]), "tzx"));
    assert!(!is_128k(&wrap_tap_as_tzx(&tape_tap()), "tzx"));

    let dir = tmp("128");
    let p = dir.join("big.z80");
    std::fs::write(&p, z80_v3_header(4)).unwrap();
    assert_eq!(prepare(&p).unwrap_err(), "128K, left out");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn music_tzx_reexport() {
    assert!(music::tzx::to_tap(&wrap_tap_as_tzx(&tape_tap())).expect("converts") == tape_tap());
}
