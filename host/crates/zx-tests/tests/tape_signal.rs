// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The tape signal player (core/zx/tapesig.c): the pulses each TZX block plays, the files it
//! refuses, port FEh with and without a signal, and a standard tape loaded by the ROM from the
//! signal against the same tape through the LD-BYTES trap.

use fmzx_pack::testprogs::{
    tape_tap, tzx_header, tzx_loop, tzx_pause, tzx_pulses, tzx_pure_data, tzx_standard, tzx_tone, tzx_turbo,
    wrap_tap_as_tzx, TzxTurbo, TAPE_DONE_BORDER,
};
use zx_sys::test_tape_pulses;
use zx_tests::*;

const PILOT: u32 = 2168;
const SYNC1: u32 = 667;
const SYNC2: u32 = 735;
const ZERO: u32 = 855;
const ONE: u32 = 1710;
const MS: u32 = 3500;

/// Two pulses per bit, MSB first; only the top `last_bits` bits of the last byte.
fn bit_pulses(data: &[u8], zero: u32, one: u32, last_bits: u8) -> Vec<u32> {
    let mut v = Vec::new();
    for (i, &b) in data.iter().enumerate() {
        let n = if i + 1 == data.len() { last_bits } else { 8 };
        for k in 0..n {
            let p = if b & (0x80 >> k) != 0 { one } else { zero };
            v.extend([p, p]);
        }
    }
    v
}

/// The pulses of a standard block (flag, data, checksum) and its pause.
fn standard_pulses(block: &[u8], pause_ms: u32) -> Vec<u32> {
    let pilots = if block[0] < 128 { 8063 } else { 3223 };
    let mut v = vec![PILOT; pilots];
    v.extend([SYNC1, SYNC2]);
    v.extend(bit_pulses(block, ZERO, ONE, 8));
    v.push(pause_ms * MS);
    v
}

/// The `.tap`'s blocks without their lengths.
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

fn tzx(blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut d = tzx_header();
    for b in blocks {
        d.extend_from_slice(b);
    }
    d
}

fn all_pulses(data: &[u8]) -> Vec<u32> {
    test_tape_pulses(data, 1 << 20).expect("tape accepted")
}

#[test]
fn standard_block_pulses() {
    let tap = tape_tap();
    let blocks = tap_blocks(&tap);
    let got = all_pulses(&wrap_tap_as_tzx(&tap));

    // the header block: 8063 pilot pulses, the syncs, its bits, then the 1000 ms pause
    let header = standard_pulses(blocks[0], 1000);
    assert_eq!(&got[..8063], &[PILOT; 8063][..]);
    assert_eq!(got[8063..8065], [SYNC1, SYNC2]);
    assert_eq!(got[8065..8065 + 19 * 16], bit_pulses(blocks[0], ZERO, ONE, 8)[..]);
    assert_eq!(got[header.len() - 1], 3_500_000);
    assert_eq!(&got[..header.len()], &header[..]);
    // the data block after it: 3223 pilot pulses, then a sync
    assert_eq!(&got[header.len()..header.len() + 3223], &[PILOT; 3223][..]);
    assert_eq!(got[header.len() + 3223], SYNC1);

    let want: Vec<u32> = blocks.iter().flat_map(|b| standard_pulses(b, 1000)).collect();
    assert_eq!(got.len(), want.len());
    assert!(got == want, "the whole tape plays as standard blocks");

    // a .tap plays the same as its TZX wrapping
    assert!(all_pulses(&tap) == want, "a .tap block plays as a 0x10 block with a 1000 ms pause");
}

const TURBO: TzxTurbo =
    TzxTurbo { pilot: 1500, sync1: 400, sync2: 450, zero: 520, one: 1040, pilot_pulses: 2000, last_bits: 5, pause_ms: 0 };

