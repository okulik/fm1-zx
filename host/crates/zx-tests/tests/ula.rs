// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

const IN_A_BC: [u8; 2] = [0xED, 0x78]; // I/O cycle starts 8 T into the instruction
const OUT_FE_A: [u8; 2] = [0xD3, 0xFE]; // I/O cycle starts 7 T into the instruction
const IN_FF: [u8; 2] = [0xDB, 0xFF]; // IN A,(FF): I/O cycle starts 7 T in

/// Runs OUT (0xFE),A with A = `v` so that its I/O cycle starts at `t`.
fn out_fe(m: &mut Machine, v: u8, t: u32) {
    run_at(m, &OUT_FE_A, v, 0, 0, t - 7);
}

#[test]
fn keyboard_half_rows() {
    let mut m = Machine::new();
    let mut rows = [0u8; 8];
    rows[1] = 0b00001; // A
    m.set_keys(rows);
    run_at(&mut m, &IN_A_BC, 0, 0xFD, 0xFE, 100);
    assert_eq!(reg_a(&m) & 1, 0);
    assert_eq!(reg_a(&m) & 0xA0, 0xA0);
    assert_eq!(reg_a(&m) & 0x1F, 0x1E);
    run_at(&mut m, &IN_A_BC, 0, 0xFE, 0xFE, 100);
    assert_eq!(reg_a(&m) & 1, 1);
    run_at(&mut m, &IN_A_BC, 0, 0x00, 0xFE, 100);
    assert_eq!(reg_a(&m) & 1, 0);
}

#[test]
fn keyboard_ands_all_selected_rows() {
    let mut m = Machine::new();
    m.set_keys([0b00001, 0b00010, 0, 0, 0, 0, 0, 0b10000]);
    run_at(&mut m, &IN_A_BC, 0, 0xFC, 0xFE, 100); // rows 0 and 1
    assert_eq!(reg_a(&m) & 0x1F, 0b11100);
    run_at(&mut m, &IN_A_BC, 0, 0x7F, 0xFE, 100); // row 7
    assert_eq!(reg_a(&m) & 0x1F, 0b01111);
    run_at(&mut m, &IN_A_BC, 0, 0xFF, 0xFE, 100); // no row
    assert_eq!(reg_a(&m) & 0x1F, 0x1F);
}

#[test]
fn ear_bit_follows_last_out() {
    let mut m = Machine::new();
    out_fe(&mut m, 0x10, 100);
    run_at(&mut m, &IN_A_BC, 0, 0xFF, 0xFE, 200);
    assert_eq!(reg_a(&m) & 0x40, 0x40);
    out_fe(&mut m, 0x00, 300);
    run_at(&mut m, &IN_A_BC, 0, 0xFF, 0xFE, 400);
    assert_eq!(reg_a(&m) & 0x40, 0);
}

#[test]
fn kempston() {
    let mut m = Machine::new();
    m.set_kempston(0b10001);
    run_at(&mut m, &IN_A_BC, 0, 0, 0x1F, 100);
    assert_eq!(reg_a(&m), 0x11);
    m.set_kempston(0);
    run_at(&mut m, &IN_A_BC, 0, 0, 0x1F, 100);
    assert_eq!(reg_a(&m), 0x00);
}

#[test]
fn border_per_line() {
    let mut m = Machine::new();
    out_fe(&mut m, 7, 10);
    out_fe(&mut m, 2, 1000); // ceil(1000 / 224) = 5
    assert_eq!(m.border_line(4), 7);
    assert_eq!(m.border_line(5), 2);
    assert_eq!(m.border_line(311), 2);
    // exactly on a line start: effective from that line
    out_fe(&mut m, 4, 7 * 224);
    assert_eq!(m.border_line(6), 2);
    assert_eq!(m.border_line(7), 4);
    let mut m = Machine::new();
    out_fe(&mut m, 2, 5 * 224);
    assert_eq!(m.border_line(4), 7);
    assert_eq!(m.border_line(5), 2);
}

