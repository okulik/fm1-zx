// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

const OUT_FE_A: [u8; 2] = [0xD3, 0xFE];

fn out_fe(m: &mut Machine, v: u8, t: u32) {
    run_at(m, &OUT_FE_A, v, 0, 0, t - 7);
}

#[test]
fn rgb444_pair_packing() {
    let mut idx = [0u8; 240];
    idx[0] = 2;
    idx[1] = 13;
    let out = PixFormat::Rgb444.pack(&idx);
    assert_eq!(&out[..3], &[0xD0, 0x00, 0xFF]);
    assert_eq!(out.len(), 360);
}

#[test]
fn rgb565_white_bright() {
    let mut idx = [15u8; 240];
    idx[1] = 7;
    let out = PixFormat::Rgb565.pack(&idx);
    assert_eq!(&out[..4], &[0xFF, 0xFF, 0xD6, 0xBA]);
}

#[test]
fn row_bytes() {
    assert_eq!(PixFormat::Rgb444.row_bytes(), 360);
    assert_eq!(PixFormat::Rgb565.row_bytes(), 480);
}

#[test]
fn compose_border_rows() {
    let mut m = Machine::new();
    out_fe(&mut m, 2, 30 * 224);
    out_fe(&mut m, 5, 270 * 224);
    assert_eq!(m.border_line(40), 2);
    assert_eq!(m.border_line(279), 5);
    assert_eq!(m.compose_row(0, 0), [2u8; 240]);
    assert_eq!(m.compose_row(239, 0), [5u8; 240]);
}

#[test]
fn compose_crop() {
    let mut m = Machine::new();
    for i in 0..0x1800u16 {
        m.poke(0x4000 + i, (i * 7 + 3) as u8);
    }
    for i in 0..0x300u16 {
        m.poke(0x5800 + i, (i * 13) as u8);
    }
    let full = m.screen_line(0);
    for crop in [0usize, 8, 16] {
        assert_eq!(m.compose_row(24, crop)[..], full[crop..crop + 240]);
    }
}
