// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_sys::{ZX_ERR_FORMAT, ZX_ERR_UNSUPPORTED};
use zx_tests::*;

fn ram_of(m: &Machine) -> Vec<u8> {
    (0x4000..=0xFFFFu32).map(|a| m.peek(a as u16)).collect()
}

fn dirty_of(m: &Machine) -> Vec<bool> {
    (0..192).map(|y| m.line_dirty(y)).collect()
}

/// Machine in a distinctive state, frames run so no line is dirty.
fn dirty_machine() -> Machine {
    let mut m = Machine::new();
    m.poke(0x8000, 0xA5);
    m.poke(0x4000, 0x5A);
    m.run_frame();
    m.set_regs(&zx_regs { pc: 0x9000, sp: 0xFE00, im: 1, i: 0x11, ..sample_regs() });
    m
}

/// A loaded machine must show the sample registers and RAM.
fn assert_loaded(m: &Machine, regs: &zx_regs, ram: &[u8]) {
    assert_eq!(format!("{:?}", m.regs()), format!("{:?}", regs));
    assert_eq!(ram_of(m), ram);
    assert!((0..192).all(|y| m.line_dirty(y)), "every line dirty after a load");
    assert_eq!(m.frame_tstate(), 0);
}

#[test]
fn sna_round_trip() {
    let regs = sample_regs();
    let ram = sample_ram();
    let mut m = Machine::new();
    m.load_sna(&make_sna(&regs, &ram, 3)).unwrap();
    let mut want = ram.clone();
    // make_sna pushed PC onto the stack; the pushed bytes stay in RAM
    want[0xFEFE - 0x4000] = regs.pc as u8;
    want[0xFEFF - 0x4000] = (regs.pc >> 8) as u8;
    assert_loaded(&m, &regs, &want);
    // one frame with no OUT keeps the border (the code under SP is data, so
    // run it from a HALT with interrupts off)
    let mut r = m.regs();
    r.pc = 0x8000;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.poke(0x8000, 0x76);
    m.run_frame();
    assert_eq!(m.border_line(0), 3);
    assert_eq!(m.border_line(311), 3);
    assert!(m.edges().is_empty());
}

#[test]
fn sna_iff_clear_and_im0() {
    let regs = zx_regs { iff1: 0, iff2: 0, im: 0, ..sample_regs() };
    let mut m = Machine::new();
    m.load_sna(&make_sna(&regs, &sample_ram(), 5)).unwrap();
    let r = m.regs();
    assert_eq!((r.iff1, r.iff2, r.im), (0, 0, 0));
    assert_eq!(m.border_line(0), 5);
}

#[test]
fn z80_v1_compressed_and_plain() {
    let regs = sample_regs();
    let ram = sample_ram();
    let mut a = Machine::new();
    let mut b = Machine::new();
    let plain = make_z80_v1(&regs, &ram, false);
    let packed = make_z80_v1(&regs, &ram, true);
    assert!(packed.len() < plain.len());
    a.load_z80(&plain).unwrap();
    b.load_z80(&packed).unwrap();
    assert_loaded(&a, &regs, &ram);
    assert_loaded(&b, &regs, &ram);
    assert_eq!(a.border_line(0), 3);
    a.run_frame();
    assert_eq!(a.border_line(0), 3);
}

#[test]
fn z80_v3_48k_pages() {
    let regs = sample_regs();
    let ram = sample_ram();
    for (version, modes) in [(2u8, vec![0u8, 1]), (3, vec![0, 1, 3])] {
        for mode in modes {
            let mut m = Machine::new();
            m.load_z80(&make_z80_vn(&regs, &ram, version, mode))
                .unwrap_or_else(|e| panic!("v{version} mode {mode}: {e}"));
            assert_loaded(&m, &regs, &ram);
            assert_eq!(m.border_line(0), 3);
        }
    }
}

#[test]
fn z80_r_bit7_and_iff() {
    let regs = zx_regs { r: 0x80, iff1: 0, iff2: 1, im: 1, ..sample_regs() };
    let mut m = Machine::new();
    m.load_z80(&make_z80_v3(&regs, &sample_ram(), 0)).unwrap();
    assert_eq!(format!("{:?}", m.regs()), format!("{:?}", regs));
}

