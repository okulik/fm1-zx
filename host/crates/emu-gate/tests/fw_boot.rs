// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware boots to the game picker in fm1-emu. Needs the device build: set
//! `FMZX_FM1_BUILD` (the tests fail with the build commands otherwise).
//!
//! The pack is the whole of `games/games.toml` (three games: the picker shows them all), built
//! once per run with `fmzx-pack`.

use emu_gate::fw::{assert_golden, build_dir, BootOpt, Firmware, Machine, KEY_OCT_DOWN};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const RGB565: BootOpt = BootOpt { rgb565: true, divider: 4 };
const BOOT_MS: u64 = 1500;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn firmware() -> &'static Firmware {
    static FW: OnceLock<Firmware> = OnceLock::new();
    FW.get_or_init(|| Firmware::load(&build_dir()))
}

fn pack() -> &'static [u8] {
    static PACK: OnceLock<Vec<u8>> = OnceLock::new();
    PACK.get_or_init(|| fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack"))
}

fn boot(fw: &Firmware) -> Machine {
    let mut m = Machine::boot(fw, RGB565);
    m.watch("fmzx_enter_uboot");
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0, "the firmware reset while booting");
    assert_eq!(m.hits("fmzx_enter_uboot"), 0, "entered UBOOT without the gesture");
    m
}

fn assert_screen(m: &Machine, name: &str) {
    let actual = build_dir().join(format!("{}.actual.png", name.trim_end_matches(".png")));
    assert_golden(m.lcd_rgb565(), &golden(name), &actual);
}

#[test]
fn boots_to_picker() {
    let m = boot(&firmware().with_pack(pack()));
    assert!(m.display_on(), "no DISPON after {BOOT_MS} ms");
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::PICKER as u32);
    assert_eq!((s.divider, s.budget), (4, 30_000));
    assert_eq!(s.pending, 0, "rows still pending after {BOOT_MS} ms");
    assert!(s.lcd.row_sends >= 240, "the whole screen was never sent: {s:?}");
    assert_screen(&m, "picker.png");
}

#[test]
fn no_pack_shows_no_games() {
    let m = boot(firmware());
    assert!(m.display_on());
    assert_eq!(m.stats().view, zx_sys::view::PICKER as u32);
    assert_screen(&m, "no_games.png");
}

#[test]
fn corrupt_pack_shows_no_games() {
    let mut p = pack().to_vec();
    let at = p.len() / 2;
    p[at] ^= 0x10;
    let m = boot(&firmware().with_pack(&p));
    assert!(m.display_on());
    assert_screen(&m, "no_games.png");
}

#[test]
fn no_misaligned_access() {
    // Every instruction goes through the alignment-checked bus, which panics with the pc; this
    // run also draws the picker at the fastest divider.
    let mut m = Machine::boot(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 0 });
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0);
    let s = m.stats();
    assert_eq!((s.divider, s.budget, s.pending), (0, 150_000, 0));
}

#[test]
fn uboot_gesture() {
    let mut m = Machine::boot(firmware(), RGB565);
    m.hold(KEY_OCT_DOWN, true);
    m.watch("fmzx_enter_uboot");
    m.run_ms(1200);
    let at = m.first_hit_ms("fmzx_enter_uboot").expect("fmzx_enter_uboot not reached in 1200 ms with OCT- held");
    assert!((1000..=1200).contains(&at), "entered UBOOT at {at} ms, expected just after 1 s");
}

#[test]
fn oct_tap_at_boot_still_boots() {
    let mut m = Machine::boot(&firmware().with_pack(pack()), RGB565);
    m.watch("fmzx_enter_uboot");
    m.press(KEY_OCT_DOWN, 300);
    m.run_ms(BOOT_MS);
    assert_eq!(m.hits("fmzx_enter_uboot"), 0);
    assert!(m.display_on());
    assert_screen(&m, "picker.png");
}

#[test]
fn cost_of_counts_whole_calls() {
    let mut m = Machine::boot(&firmware().with_pack(pack()), RGB565);
    m.watch("lcd_compose_band");
    m.watch("ui_compose_row");
    m.run_ms(BOOT_MS);
    let bands = m.cost_of("lcd_compose_band");
    let rows = m.cost_of("ui_compose_row");
    let s = m.stats();
    assert_eq!(bands.len() as u32, s.lcd.bands, "one compose per band sent");
    assert_eq!(rows.len() as u32, s.lcd.row_sends, "one ui_compose_row per row sent");
    // a band's cost includes its rows' (nested calls count in the caller)
    let (b, r): (u64, u64) = (bands.iter().sum(), rows.iter().sum());
    assert!(r < b, "rows {r} >= bands {b}");
    assert!(rows.iter().all(|&c| (100..100_000).contains(&c)), "row costs {rows:?}");
}

/// The screen the host composes from the same pack (app/ built for the host through zx-sys).
fn host_picker(pack: Option<&[u8]>) -> Vec<u16> {
    let count = pack.map_or(0, |p| zx_sys::pack_open(p).expect("pack opens on the host"));
    let mut ui = zx_sys::Ui::new();
    let mut menu = zx_sys::Menu::new(count);
    menu.set_divider(4);
    ui.draw(&menu, pack);
    (0..240)
        .flat_map(|r| {
            let bytes = zx_sys::PixFormat::Rgb565.pack(&ui.compose_row(r));
            bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn picker_matches_host_composition() {
    let m = boot(&firmware().with_pack(pack()));
    assert!(m.lcd_rgb565() == &host_picker(Some(pack()))[..], "the LCD differs from the host's picker");
    let m = boot(firmware());
    assert!(m.lcd_rgb565() == &host_picker(None)[..], "the LCD differs from the host's \"No games\"");
}