#[test]
fn turbo_block_pulses() {
    let data = [0xA5, 0x3C, 0xF8];
    let got = all_pulses(&tzx(&[tzx_turbo(&TURBO, &data)]));
    let mut want = vec![1500; 2000];
    want.extend([400, 450]);
    want.extend(bit_pulses(&data, 520, 1040, 5));
    assert_eq!(want.len(), 2000 + 2 + 2 * (16 + 5));
    // pause 0 after a data block is no pause: nothing follows, the tape ends there
    assert!(got == want, "turbo block: {} pulses, want {}", got.len(), want.len());

    // a turbo block with pause 0 runs straight into the next block (no pause entry)...
    let got = all_pulses(&tzx(&[tzx_turbo(&TURBO, &data), tzx_tone(777, 3)]));
    let mut cont = want.clone();
    cont.extend([777, 777, 777]);
    assert!(got == cont, "pause 0 runs on into the next block");

    // ...while a 0x20 pause 0 is a stop, one entry of 0, and play goes on after it
    let got = all_pulses(&tzx(&[tzx_turbo(&TURBO, &data), tzx_pause(0), tzx_tone(777, 3)]));
    let mut stop = want.clone();
    stop.extend([0, 777, 777, 777]);
    assert!(got == stop, "a 0x20 pause 0 is a stop entry");

    // a turbo pause of 12 ms is one entry
    let t = TzxTurbo { pause_ms: 12, ..TURBO };
    let got = all_pulses(&tzx(&[tzx_turbo(&t, &data)]));
    assert_eq!(got.len(), want.len() + 1);
    assert_eq!(got[want.len()], 12 * MS);
}

#[test]
fn tone_seq_data_pulses() {
    assert_eq!(all_pulses(&tzx(&[tzx_tone(2000, 7)])), vec![2000; 7]);
    assert_eq!(all_pulses(&tzx(&[tzx_pulses(&[100, 200, 300])])), vec![100, 200, 300]);
    let data = [0x96, 0x40];
    let mut want = bit_pulses(&data, 600, 1200, 3);
    assert_eq!(all_pulses(&tzx(&[tzx_pure_data(600, 1200, 3, 0, &data)])), want);
    want.push(5 * MS);
    assert_eq!(all_pulses(&tzx(&[tzx_pure_data(600, 1200, 3, 5, &data)])), want);

    // all three in a row, with a 0x20 pause between them
    let got = all_pulses(&tzx(&[
        tzx_tone(2000, 7),
        tzx_pulses(&[100, 200, 300]),
        tzx_pause(2),
        tzx_pure_data(600, 1200, 8, 0, &[0x81]),
    ]));
    let mut want = vec![2000; 7];
    want.extend([100, 200, 300, 2 * MS]);
    want.extend(bit_pulses(&[0x81], 600, 1200, 8));
    assert_eq!(got, want);

    // a loop of 3 around a pure tone plays it three times
    let got = all_pulses(&tzx(&[tzx_loop(3, &tzx_tone(2000, 7)), tzx_pulses(&[50])]));
    let mut want = vec![2000; 21];
    want.push(50);
    assert_eq!(got, want);
}

#[test]
fn info_blocks_skipped() {
    let tap = tape_tap();
    let blocks = tap_blocks(&tap);
    let plain = tzx(&[tzx_standard(1000, blocks[0]), tzx_standard(1000, blocks[1])]);
    let group_start = [&[0x21, 4][..], b"boot"].concat();
    let group_end = vec![0x22];
    let text = [&[0x30, 5][..], b"hello"].concat();
    // archive info: length 9, one string (00h, title) of 6 bytes
    let archive = [&[0x32, 9, 0, 1, 0, 6][..], b"tapsig"].concat();
    let with_info = tzx(&[
        group_start,
        tzx_standard(1000, blocks[0]),
        text.clone(),
        archive.clone(),
        tzx_standard(1000, blocks[1]),
        group_end,
        text,
        archive,
    ]);
    let want = all_pulses(&plain);
    assert!(all_pulses(&with_info) == want, "info and group blocks change no pulse");
}

