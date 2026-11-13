// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware's key-matrix, encoder and MASTER-pot input, seen through the game picker in
//! fm1-emu. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! fm1-emu key ids: buttons 0-13 (OCT- 0, OCT+ 1, ... PLAY 12, REC 13), notes 14-40.

use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const RGB565: BootOpt = BootOpt { rgb565: true, divider: 4 };
const OCT_UP: u8 = 1;
const PLAY: u8 = 12;
const SELECT: u8 = 0;
const ALGORITHM: u8 = 1;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn firmware() -> &'static Firmware {
    static FW: OnceLock<Firmware> = OnceLock::new();
    FW.get_or_init(|| Firmware::load(&build_dir()))
}

fn pack() -> &'static [u8] {
    static PACK: OnceLock<Vec<u8>> = OnceLock::new();
    PACK.get_or_init(|| fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack"))
}

fn boot() -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack()), RGB565);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    m
}

/// Presses PLAY and returns the game the picker started.
fn play(m: &mut Machine) -> u32 {
    m.press(PLAY, 30);
    m.run_ms(100);
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::GAME as u32, "PLAY did not start a game");
    s.game
}

#[test]
fn encoder_scrolls_picker() {
    let mut m = boot();
    m.turn(SELECT, 2);
    m.run_ms(200);
    assert_eq!(play(&mut m), 2);
}

/// The picker reaches its last entry (the last of the nine entries, which all fit on one screen) and PLAY starts
/// it: `tune`, found by id in the manifest.
#[test]
fn picker_reaches_last_game() {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read the manifest");
    let manifest = fmzx_pack::manifest::parse_manifest(&text).expect("parse the manifest");
    let last = manifest.game.len() as u32 - 1;
    let tune = manifest.game.iter().position(|g| g.id == "tune").expect("no game tune") as u32;
    assert_eq!(tune, last, "tune is the last game in the picker");
    let mut m = boot();
    m.turn(SELECT, last as i32);
    m.run_ms(200);
    assert_eq!(play(&mut m), tune);
}

#[test]
fn algorithm_encoder_scrolls_back() {
    let mut m = boot();
    m.turn(ALGORITHM, 3);
    m.run_ms(200);
    m.turn(ALGORITHM, -1);
    m.run_ms(200);
    assert_eq!(play(&mut m), 2);
}

#[test]
fn oct_buttons_scroll() {
    let mut m = boot();
    m.press(OCT_UP, 30);
    m.run_ms(100);
    assert_eq!(play(&mut m), 1);
}

#[test]
fn play_starts_game() {
    let mut m = boot();
    assert_eq!(m.stats().view, zx_sys::view::PICKER as u32);
    m.turn(SELECT, 2);
    m.run_ms(200);
    assert_eq!(play(&mut m), 2);
}

#[test]
fn held_button_is_one_press() {
    let mut m = boot();
    m.press(OCT_UP, 400);
    m.run_ms(100);
    assert_eq!(play(&mut m), 1);
}

#[test]
fn master_pot_read() {
    let mut m = boot();
    m.set_adc(4, 512);
    m.run_ms(600);
    let v = m.peek32(m.symbol("fmzx_master")) & 0xFFFF;
    assert!(v.abs_diff(512) <= 8, "fmzx_master {v}");
    m.set_adc(4, 100);
    m.run_ms(600);
    let v = m.peek32(m.symbol("fmzx_master")) & 0xFFFF;
    assert!(v.abs_diff(100) <= 8, "fmzx_master {v}");
}
