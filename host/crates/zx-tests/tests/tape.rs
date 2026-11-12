// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_sys::ZX_ERR_FORMAT;
use zx_tests::*;

/// Calls LD-BYTES (0x0556) from a stub at 0x8000 and returns the registers
/// after it has returned (the stub then spins on `JR $`).
fn call_ld_bytes(m: &mut Machine, flag: u8, dest: u16, len: u16, load: bool) -> zx_regs {
    load_code(m);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.af = ((flag as u16) << 8) | u16::from(load);
    r.ix = dest;
    r.de = len;
    m.set_regs(&r);
    m.run_frame();
    let r = m.regs();
    assert_eq!(r.pc, 0x8003, "LD-BYTES returned to the caller");
    r
}

fn load_code(m: &mut Machine) {
    // CALL 0556h; JR $
    load(m, 0x8000, &[0xCD, 0x56, 0x05, 0x18, 0xFE]);
}

fn carry(r: &zx_regs) -> bool {
    r.af & 1 != 0
}

fn bytes(m: &Machine, addr: u16, n: u16) -> Vec<u8> {
    (0..n).map(|i| m.peek(addr + i)).collect()
}

#[test]
fn autostart_loads_basic_and_code() {
    let code = [0x3E, 0x5A, 0x32, 0x00, 0x40, 0x18, 0xFE]; // LD A,5Ah; LD (4000h),A; JR $
    let mut line = vec![0xEF, 0x22, 0x22, 0xAF, 0x3A, 0xF9, 0xC0];
    line.extend_from_slice(b"32768");
    line.extend_from_slice(&[0x0E, 0x00, 0x00, 0x00, 0x80, 0x00]);
    let prog = basic_line(10, &line);
    let mut tap = tap_header(0, "boot", prog.len() as u16, 10, prog.len() as u16);
    tap.extend(tap_block(0xFF, &prog));
    tap.extend(tap_header(3, "code", code.len() as u16, 32768, 0));
    tap.extend(tap_block(0xFF, &code));

    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();
    m.tape_autostart();
    let mut frames = 0;
    while m.peek(0x4000) != 0x5A {
        assert!(frames < 600, "program did not run within 600 frames; PC={:04X}", m.regs().pc);
        m.run_frame();
        frames += 1;
    }
    for _ in 0..3 {
        m.run_frame();
    }
    let pc = m.regs().pc;
    assert!((0x8005..=0x8006).contains(&pc), "PC at the JR $ loop, got {pc:04X}");
}

#[test]
fn bad_checksum_and_end_of_tape() {
    let mut bad = tap_block(0xFF, &[1, 2, 3, 4]);
    let n = bad.len();
    bad[n - 1] ^= 0x55;
    let mut tap = bad;
    tap.extend(tap_block(0xFF, &[9, 8, 7, 6]));
    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();

    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 4, true);
    assert!(!carry(&r), "bad checksum fails");
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 4, true);
    assert!(carry(&r), "the next call loads the following block");
    assert_eq!(bytes(&m, 0x9000, 4), [9, 8, 7, 6]);
    assert_eq!((r.ix, r.de), (0x9004, 0));

    let r = call_ld_bytes(&mut m, 0xFF, 0x9100, 4, true);
    assert!(!carry(&r), "end of tape fails");
    let r = call_ld_bytes(&mut m, 0xFF, 0x9100, 4, true);
    assert!(!carry(&r), "and keeps failing without consuming anything");
    for _ in 0..3 {
        m.run_frame();
    }
    assert_eq!(m.regs().pc, 0x8003);
}

#[test]
fn flag_mismatch_skips_block() {
    let mut tap = tap_block(0x00, &[1, 2, 3]);
    tap.extend(tap_block(0xFF, &[4, 5, 6]));
    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 3, true);
    assert!(!carry(&r), "flag 0x00 block does not satisfy flag 0xFF");
    assert_eq!(bytes(&m, 0x9000, 3), [0, 0, 0], "nothing written on mismatch");
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 3, true);
    assert!(carry(&r));
    assert_eq!(bytes(&m, 0x9000, 3), [4, 5, 6]);
}

#[test]
fn short_block_fails_and_long_block_loads_de_bytes() {
    let mut tap = tap_block(0xFF, &[1, 2]);
    tap.extend(tap_block(0xFF, &[1, 2, 3, 4, 5]));
    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 4, true);
    assert!(!carry(&r), "block shorter than DE fails");
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 3, true);
    assert!(carry(&r));
    assert_eq!(bytes(&m, 0x9000, 4), [1, 2, 3, 0]);
    assert_eq!((r.ix, r.de), (0x9003, 0));
}