#[test]
fn bad_tzx_refused() {
    let turbo = tzx(&[tzx_turbo(&TURBO, &[1, 2, 3])]);
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("0x11 length past the end", turbo[..turbo.len() - 1].to_vec(), "0x11"),
        ("0x13 count 0", tzx(&[vec![0x13, 0]]), "0x13"),
        ("0x13 body cut short", tzx(&[vec![0x13, 3, 100, 0, 200, 0]]), "0x13"),
        ("block 0x15", tzx(&[vec![0x15, 79, 0, 0, 0, 0, 1, 0, 0, 0xFF]]), "0x15"),
        ("block 0x19", tzx(&[vec![0x19, 0, 0, 0, 0]]), "0x19"),
        ("id 0x7F", tzx(&[tzx_tone(2000, 7), vec![0x7F, 0, 0, 0, 0]]), "0x7F"),
        ("a 9-byte header", b"ZXTape!\x1a\x01".to_vec(), "header"),
        ("a .tap block past the end", vec![5, 0, 0xFF, 1], ".tap"),
    ];
    for (what, data, cause) in cases {
        let e = test_tape_pulses(&data, 100).expect_err(what);
        assert!(e.contains(cause), "{what}: {e:?} does not name {cause}");
        let mut m = Machine::new();
        let e2 = m.tape_signal_insert(&data).expect_err(what);
        assert_eq!(e, e2, "{what}");
        assert!(!m.tape_signal_done() && !m.tape_signal_stopped(), "{what}: nothing inserted");
        // the machine runs on as without a tape
        m.run_frame();
    }
}

#[test]
fn cut_and_corrupted_tapes_never_crash() {
    let rich = tzx(&[
        [&[0x30, 3][..], b"abc"].concat(),
        tzx_standard(5, &[0xFF, 1, 2, 0xFC]),
        tzx_turbo(&TzxTurbo { pause_ms: 1, ..TURBO }, &[0xA5, 0x3C]),
        tzx_loop(2, &[tzx_tone(300, 5), tzx_pulses(&[10, 20])].concat()),
        tzx_pause(0),
        tzx_pure_data(600, 1200, 4, 3, &[0x96, 0x40]),
        vec![0x22],
    ]);
    // every cut is refused or plays to its end; so does every one-byte change
    for n in 0..=rich.len() {
        let _ = test_tape_pulses(&rich[..n], 1 << 16);
    }
    let mut x = 0x1234_5678u32;
    for _ in 0..4000 {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut d = rich.clone();
        let at = 10 + (x >> 8) as usize % (d.len() - 10);
        d[at] = (x >> 24) as u8;
        if let Ok(p) = test_tape_pulses(&d, 1 << 22) {
            assert!(p.len() < 1 << 22, "a changed byte made the tape endless");
        }
    }
}

/// IN A,(C) with BC = FEFEh, its I/O cycle 8 T into the instruction.
const IN_A_C: [u8; 2] = [0xED, 0x78];
const OUT_FE_A: [u8; 2] = [0xD3, 0xFE];

fn port_fe_after_out(m: &mut Machine, out: u8) -> u8 {
    run_at(m, &OUT_FE_A, out, 0, 0, 1000);
    run_at(m, &IN_A_C, 0, 0xFE, 0xFE, 2000);
    reg_a(m)
}

#[test]
fn ear_unchanged_without_signal() {
    let mut m = Machine::new();
    assert_eq!(port_fe_after_out(&mut m, 0x10), 0xFF);
    assert_eq!(port_fe_after_out(&mut m, 0x00), 0xBF);
    assert_eq!(port_fe_after_out(&mut m, 0x17), 0xFF);
    assert_eq!(port_fe_after_out(&mut m, 0x07), 0xBF);

    // after a signal is inserted and ejected, the same again
    m.tape_signal_insert(&wrap_tap_as_tzx(&tape_tap())).unwrap();
    m.tape_signal_eject();
    assert_eq!(port_fe_after_out(&mut m, 0x10), 0xFF);
    assert_eq!(port_fe_after_out(&mut m, 0x00), 0xBF);

    // a snapshot load ejects the signal: its 2-frame tone never ends
    m.tape_signal_insert(&tzx(&[tzx_tone(1500, 100)])).unwrap();
    m.load_sna(&make_sna(&sample_regs(), &sample_ram(), 7)).unwrap();
    for _ in 0..5 {
        m.run_frame();
    }
    assert!(!m.tape_signal_done(), "ejected by the snapshot load");
    assert_eq!(port_fe_after_out(&mut m, 0x00), 0xBF);
}

