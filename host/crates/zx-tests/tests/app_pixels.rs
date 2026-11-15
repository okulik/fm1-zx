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

// ---- screen modes (pix_compose_row) ------------------------------------------------------------

use zx_sys::{pix_mark_border_line, pix_mark_paper_line, PIX_BORDER, PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH};

const FMTS: [PixFormat; 2] = [PixFormat::Rgb444, PixFormat::Rgb565];

fn snap(name: &str, frames: u32) -> Machine {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/test").join(name);
    let mut m = Machine::new();
    m.load_sna(&std::fs::read(&path).unwrap()).unwrap();
    for _ in 0..frames {
        m.run_frame();
    }
    m
}

fn scroll_sna() -> Machine {
    snap("scroll.sna", 50)
}

fn border_sna() -> Machine {
    snap("border.sna", 50)
}

/// Every attribute `attr` and every bitmap byte `bits`.
fn paint(m: &mut Machine, attr: u8, bits: u8) {
    for i in 0..0x1800u16 {
        m.poke(0x4000 + i, bits);
    }
    for i in 0..0x300u16 {
        m.poke(0x5800 + i, attr);
    }
}

/// Address of the first bitmap byte of paper line `y`.
fn line_addr(y: usize) -> u16 {
    (0x4000 | ((y & 0xC0) << 5) | ((y & 7) << 8) | ((y & 0x38) << 2)) as u16
}

/// A machine whose border is black from frame line 1, then `colours` (frame line, colour) from
/// each line on, in increasing line order (lines 2 and on). The OUTs come 8 T-states before their
/// line, which their I/O contention never pushes into the line.
fn border_machine(attr: u8, bits: u8, colours: &[(usize, u8)]) -> Machine {
    let mut m = Machine::new();
    paint(&mut m, attr, bits);
    out_fe(&mut m, 0, 224 - 8);
    for &(line, c) in colours {
        out_fe(&mut m, c, line as u32 * 224 - 8);
    }
    for (i, &(line, c)) in colours.iter().enumerate() {
        let end = colours.get(i + 1).map_or(312, |n| n.0);
        assert!((line..end).all(|l| m.border_line(l) == c), "border colour {c} on lines {line}-{end}");
    }
    m
}

fn all_rows(m: &Machine, mode: u32, crop_x: usize, fmt: PixFormat) -> Vec<Vec<u8>> {
    (0..240).map(|r| m.compose_row_mode(r, mode, crop_x, fmt.raw())).collect()
}

/// Pixel `x` of a packed row as (r, g, b) in the panel's units.
fn px(fmt: PixFormat, row: &[u8], x: usize) -> (u8, u8, u8) {
    match fmt {
        PixFormat::Rgb565 => {
            let c = (row[2 * x] as u16) << 8 | row[2 * x + 1] as u16;
            ((c >> 11) as u8, (c >> 5 & 63) as u8, (c & 31) as u8)
        }
        PixFormat::Rgb444 => {
            let p = &row[x / 2 * 3..x / 2 * 3 + 3];
            if x & 1 == 0 {
                (p[0] >> 4, p[0] & 15, p[1] >> 4)
            } else {
                (p[1] & 15, p[2] >> 4, p[2] & 15)
            }
        }
    }
}

fn marked(d: [u8; 30]) -> Vec<usize> {
    (0..240).filter(|&r| d[r >> 3] >> (r & 7) & 1 != 0).collect()
}

fn changed(a: &[Vec<u8>], b: &[Vec<u8>]) -> Vec<usize> {
    (0..240).filter(|&r| a[r] != b[r]).collect()
}

#[test]
fn crop_mode_is_today() {
    for m in [scroll_sna(), border_sna()] {
        for fmt in FMTS {
            for crop in [0usize, 8, 16] {
                for row in 0..240 {
                    assert_eq!(m.compose_row_mode(row, PIX_CROP, crop, fmt.raw()), fmt.pack(&m.compose_row(row, crop)), "row {row} crop {crop}");
                }
            }
        }
    }
}

