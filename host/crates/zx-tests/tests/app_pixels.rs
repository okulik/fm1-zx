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
    // every frame line next to 40 and 279 has a different colour
    out_fe(&mut m, 2, 40 * 224);
    out_fe(&mut m, 3, 41 * 224);
    out_fe(&mut m, 5, 279 * 224);
    out_fe(&mut m, 6, 280 * 224);
    assert_eq!([39, 40, 41].map(|l| m.border_line(l)), [7, 2, 3]);
    assert_eq!([278, 279, 280].map(|l| m.border_line(l)), [3, 5, 6]);
    assert_eq!(m.compose_row(0, 0), [2u8; 240]);
    assert_eq!(m.compose_row(1, 0), [3u8; 240]);
    assert_eq!(m.compose_row(238, 0), [3u8; 240]);
    assert_eq!(m.compose_row(239, 0), [5u8; 240]);
    // row 23 is the last top border row, 216 the first bottom one
    assert_eq!(m.compose_row(23, 0), [m.border_line(63); 240]);
    assert_eq!(m.compose_row(216, 0), [m.border_line(256); 240]);
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

#[test]
fn compose_paper_edges_and_crop_clamp() {
    let mut m = Machine::new();
    for i in 0..0x1800u16 {
        m.poke(0x4000 + i, (i * 5 + 1) as u8);
    }
    for i in 0..0x300u16 {
        m.poke(0x5800 + i, (i * 11 + 9) as u8);
    }
    let last = m.screen_line(191);
    assert_eq!(m.compose_row(215, 8)[..], last[8..248]);
    assert_eq!(m.compose_row(24, 40), m.compose_row(24, 16));
    assert_eq!(m.compose_row(24, 40)[..], m.screen_line(0)[16..256]);
}

#[test]
fn rgb444_second_pair() {
    let mut idx = [0u8; 240];
    idx[10] = 7; // white
    idx[11] = 8; // bright black
    idx[12] = 15; // bright white
    idx[13] = 1; // blue
    let out = PixFormat::Rgb444.pack(&idx);
    // pair 5 (pixels 10, 11) -> bytes 15..18; pair 6 (12, 13) -> bytes 18..21
    assert_eq!(&out[15..18], &[0xDD, 0xD0, 0x00]);
    assert_eq!(&out[18..21], &[0xFF, 0xF0, 0x0D]);
}