#[test]
fn verify_mode_does_not_write() {
    let mut tap = tap_block(0xFF, &[1, 2, 3]);
    tap.extend(tap_block(0xFF, &[1, 2, 3]));
    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();
    for (i, b) in [1u8, 9, 3].iter().enumerate() {
        m.poke(0x9000 + i as u16, *b);
    }
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 3, false);
    assert!(!carry(&r), "VERIFY of differing memory fails");
    assert_eq!(bytes(&m, 0x9000, 3), [1, 9, 3], "and writes nothing");
    m.poke(0x9001, 2);
    let r = call_ld_bytes(&mut m, 0xFF, 0x9000, 3, false);
    assert!(carry(&r), "VERIFY of equal memory succeeds");
    assert_eq!((r.ix, r.de), (0x9003, 0));
    assert_eq!(bytes(&m, 0x9000, 3), [1, 2, 3]);
}

#[test]
fn no_tape_runs_the_rom_routine() {
    let mut m = Machine::new();
    load_code(&mut m);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.af = 0xFF01;
    r.ix = 0x9000;
    r.de = 4;
    m.set_regs(&r);
    m.run_frame();
    let pc = m.regs().pc;
    assert!((0x0556..0x0600).contains(&pc), "still inside LD-BYTES waiting for an edge: {pc:04X}");
}

#[test]
fn ld_h_h_still_executes_with_a_tape_inserted() {
    // 0x64 doubles as the CPU library's hook opcode; outside LD-BYTES it is a plain LD H,H.
    let mut m = Machine::new();
    m.tape_insert(&tap_block(0xFF, &[1])).unwrap();
    load(&mut m, 0x8000, &[0x64, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.hl = 0x1234;
    r.af = 0x00FF;
    m.set_regs(&r);
    m.run_frame();
    let r = m.regs();
    assert_eq!((r.pc, r.hl, r.af), (0x8001, 0x1234, 0x00FF));
}

#[test]
fn rejects_malformed_tap() {
    let mut m = Machine::new();
    let mut tap = tap_block(0xFF, &[1, 2, 3]);
    tap.extend_from_slice(&[10, 0, 0xFF, 1]); // claims 10 bytes, has 2
    assert_eq!(m.tape_insert(&tap), Err(ZX_ERR_FORMAT));
    assert_eq!(m.tape_insert(&[5]), Err(ZX_ERR_FORMAT), "half a length word");
    assert_eq!(m.tape_insert(&tap_block(0xFF, &[1])[..4]), Err(ZX_ERR_FORMAT));
    assert!(m.tape_insert(&[]).is_ok(), "an empty tape is fine");
}

#[test]
fn z80test_full_passes() {
    // CARGO_MANIFEST_DIR is host/crates/zx-tests.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../build/test-data/z80full.tap");
    let tap = std::fs::read(path).unwrap_or_else(|_| panic!("{path} is missing: run tools/fetch-test-data"));
    let mut m = Machine::new();
    m.tape_insert(&tap).unwrap();
    m.tape_autostart();

    // Run until z80test prints its "Result:" line, checked every 50 frames, or
    // 60,000 frames pass. (The brief's "screen still for 50 frames" stop does
    // not work: one test runs for longer than that without printing.)
    let start = std::time::Instant::now();
    // The output outgrows the screen, so the ROM asks "scroll?" and waits
    // for a key: press ENTER (row 6, bit 0) whenever that shows.
    let (mut frames, mut pressed) = (0u32, false);
    while frames < 60_000 {
        m.run_frame();
        frames += 1;
        if frames % 4 == 0 {
            let scroll = (22..24).any(|r| screen_text(&m, r).contains("scroll?"));
            if scroll || pressed {
                m.set_keys([0, 0, 0, 0, 0, 0, u8::from(scroll), 0]);
            }
            pressed = scroll;
        }
        if frames % 50 == 0 && (0..24).any(|r| screen_text(&m, r).starts_with("Result:")) {
            break;
        }
    }
    let text: Vec<String> = (0..24).map(|r| screen_text(&m, r)).collect();
    eprintln!("z80full: {frames} frames in {:?}\n{}", start.elapsed(), text.join("\n"));
    assert!(
        text.iter().any(|l| l.contains("Result: all tests passed.")),
        "z80full did not report success after {frames} frames:\n{}",
        text.join("\n")
    );
}

#[test]
fn eject_restores_the_rom_routine() {
    for by_empty_insert in [false, true] {
        let mut m = Machine::new();
        m.tape_insert(&tap_block(0xFF, &[1, 2, 3, 4])).unwrap();
        if by_empty_insert {
            m.tape_insert(&[]).unwrap();
        } else {
            m.tape_eject();
        }
        load_code(&mut m);
        let mut r = m.regs();
        r.pc = 0x8000;
        r.sp = 0xFF00;
        r.af = 0xFF01;
        r.ix = 0x9000;
        r.de = 4;
        m.set_regs(&r);
        m.run_frame();
        let pc = m.regs().pc;
        assert!((0x0556..0x0600).contains(&pc), "real ROM routine runs, no trap: {pc:04X}");
    }
}