#[test]
fn fit_drops_every_16th() {
    // the snapshots: output column x is source column x + x/15; border rows as Crop
    for m in [scroll_sna(), border_sna()] {
        for fmt in FMTS {
            for row in 0..240 {
                let idx: [u8; 240] = if (24..216).contains(&row) {
                    let line = m.screen_line(row - 24);
                    std::array::from_fn(|x| line[x + x / 15])
                } else {
                    m.compose_row(row, 0)
                };
                assert_eq!(m.compose_row_mode(row, PIX_FIT, 8, fmt.raw()), fmt.pack(&idx), "row {row}");
            }
        }
    }
    // one lit column at a time: columns 15, 31, ... 255 never appear, the others once each
    let mut m = border_machine(0x47, 0, &[]);
    let white = PixFormat::Rgb565.pack(&[15u8; 240]);
    for c in 0..256usize {
        let a = line_addr(0) + (c / 8) as u16;
        m.poke(a, 0x80 >> (c % 8));
        let row = m.compose_row_mode(24, PIX_FIT, 0, PixFormat::Rgb565.raw());
        m.poke(a, 0);
        let lit: Vec<usize> = (0..240).filter(|&x| row[2 * x..2 * x + 2] == white[2 * x..2 * x + 2]).collect();
        if c % 16 == 15 {
            assert!(lit.is_empty(), "column {c} shows at {lit:?}");
        } else {
            assert_eq!(lit, [c - c / 16], "column {c}");
        }
    }
}

#[test]
fn fit_smooth_weights_sum() {
    // bright white (0xFF) on black: an output pixel's green is (255 w + 8) >> 4 for the lit
    // column's weight w in sixteenths, and RGB565 keeps 6 bits of it, enough to tell w back
    let g6: Vec<u8> = (0..=16u32).map(|w| (((255 * w + 8) >> 4) >> 2) as u8).collect();
    let mut w = vec![[0u32; 256]; 240];
    let mut m = border_machine(0x47, 0, &[]);
    for c in 0..256usize {
        let a = line_addr(0) + (c / 8) as u16;
        m.poke(a, 0x80 >> (c % 8));
        let row = m.compose_row_mode(24, PIX_FIT_SMOOTH, 0, PixFormat::Rgb565.raw());
        m.poke(a, 0);
        for (x, wx) in w.iter_mut().enumerate() {
            let (_, g, _) = px(PixFormat::Rgb565, &row, x);
            wx[c] = g6.iter().position(|&v| v == g).unwrap_or_else(|| panic!("x {x} c {c}: green {g}")) as u32;
        }
    }
    for (x, wx) in w.iter().enumerate() {
        assert_eq!(wx.iter().sum::<u32>(), 16, "x {x}");
        let c0 = x + x / 15;
        let s = (x % 15) as u32;
        for (c, &v) in wx.iter().enumerate() {
            let want = if c == c0 { 15 - s } else if c == c0 + 1 { 1 + s } else { 0 };
            assert_eq!(v, want, "x {x} c {c}");
        }
    }
}

#[test]
fn blend_flat_is_exact() {
    for i in 0..16u8 {
        let attr = (i >> 3) << 6 | (i & 7) << 3 | (i & 7);
        let m = border_machine(attr, 0x55, &[(2, i & 7)]);
        for fmt in FMTS {
            let flat = fmt.pack(&[i; 240]);
            let border = fmt.pack(&[i & 7; 240]);
            let smooth = all_rows(&m, PIX_FIT_SMOOTH, 0, fmt);
            for (r, row) in smooth.iter().enumerate() {
                assert_eq!(row, if (24..216).contains(&r) { &flat } else { &border }, "colour {i} fit smooth row {r}");
            }
            let b = all_rows(&m, PIX_BORDER, 0, fmt);
            if i < 8 {
                for (r, row) in b.iter().enumerate() {
                    assert_eq!(row, &flat, "colour {i} border row {r}");
                }
                continue;
            }
            // a bright paper inside a plain border: rows of border lines only, and the paper inside
            for r in (0..35).chain(205..240) {
                assert_eq!(b[r], border, "colour {i} border row {r}");
            }
            for (r, row) in b.iter().enumerate().take(204).skip(36) {
                for x in 8..=231 {
                    assert_eq!(px(fmt, row, x), px(fmt, &flat, x), "colour {i} border row {r} x {x}");
                }
            }
        }
    }
}

#[test]
fn modes_ignore_crop_x() {
    for m in [scroll_sna(), border_sna()] {
        for fmt in FMTS {
            for mode in [PIX_FIT, PIX_FIT_SMOOTH, PIX_BORDER] {
                assert_eq!(all_rows(&m, mode, 0, fmt), all_rows(&m, mode, 16, fmt), "mode {mode}");
            }
        }
    }
}

#[test]
fn unknown_mode_is_crop() {
    // a mode number past Border composes as Crop, with the game's crop_x
    for m in [scroll_sna(), border_sna()] {
        for fmt in FMTS {
            for crop_x in [0, 8, 16] {
                assert_eq!(all_rows(&m, PIX_BORDER + 1, crop_x, fmt), all_rows(&m, PIX_CROP, crop_x, fmt), "crop_x {crop_x}");
                assert_eq!(all_rows(&m, 255, crop_x, fmt), all_rows(&m, PIX_CROP, crop_x, fmt), "crop_x {crop_x}");
            }
        }
    }
}

