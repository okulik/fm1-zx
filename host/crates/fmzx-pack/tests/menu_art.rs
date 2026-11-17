// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use fmzx_pack::menu_art::generate;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../..");

#[test]
fn menu_art_c_is_generated() {
    let png = std::fs::read(format!("{ROOT}/app/art/menu-top.png")).unwrap();
    let want = std::fs::read_to_string(format!("{ROOT}/app/menu_art.c")).unwrap();
    assert_eq!(generate(&png).unwrap(), want, "regenerate: fmzx-pack menu-art app/art/menu-top.png -o app/menu_art.c");
}

fn rgb_png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    out
}

#[test]
fn menu_art_refuses_unknown_colour() {
    let png = rgb_png(240, 64, |x, y| if (x, y) == (5, 7) { [0x12, 0x34, 0x56] } else { [0, 0, 0] });
    let err = generate(&png).unwrap_err();
    assert!(err.contains("123456"), "{err}");
}

#[test]
fn menu_art_refuses_wrong_size() {
    let png = rgb_png(240, 63, |_, _| [0, 0, 0]);
    assert!(generate(&png).is_err());
}

#[test]
fn menu_art_accepts_band_colours() {
    let png = rgb_png(240, 64, |x, _| if x % 2 == 0 { [0xcc, 0x99, 0xdd] } else { [0xff, 0xff, 0xff] });
    assert!(generate(&png).unwrap().contains("ui_art_top"));
}