#[test]
fn border_carries_over_and_resets_each_frame() {
    let mut m = Machine::new();
    // OUT (FE),A at T=0 of the frame sets border 3; then idle.
    load(&mut m, 0x8000, &[0x3E, 0x03, 0xD3, 0xFE, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.iff1 = 0;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.border_line(0), 7);
    assert_eq!(m.border_line(1), 3);
    m.run_frame();
    // new frame: every line starts with the colour in effect at the end of the last
    assert_eq!(m.border_line(0), 3);
    assert_eq!(m.border_line(311), 3);
}

#[test]
fn beeper_edges_have_tstates() {
    let mut m = Machine::new();
    out_fe(&mut m, 0x10, 1000);
    out_fe(&mut m, 0x10, 1500); // same level: no edge
    out_fe(&mut m, 0x00, 2000);
    let e = m.edges();
    assert_eq!(e.len(), 2);
    assert!(e[0].tstate.abs_diff(1000) <= 1 && e[0].level == 2);
    assert!(e[1].tstate.abs_diff(2000) <= 1 && e[1].level == 0);
}

#[test]
fn mic_bit_is_level_bit_zero() {
    let mut m = Machine::new();
    out_fe(&mut m, 0x08, 100);
    assert_eq!(m.edges()[0].level, 1);
    assert_eq!(m.test_level(), 1);
}

#[test]
fn edge_overflow_keeps_final_level() {
    let mut m = Machine::new();
    // OUT (C),B / OUT (C),D alternating, 5001 toggles of 12 T each, from level 0.
    let mut code = Vec::new();
    for i in 0..5001 {
        code.extend_from_slice(if i % 2 == 0 { &[0xED, 0x41] } else { &[0xED, 0x51] });
    }
    code.extend_from_slice(&[0x18, 0xFE]);
    load(&mut m, 0x8000, &code);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.bc = 0x10FE; // B = 0x10 (EAR high), C = port
    r.de = 0x0000;
    r.sp = 0xFF00;
    r.iff1 = 0;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.edges().len(), 4096);
    assert_eq!(m.test_level(), 2); // the 5001st OUT writes B
    assert_eq!(m.edges()[4095].level, 0);
}

#[test]
fn changes_past_the_frame_end_belong_to_the_next_frame() {
    let mut m = Machine::new();
    // OUT (FE),A with A = 0x12 (border 2, EAR) whose I/O cycle starts at T = 69892.
    load(&mut m, 0x8000, &OUT_FE_A);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.af = 0x1200;
    r.iff1 = 0;
    m.set_regs(&r);
    m.test_set_tstate(69885);
    m.run_frame();
    assert!(m.edges().is_empty());
    assert_eq!(m.border_line(311), 7);
    assert_eq!(m.test_level(), 2);
    m.run_frame();
    assert_eq!(m.edges().len(), 1);
    assert_eq!(m.edges()[0].tstate, 4);
    assert_eq!(m.edges()[0].level, 2);
    assert_eq!(m.border_line(0), 7);
    assert_eq!(m.border_line(1), 2);
}

// Floating bus. With t = T - 14335 (the contention origin), paper line y is
// t / 224 and x = t % 224; for x < 128 the ULA does, per 8 T, bitmap(2g),
// attr(2g), bitmap(2g+1), attr(2g+1), then 4 idle, with g = x / 8. Contention
// holds a CPU access in phases 0-5 until phase 6, where the bus goes idle, so
// phases 0-3 are the four fetches.
fn float_at(m: &mut Machine, t: u32) -> u8 {
    run_at(m, &IN_FF, 0, 0, 0, t - 7);
    reg_a(m)
}

#[test]
fn floating_bus() {
    let mut m = Machine::new();
    for i in 0..0x1800u16 {
        m.poke(0x4000 + i, (i % 251) as u8 | 1);
    }
    for i in 0..0x300u16 {
        m.poke(0x5800 + i, (i % 241) as u8 ^ 0x80);
    }
    let bitmap = |y: u16, col: u16| 0x4000 + ((y & 0xC0) << 5) + ((y & 7) << 8) + ((y & 0x38) << 2) + col;
    let attr = |y: u16, col: u16| 0x5800 + (y >> 3) * 32 + col;
    for &(y, g) in &[(0u16, 0u16), (0, 5), (1, 0), (7, 15), (8, 3), (63, 9), (64, 2), (191, 15)] {
        let base = 14335 + y as u32 * 224 + g as u32 * 8;
        let (c0, c1) = (2 * g, 2 * g + 1);
        assert_eq!(float_at(&mut m, base), m.peek(bitmap(y, c0)), "y={y} g={g} bitmap");
        assert_eq!(float_at(&mut m, base + 1), m.peek(attr(y, c0)), "attr");
        assert_eq!(float_at(&mut m, base + 2), m.peek(bitmap(y, c1)), "bitmap+1");
        assert_eq!(float_at(&mut m, base + 3), m.peek(attr(y, c1)), "attr+1");
        for k in 4..8 {
            assert_eq!(float_at(&mut m, base + k), 0xFF, "idle {k}");
        }
    }
    // outside the paper and the 128 T window
    assert_eq!(float_at(&mut m, 14335 + 128), 0xFF);
    assert_eq!(float_at(&mut m, 14335 + 223), 0xFF);
    assert_eq!(float_at(&mut m, 14334), 0xFF);
    assert_eq!(float_at(&mut m, 100), 0xFF);
    assert_eq!(float_at(&mut m, 14335 + 192 * 224), 0xFF);
    assert_eq!(float_at(&mut m, 69000), 0xFF);
}