#[test]
fn stop_holds_until_play() {
    // 4 pulses, a stop, 4 pulses: well inside one frame each
    let mut m = Machine::new();
    m.tape_signal_insert(&tzx(&[tzx_tone(1000, 4), tzx_pause(0), tzx_tone(1000, 4)])).unwrap();
    assert!(!m.tape_signal_stopped() && !m.tape_signal_done());
    m.run_frame();
    assert!(m.tape_signal_stopped(), "held by the stop");
    assert!(!m.tape_signal_done());
    m.run_frame();
    assert!(m.tape_signal_stopped(), "still held a frame later");
    assert_eq!(port_fe_after_out(&mut m, 0x10) & 0x40, 0, "the level is low while stopped");
    m.tape_signal_play();
    assert!(!m.tape_signal_stopped());
    m.run_frame();
    assert!(m.tape_signal_done(), "the rest played after play");
    assert!(!m.tape_signal_stopped());
}

/// A header block's type, length and first parameter (a CODE block's address).
fn header_fields(block: &[u8]) -> (u8, u16, u16) {
    let w = |i: usize| u16::from_le_bytes([block[i], block[i + 1]]);
    (block[1], w(12), w(14))
}

fn ram(m: &Machine, addr: u16, len: u16) -> Vec<u8> {
    (0..len).map(|i| m.peek(addr + i)).collect()
}

#[test]
fn standard_tape_via_signal_matches_trap() {
    let tap = tape_tap();
    let blocks = tap_blocks(&tap);

    let mut trap = Machine::new();
    trap.tape_insert(&tap).unwrap();
    trap.tape_autostart();
    let mut frames = 0;
    while trap.border() != TAPE_DONE_BORDER {
        assert!(frames < 1000, "the trap load did not finish");
        trap.run_frame();
        frames += 1;
    }

    // The signal: the .tap's blocks as standard blocks with its 1000 ms pauses, except after
    // the code, where the gap is 3000 ms. The code waits TAPE_WAIT_FRAMES (2 s) before its
    // LD-BYTES call, and LD-BYTES waits 1 s after the first edge it sees before it times the
    // pilot: behind a 1 s gap the data block's 2 s pilot would be over by then, on a cassette
    // as here. With 3 s the call is waiting when the pilot starts.
    let wrapped: Vec<Vec<u8>> = blocks.iter().map(|b| tzx_standard(1000, b)).collect();
    assert_eq!(wrap_tap_as_tzx(&tap), tzx(&wrapped));
    let gapped: Vec<Vec<u8>> =
        blocks.iter().enumerate().map(|(i, b)| tzx_standard(if i == 3 { 3000 } else { 1000 }, b)).collect();
    let mut sig = Machine::new();
    sig.tape_signal_insert(&tzx(&gapped)).unwrap();
    sig.tape_autostart();
    let mut frames = 0;
    while !sig.tape_signal_done() {
        assert!(frames < 4000, "the signal did not reach its end; PC={:04X}", sig.regs().pc);
        sig.run_frame();
        frames += 1;
    }
    for _ in 0..100 {
        sig.run_frame();
    }
    assert!(!sig.tape_signal_stopped());

    // where the blocks load: the program at PROG, the code at its header's address, and the
    // headerless screen image at 4000h (the code's LD-BYTES call)
    let prog = u16::from_le_bytes([trap.peek(0x5C53), trap.peek(0x5C54)]);
    let (kind, prog_len, _) = header_fields(blocks[0]);
    assert_eq!(kind, 0);
    let (kind, code_len, code_at) = header_fields(blocks[2]);
    assert_eq!(kind, 3);
    let ranges = [(prog, prog_len, blocks[1]), (code_at, code_len, blocks[3]), (0x4000, 6912, blocks[4])];
    for (at, len, block) in ranges {
        assert_eq!(ram(&trap, at, len), block[1..1 + len as usize], "trap: {at:04X}");
        assert_eq!(ram(&sig, at, len), ram(&trap, at, len), "signal: {at:04X}");
    }
    assert_eq!(trap.border(), TAPE_DONE_BORDER);
    assert_eq!(sig.border(), TAPE_DONE_BORDER);
}