#[test]
fn rows_match_row() {
    // pix_compose_rows (the firmware's bands, which share source lines in Border) gives every row
    // exactly as pix_compose_row does, for bands at every offset and of every length up to 9
    for m in [scroll_sna(), border_sna()] {
        for fmt in FMTS {
            for mode in [PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH, PIX_BORDER] {
                let rows = all_rows(&m, mode, 8, fmt);
                for len in 1..=9 {
                    for row0 in 0..=240 - len {
                        let band = m.compose_rows_mode(row0, len, mode, 8, fmt.raw());
                        let want: Vec<u8> = rows[row0..row0 + len].concat();
                        assert!(band == want, "mode {mode}, rows {row0}..{}", row0 + len);
                    }
                }
            }
        }
    }
}

/// 8-bit R, G, B of palette index `i`.
fn pal8(i: u8) -> [u32; 3] {
    let ch = |bit: u8| if i & bit == 0 { 0 } else if i & 8 != 0 { 0xFF } else { 0xD7 };
    [ch(2), ch(4), ch(1)]
}

/// Packs 8-bit RGB pixels as the panel takes them (RGB565 5/6/5, RGB444 the top nibbles in pairs).
fn pack_rgb(fmt: PixFormat, px: &[[u32; 3]]) -> Vec<u8> {
    match fmt {
        PixFormat::Rgb565 => px
            .iter()
            .flat_map(|&[r, g, b]| (((r >> 3) << 11 | (g >> 2) << 5 | b >> 3) as u16).to_be_bytes())
            .collect(),
        PixFormat::Rgb444 => px
            .chunks(2)
            .flat_map(|p| {
                let ([r0, g0, b0], [r1, g1, b1]) = (p[0], p[1]);
                [(r0 & 0xF0 | g0 >> 4) as u8, (b0 & 0xF0 | r1 >> 4) as u8, (g1 & 0xF0 | b1 >> 4) as u8]
            })
            .collect(),
    }
}

/// Border's weights, {first, w0, w1, w2} (app/pixmodes.c): across a period of 15 output pixels
/// (17 columns), and down a period of 53 rows (60 lines).
const BORDER_H: [[u32; 4]; 15] = [
    [0, 226, 30, 0], [1, 196, 60, 0], [2, 166, 90, 0], [3, 136, 120, 0], [4, 105, 151, 0],
    [5, 75, 181, 0], [6, 45, 211, 0], [7, 15, 226, 15], [9, 211, 45, 0], [10, 181, 75, 0],
    [11, 151, 105, 0], [12, 120, 136, 0], [13, 90, 166, 0], [14, 60, 196, 0], [15, 30, 226, 0],
];
const BORDER_V: [[u32; 4]; 53] = [
    [0, 226, 30, 0], [1, 196, 60, 0], [2, 166, 90, 0], [3, 137, 119, 0], [4, 107, 149, 0],
    [5, 77, 179, 0], [6, 47, 209, 0], [7, 17, 226, 13], [9, 213, 43, 0], [10, 183, 73, 0],
    [11, 154, 102, 0], [12, 124, 132, 0], [13, 94, 162, 0], [14, 64, 192, 0], [15, 34, 222, 0],
    [16, 4, 226, 26], [18, 201, 55, 0], [19, 171, 85, 0], [20, 141, 115, 0], [21, 111, 145, 0],
    [22, 81, 175, 0], [23, 51, 205, 0], [24, 21, 226, 9], [26, 218, 38, 0], [27, 188, 68, 0],
    [28, 158, 98, 0], [29, 128, 128, 0], [30, 98, 158, 0], [31, 68, 188, 0], [32, 38, 218, 0],
    [33, 9, 226, 21], [35, 205, 51, 0], [36, 175, 81, 0], [37, 145, 111, 0], [38, 115, 141, 0],
    [39, 85, 171, 0], [40, 55, 201, 0], [41, 26, 226, 4], [43, 222, 34, 0], [44, 192, 64, 0],
    [45, 162, 94, 0], [46, 132, 124, 0], [47, 102, 154, 0], [48, 73, 183, 0], [49, 43, 213, 0],
    [50, 13, 226, 17], [52, 209, 47, 0], [53, 179, 77, 0], [54, 149, 107, 0], [55, 119, 137, 0],
    [56, 90, 166, 0], [57, 60, 196, 0], [58, 30, 226, 0],
];