/// `EI; HALT; LD A,(A001h); INC A; LD (A001h),A; JR 0x8000` under IM `im`,
/// with an IM 2 handler at 0x9000 that counts into 0xA000. IM 1 runs the ROM's
/// own handler at 0x38 (it counts FRAMES at 0x5C78 and needs IY = ERR_NR).
fn halt_loop(im: u8) -> Machine {
    let mut m = Machine::new();
    load(&mut m, 0x80FF, &[0x00, 0x90]); // IM 2 vector word for bus value 0xFF
    // LD A,(A000h); INC A; LD (A000h),A; EI; RETI
    load(&mut m, 0x9000, &[0x3A, 0x00, 0xA0, 0x3C, 0x32, 0x00, 0xA0, 0xFB, 0xED, 0x4D]);
    load(&mut m, 0x8000, &[0xFB, 0x76, 0x3A, 0x01, 0xA0, 0x3C, 0x32, 0x01, 0xA0, 0x18, 0xF5]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFFF0;
    r.i = 0x80;
    r.iy = 0x5C3A;
    r.im = im;
    m.set_regs(&r);
    m
}

#[test]
fn one_interrupt_per_frame_with_ei_halt() {
    for im in [1u8, 2] {
        let mut m = halt_loop(im);
        for f in 1..=5u32 {
            m.run_frame();
            // the main loop gets past the HALT once per frame, so PC returned to just after it
            assert_eq!(m.peek(0xA001) as u32, f, "IM {im}, frame {f}: HALT left once");
            if im == 2 {
                assert_eq!(m.peek(0xA000) as u32, f, "IM 2 handler runs once per frame");
            } else {
                assert_eq!(m.peek(0x5C78) as u32, f, "IM 1: the ROM handler bumps FRAMES once per frame");
            }
            assert_eq!(m.regs().pc, 0x8002, "parked at the HALT's successor loop head");
        }
    }
}

/// Renders the machine's current edge list the way a firmware would.
fn render_frame(m: &Machine, b: &mut Beeper) {
    b.set_level(m.edges_start_level());
    b.render(m.edges());
}

#[test]
fn renderer_level_resyncs_after_edge_overflow() {
    let mut m = Machine::new();
    let mut b = Beeper::new(48_000, 1);
    // 5001 toggles overflow the list; the dropped ones leave a renderer one level behind.
    let mut code = Vec::new();
    for i in 0..5001 {
        code.extend_from_slice(if i % 2 == 0 { &[0xED, 0x41] } else { &[0xED, 0x51] });
    }
    // then spin with no more output
    code.extend_from_slice(&[0x18, 0xFE]);
    load(&mut m, 0x8000, &code);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.bc = 0x10FE;
    r.sp = 0xFF00;
    r.iff1 = 0;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.edges_start_level(), 0);
    render_frame(&m, &mut b);
    assert_ne!(b.test_level(), m.test_level(), "the overflow left the renderer behind");
    m.run_frame();
    assert_eq!(m.edges_start_level(), m.test_level(), "no edges: the list starts at the machine's level");
    render_frame(&m, &mut b);
    assert_eq!(b.test_level(), m.test_level());
}

#[test]
fn renderer_level_resyncs_after_a_snapshot_load() {
    let mut m = Machine::new();
    let mut b = Beeper::new(48_000, 1);
    run_at(&mut m, &[0xD3, 0xFE], 0x10, 0, 0, 100);
    assert_eq!(m.test_level(), 2);
    b.set_level(2); // the renderer follows
    let regs = zx_regs { pc: 0x8000, sp: 0xFF00, iff1: 0, iff2: 0, ..sample_regs() };
    let mut ram = sample_ram();
    ram[0x4000..0x4003].copy_from_slice(&[0xF3, 0x18, 0xFE]);
    m.load_sna(&make_sna(&regs, &ram, 1)).unwrap();
    assert_eq!(m.test_level(), 0, "the load resets the beeper level");
    m.run_frame();
    render_frame(&m, &mut b);
    assert_eq!(b.test_level(), m.test_level());
    assert_eq!(b.test_level(), 0);
}

#[test]
fn start_level_follows_edges_across_frames() {
    let mut m = Machine::new();
    // OUT (FE),A with A = 0x10 near the end of frame 1, then idle
    load(&mut m, 0x8000, &[0x3E, 0x10, 0xD3, 0xFE, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.iff1 = 0;
    m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.edges_start_level(), 0);
    assert_eq!(m.edges().len(), 1);
    m.run_frame();
    assert_eq!(m.edges_start_level(), 2);
    assert!(m.edges().is_empty());
}