#[test]
fn rejects_128k() {
    let regs = sample_regs();
    let ram = sample_ram();
    let mut m = dirty_machine();
    let before = (format!("{:?}", m.regs()), ram_of(&m), dirty_of(&m), m.border_line(0));
    for (version, mode) in [(2u8, 2u8), (2, 3), (2, 4), (3, 2), (3, 4), (3, 5), (3, 6), (3, 9)] {
        assert_eq!(m.load_z80(&make_z80_vn(&regs, &ram, version, mode)), Err(ZX_ERR_UNSUPPORTED), "v{version} mode {mode}");
    }
    // "modify hardware" flag in a v3
    let mut f = make_z80_v3(&regs, &ram, 0);
    f[37] |= 0x80;
    assert_eq!(m.load_z80(&f), Err(ZX_ERR_UNSUPPORTED));
    // 128K .sna sizes
    let mut big = make_sna(&regs, &ram, 3);
    big.extend(vec![0u8; 131103 - big.len()]);
    assert_eq!(m.load_sna(&big), Err(ZX_ERR_UNSUPPORTED));
    let after = (format!("{:?}", m.regs()), ram_of(&m), dirty_of(&m), m.border_line(0));
    assert_eq!(before, after);
    assert_eq!(m.peek(0x8000), 0xA5);
}

#[test]
fn rejects_bad_formats() {
    let regs = sample_regs();
    let ram = sample_ram();
    let mut m = Machine::new();
    // bad IM in a .sna
    let mut f = make_sna(&regs, &ram, 3);
    f[25] = 3;
    assert_eq!(m.load_sna(&f), Err(ZX_ERR_FORMAT));
    // unknown extended header length
    let mut f = make_z80_v3(&regs, &ram, 0);
    f[30] = 40;
    assert_eq!(m.load_z80(&f), Err(ZX_ERR_FORMAT));
    // a page that is not a 48K page
    let mut f = make_z80_v3(&regs, &ram, 0);
    f[32 + 54 + 2] = 3;
    assert_eq!(m.load_z80(&f), Err(ZX_ERR_FORMAT));
    // a page missing
    let f = make_z80_v3(&regs, &ram, 0);
    assert_eq!(m.load_z80(&f[..f.len() - 3 - 0x4000]), Err(ZX_ERR_FORMAT));
    // a compressed page that decodes short
    let mut f = make_z80_v3(&regs, &ram, 0);
    let len = u16::from_le_bytes([f[32 + 54], f[32 + 55]]) as usize;
    f[32 + 54] = (len - 1) as u8;
    f[32 + 55] = ((len - 1) >> 8) as u8;
    assert_eq!(m.load_z80(&f), Err(ZX_ERR_FORMAT));
}

fn check_prefixes(name: &str, file: &[u8], step: impl Fn(usize) -> bool) {
    let mut m = dirty_machine();
    let before = (format!("{:?}", m.regs()), ram_of(&m), dirty_of(&m), m.border_line(0));
    for n in (0..file.len()).filter(|&n| step(n)) {
        let r = if name == "sna" { m.load_sna(&file[..n]) } else { m.load_z80(&file[..n]) };
        assert_eq!(r, Err(ZX_ERR_FORMAT), "{name} prefix {n}");
        assert_eq!(format!("{:?}", m.regs()), before.0, "{name} prefix {n}");
        assert_eq!(m.peek(0x8000), 0xA5);
    }
    assert_eq!(before, (format!("{:?}", m.regs()), ram_of(&m), dirty_of(&m), m.border_line(0)));
}

#[test]
fn rejects_truncated() {
    let regs = sample_regs();
    let ram = sample_ram();
    check_prefixes("sna", &make_sna(&regs, &ram, 3), |_| true);
    // the plain v1 file is 49 KB: every prefix near the header and the end,
    // every 7th between (each prefix is a full-length validation)
    let v1 = make_z80_v1(&regs, &ram, false);
    let l = v1.len();
    check_prefixes("z80", &v1, |n| n < 64 || n + 64 > l || n % 7 == 0);
    check_prefixes("z80", &make_z80_v1(&regs, &ram, true), |_| true);
    check_prefixes("z80", &make_z80_v3(&regs, &ram, 0), |_| true);
    check_prefixes("z80", &make_z80_vn(&regs, &ram, 2, 0), |_| true);
}

#[test]
fn rejects_run_past_48k() {
    let regs = sample_regs();
    let mut f = make_z80_v1(&regs, &sample_ram(), true);
    let n = f.len();
    // replace the end marker with a run of 200 bytes that overshoots 48 KB
    f.truncate(n - 4);
    f.extend_from_slice(&[0xED, 0xED, 200, 0x11, 0x00, 0xED, 0xED, 0x00]);
    let mut m = dirty_machine();
    assert_eq!(m.load_z80(&f), Err(ZX_ERR_FORMAT));
    // a stream that ends exactly at 48 KB but with no end marker
    let mut g = make_z80_v1(&regs, &sample_ram(), true);
    let n = g.len();
    g.truncate(n - 4);
    assert_eq!(m.load_z80(&g), Err(ZX_ERR_FORMAT));
    // a zero-length run
    let mut h = make_z80_v1(&regs, &sample_ram(), true);
    h.splice(30..30, [0xED, 0xED, 0x00, 0x01]);
    assert_eq!(m.load_z80(&h), Err(ZX_ERR_FORMAT));
}