/// Border's picture row `o` (0-211) by the filter's definition: each column blended down its
/// lines, then across, one rounding at the end.
fn border_reference(m: &Machine, o: usize) -> Vec<[u32; 3]> {
    let wv = BORDER_V[o % 53];
    let l0 = o / 53 * 60 + wv[0] as usize;
    let src = |l: usize, c: usize| -> u8 {
        if (24..216).contains(&l) && (8..264).contains(&c) {
            m.screen_line(l - 24)[c - 8]
        } else {
            m.border_line(40 + l)
        }
    };
    let v: Vec<[u32; 3]> = (0..272)
        .map(|c| {
            let mut acc = [0u32; 3];
            for k in 0..3 {
                if wv[1 + k] != 0 {
                    let p = pal8(src(l0 + k, c));
                    for ch in 0..3 {
                        acc[ch] += wv[1 + k] * p[ch];
                    }
                }
            }
            acc
        })
        .collect();
    (0..240)
        .map(|x| {
            let wh = BORDER_H[x % 15];
            let f = x / 15 * 17 + wh[0] as usize;
            let mut out = [0u32; 3];
            for (ch, o) in out.iter_mut().enumerate() {
                let sum: u32 = (0..3).filter(|&j| wh[1 + j] != 0).map(|j| wh[1 + j] * v[f + j][ch]).sum();
                *o = (sum + 32768) >> 16;
            }
            out
        })
        .collect()
}

/// Fit smooth's paper line `y` by its definition.
fn fit_smooth_reference(m: &Machine, y: usize) -> Vec<[u32; 3]> {
    let line = m.screen_line(y);
    (0..240)
        .map(|x| {
            let (k, s) = (x / 15 * 16, x % 15);
            let (a, b) = (pal8(line[k + s]), pal8(line[k + s + 1]));
            let (wa, wb) = (15 - s as u32, 1 + s as u32);
            [0, 1, 2].map(|ch| (wa * a[ch] + wb * b[ch] + 8) >> 4)
        })
        .collect()
}

#[test]
fn blends_match_reference() {
    // screens of random cells, some flat (one colour across) and some not, under a border that
    // changes colour on random lines: Fit smooth and Border equal their definitions, every row
    let mut seed = 0x2545_F491u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    // first a screen of whole lines of one colour: black paper, paper line 6 white, which Border's
    // row 40 blends half and half with line 5 (weights 128 and 128: a value half way between two
    // 8-bit steps, where the rounding shows)
    let mut half = border_machine(0x07, 0, &[(2, 0)]);
    for cell in 0..32u16 {
        half.poke(line_addr(6) + cell, 0xFF);
    }
    let mut machines = vec![half];
    for _ in 0..6 {
        let mut lines: Vec<usize> = (0..12).map(|_| 2 + rnd() as usize % 300).collect();
        lines.sort_unstable();
        lines.dedup();
        let colours: Vec<(usize, u8)> = lines.iter().map(|&l| (l, (rnd() % 8) as u8)).collect();
        let mut m = border_machine(0x38, 0, &colours);
        for i in 0..0x300u16 {
            m.poke(0x5800 + i, (rnd() & 0x7F) as u8); // no FLASH
        }
        for y in 0..192 {
            for cell in 0..32u16 {
                let bits = match rnd() % 4 {
                    0 => 0x00,
                    1 => 0xFF,
                    _ => rnd() as u8,
                };
                m.poke(line_addr(y) + cell, bits);
            }
        }
        machines.push(m);
    }
    for m in &machines {
        for fmt in FMTS {
            for o in 0..212 {
                assert_eq!(m.compose_row_mode(14 + o, PIX_BORDER, 0, fmt.raw()), pack_rgb(fmt, &border_reference(m, o)), "Border row {}", 14 + o);
            }
            for y in 0..192 {
                assert_eq!(m.compose_row_mode(24 + y, PIX_FIT_SMOOTH, 0, fmt.raw()), pack_rgb(fmt, &fit_smooth_reference(m, y)), "Fit smooth row {}", 24 + y);
            }
        }
    }
}

