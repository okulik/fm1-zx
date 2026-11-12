// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The update packages boot in fm1-emu the way its web page boots a local package
//! (`fm1_soc::stock_from_package`, `stock_boot`, board `Fm1`).
//!
//! Needs the device builds and the packages: `tools/make-packages` (it runs both device builds,
//! `fm1` and `fm1-preview`, and writes `build/fm1/fmzx.fwsc` and `build/fm1/fmzx-preview.fwsc`).
//! Then, from `host/`, `FMZX_FM1_BUILD=$PWD/../build/fm1` as for the other firmware tests; the
//! preview build is `build/fm1-preview` next to it (or `FMZX_FM1_PREVIEW_BUILD`).

use emu_gate::fw::{assert_golden, build_dir, BootOpt, Firmware, Machine};
use fmzx_pack::inputs::{sha256_hex, Pins};
use fmzx_pack::layout::pack_offset;
use fmzx_pack::package::{verify, PRODUCT};
use std::path::{Path, PathBuf};

const BOOT_MS: u64 = 1500;
/// Felucca 1.6.1's loader wrapped as `ota.bin`, as Felucca's own tools/build.py `ldr_wrap`
/// wraps it (computed with those functions from the pinned `loader.bin`): 6,934 bytes.
const OTA_SHA256: &str = "d578658e22250ad9c1616fe082671c2fec115e84f249b9e75973895270746203";

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn preview_dir() -> PathBuf {
    match std::env::var_os("FMZX_FM1_PREVIEW_BUILD").filter(|v| !v.is_empty()) {
        Some(v) => std::path::absolute(v).expect("absolute preview build dir"),
        None => build_dir().join("../fm1-preview"),
    }
}

fn read_package(name: &str) -> Vec<u8> {
    let path = build_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!("read {}: {e}\nmake the packages first: tools/make-packages (from the repository root)", path.display())
    })
}

/// The package holds exactly this build's image (it is not stale) and a game pack after it.
fn assert_holds(pkg: &[u8], fw: &Firmware) {
    let u = fm1_soc::fwsc::unpack(pkg).expect("the package unpacks");
    assert_eq!(u.identity, PRODUCT);
    let img = &u.xip[0x120..0x120 + fw.image.len()];
    assert!(img == &fw.image[..], "the package's image is not this build's fmzx_app.bin: run tools/make-packages again");
    let at = pack_offset(fw.image.len());
    assert_eq!(&u.xip[at..at + 8], b"FMZXPACK", "no game pack at {at:#x}");
}

fn assert_screen(m: &Machine, dir: &Path, name: &str, actual: &str) {
    assert_golden(m.lcd_rgb565(), &golden(name), &dir.join(actual));
}

#[test]
fn package_boots() {
    let fw = Firmware::load(&build_dir());
    let pkg = read_package("fmzx.fwsc");
    assert_holds(&pkg, &fw);
    // RGB565 by boot option, so that fm1-emu's LCD shows the picture: the picker as in fw_boot
    let mut m = Machine::boot_package(&fw, &pkg, Some(BootOpt { rgb565: true, divider: 4 }));
    m.watch("fmzx_enter_uboot");
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0);
    assert_eq!(m.hits("fmzx_enter_uboot"), 0);
    assert!(m.display_on());
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::PICKER as u32);
    assert_eq!((s.divider, s.pending), (4, 0));
    assert_screen(&m, &build_dir(), "picker.png", "package_picker.actual.png");
}

#[test]
fn release_package_is_rgb444_without_boot_options() {
    // the panel gets RGB444, which fm1-emu's LCD reads as RGB565: the firmware runs, the picture
    // is not the picker's (hence the preview package)
    let fw = Firmware::load(&build_dir());
    let mut m = Machine::boot_package(&fw, &read_package("fmzx.fwsc"), None);
    m.run_ms(BOOT_MS);
    assert!(m.display_on());
    assert_eq!(m.stats().view, zx_sys::view::PICKER as u32);
    let want = emu_gate::fw::read_png(&golden("picker.png")).expect("picker.png");
    let got: Vec<u8> = m.lcd_rgb565().iter().flat_map(|&v| emu_gate::fw::rgb565_to_rgb8(v)).collect();
    assert_ne!(got, want);
}

#[test]
fn preview_package_boots_without_boot_options() {
    let fw = Firmware::load(&preview_dir());
    let pkg = read_package("fmzx-preview.fwsc");
    assert_holds(&pkg, &fw);
    let mut m = Machine::boot_package(&fw, &pkg, None);
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0);
    assert!(m.display_on());
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::PICKER as u32);
    assert_eq!((s.divider, s.pending), (4, 0));
    assert_screen(&m, &build_dir(), "picker.png", "preview_picker.actual.png");
    // as on fm1-emu's web page, no audio PLL: no sound
    assert!(m.audio().is_empty(), "audio ran without the stock audio PLL");
}

#[test]
fn packages_verify() {
    for name in ["fmzx.fwsc", "fmzx-preview.fwsc"] {
        let r = verify(&read_package(name), &Pins::builtin(), Some(PRODUCT));
        assert!(r.problems.is_empty(), "{name}: {:?}", r.problems);
        assert_eq!(r.pack.map(|p| p.1), Some(4), "{name}: the four games of games/games.toml");
    }
}

#[test]
fn loader_wraps_like_felucca() {
    let loader = std::fs::read(build_dir().join("third_party/felucca-loader/loader.bin")).expect("read loader.bin");
    fmzx_pack::inputs::check_loader(&Pins::builtin(), &loader).expect("the pinned loader");
    let ota = fmzx_pack::lz4blk::wrap_loader(&loader).unwrap();
    assert_eq!((ota.len(), sha256_hex(&ota).as_str()), (6934, OTA_SHA256));
}