#[test]
fn border_geometry() {
    let base = border_machine(0x38, 0x0F, &[(2, 5)]);
    // frame lines 2-63 red, from 64 cyan: only the top border lines 40-63 differ
    let top = border_machine(0x38, 0x0F, &[(2, 2), (64, 5)]);
    // the bottom border lines 256-279 (and on) red
    let bottom = border_machine(0x38, 0x0F, &[(2, 5), (256, 2)]);
    assert!((40..64).all(|l| top.border_line(l) == 2) && (64..280).all(|l| top.border_line(l) == 5));
    assert!((40..256).all(|l| bottom.border_line(l) == 5) && (256..280).all(|l| bottom.border_line(l) == 2));
    for fmt in FMTS {
        let (b, t, bt) = (all_rows(&base, PIX_BORDER, 0, fmt), all_rows(&top, PIX_BORDER, 0, fmt), all_rows(&bottom, PIX_BORDER, 0, fmt));
        let (red, cyan) = (fmt.pack(&[2; 240]), fmt.pack(&[5; 240]));
        for r in 0..14 {
            assert_eq!((&b[r], &t[r], &bt[r]), (&cyan, &red, &cyan), "row {r}");
        }
        for r in 226..240 {
            assert_eq!((&b[r], &t[r], &bt[r]), (&cyan, &cyan, &red), "row {r}");
        }
        assert_eq!(changed(&b, &t), (0..36).collect::<Vec<_>>());
        assert_eq!(changed(&b, &bt), (204..240).collect::<Vec<_>>());
    }
}

#[test]
fn mark_rows() {
    for mode in [PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH] {
        for y in 0..192 {
            assert_eq!(marked(pix_mark_paper_line(mode, y)), [24 + y], "mode {mode} paper line {y}");
        }
        assert_eq!(marked(pix_mark_paper_line(mode, 192)), [0usize; 0]);
        for line in 0..312 {
            let want: Vec<usize> = if (40..64).contains(&line) || (256..280).contains(&line) { vec![line - 40] } else { vec![] };
            assert_eq!(marked(pix_mark_border_line(mode, line)), want, "mode {mode} border line {line}");
        }
    }
    // Border: the rows a change shows in are the rows marked (RGB565 shows every change RGB444
    // does, and more)
    let fmt = PixFormat::Rgb565;
    assert_eq!(marked(pix_mark_paper_line(PIX_BORDER, 192)), [0usize; 0]);
    let before = all_rows(&border_machine(0x47, 0, &[]), PIX_BORDER, 0, fmt);
    for y in 0..192 {
        let mut m = border_machine(0x47, 0, &[]);
        for i in 0..32 {
            m.poke(line_addr(y) + i, 0xFF);
        }
        assert_eq!(changed(&before, &all_rows(&m, PIX_BORDER, 0, fmt)), marked(pix_mark_paper_line(PIX_BORDER, y)), "paper line {y}");
    }
    let black = all_rows(&border_machine(0, 0, &[]), PIX_BORDER, 0, fmt);
    for line in 2..309 {
        let want = marked(pix_mark_border_line(PIX_BORDER, line));
        // black to white; then the same between green lines two either side, which shows the
        // change in the rows where the line's weight is small
        let one = changed(&black, &all_rows(&border_machine(0, 0, &[(line, 7), (line + 1, 0)]), PIX_BORDER, 0, fmt));
        let two = if line >= 4 {
            let ring = [(line - 2, 4), (line - 1, 0), (line + 2, 4), (line + 3, 0)];
            let lit = [(line - 2, 4), (line - 1, 0), (line, 7), (line + 1, 0), (line + 2, 4), (line + 3, 0)];
            changed(&all_rows(&border_machine(0, 0, &ring), PIX_BORDER, 0, fmt), &all_rows(&border_machine(0, 0, &lit), PIX_BORDER, 0, fmt))
        } else {
            vec![]
        };
        assert!(one.iter().chain(&two).all(|r| want.contains(r)), "border line {line}: {one:?} {two:?} not in {want:?}");
        let mut seen: Vec<usize> = one.iter().chain(&two).copied().collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen, want, "border line {line}");
        assert_eq!(want.is_empty(), !(40..280).contains(&line), "border line {line}");
    }
}

#[test]
fn golden_modes() {
    for (name, m) in [("scroll", scroll_sna()), ("border", border_sna())] {
        for (mname, mode) in [("crop", PIX_CROP), ("fit", PIX_FIT), ("fitsmooth", PIX_FIT_SMOOTH), ("border", PIX_BORDER)] {
            for (fname, fmt) in [("rgb444", PixFormat::Rgb444), ("rgb565", PixFormat::Rgb565)] {
                let all: Vec<u8> = all_rows(&m, mode, 8, fmt).concat();
                golden(&format!("pixels_{name}_frame50_{mname}_{fname}"), &format!("{:08x}", fnv1a(&all)));
            }
        }
    }
}
