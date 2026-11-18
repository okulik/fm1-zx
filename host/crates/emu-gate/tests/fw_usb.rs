// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! USB in fm1-emu, with a host at the other end of the cable (`Soc::usb_plug`): FM-1 ZX enumerates
//! as a USB-MIDI device named "FM-1 ZX" and answers the installers' handshake in every view, while
//! MIDI from the computer comes in, and after a cable plugged in while a game runs. Needs the device
//! build (`FMZX_FM1_BUILD`).

use emu_gate::budgets::{start_game_in, SCREEN_CROP, SOUND_FM};
use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use emu_gate::upgrade::{handshake, Identity};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::view;

const PLAY: u8 = 12;
const HOME: u8 = 8;
const SELECT: u8 = 0;
const OPT: BootOpt = BootOpt { rgb565: false, divider: 4 };

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn firmware() -> &'static Firmware {
    static FW: OnceLock<Firmware> = OnceLock::new();
    FW.get_or_init(|| {
        let pack = fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack");
        Firmware::load(&build_dir()).with_pack(&pack)
    })
}

/// Index of the game `id` in `games/games.toml`.
fn game_index(id: &str) -> u32 {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read the manifest");
    let m = fmzx_pack::manifest::parse_manifest(&text).expect("parse the manifest");
    m.game.iter().position(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}")) as u32
}

/// Booted with the cable in, in the picker.
fn boot_plugged() -> Machine {
    let mut m = Machine::boot(firmware(), OPT);
    m.soc.usb_plug();
    m.run_ms(1500);
    assert_eq!(m.resets(), 0, "the firmware reset while booting");
    m
}

/// Presses `key` and waits (up to 300 ms) until fmzx_stats.view is `v`.
fn press_to(m: &mut Machine, key: u8, v: i32) {
    m.press(key, 30);
    for _ in 0..300 {
        if m.stats().view == v as u32 {
            return;
        }
        m.run_ms(1);
    }
    panic!("view {} after pressing {key}, expected {v}", m.stats().view);
}

/// From the picker, Castaway started (its tape loading).
fn start_castaway(m: &mut Machine) {
    m.turn(SELECT, game_index("castaway") as i32);
    m.run_ms(200);
    press_to(m, PLAY, view::GAME);
}

fn assert_identity(id: Option<Identity>, what: &str) {
    let id = id.unwrap_or_else(|| panic!("{what}: no answer to the handshake"));
    assert_eq!(id.text, "FM-1_048", "{what}");
}

#[test]
fn enumerates_as_fm1_zx() {
    let m = boot_plugged();
    assert_eq!(m.soc.usb.product(), Some("FM-1 ZX"));
}

#[test]
fn handshake_reports_identity() {
    let mut m = boot_plugged();
    assert_identity(handshake(&mut m, false, 500), "the picker");
}

#[test]
fn full_handshake_reports_flash() {
    let mut m = boot_plugged();
    let id = handshake(&mut m, true, 500).expect("no answer to the full handshake");
    assert_eq!(id.text, "FM-1_048");
    assert_eq!(id.jedec, Some([0x85, 0x60, 0x14]), "the Puya part");

    let mut m = Machine::boot(firmware(), OPT);
    m.soc.flash.jedec = [0xB3, 0x60, 0x14];
    m.soc.usb_plug();
    m.run_ms(1500);
    let id = handshake(&mut m, true, 500).expect("no answer to the full handshake");
    assert_eq!(id.jedec, Some([0xB3, 0x60, 0x14]), "the UC25HQ80 part");
}

#[test]
fn handshake_in_every_view() {
    let mut m = boot_plugged();
    assert_identity(handshake(&mut m, false, 500), "the picker");
    start_castaway(&mut m);
    m.run_ms(1000);
    assert_identity(handshake(&mut m, false, 500), "Castaway running");
    assert_eq!(m.stats().view, view::GAME as u32);
    press_to(&mut m, HOME, view::MENU);
    assert_identity(handshake(&mut m, false, 500), "the HOME menu");
    press_to(&mut m, PLAY, view::GAME); // OK on "Resume"
    m.run_ms(300);
    press_to(&mut m, PLAY, view::PAUSED);
    assert_identity(handshake(&mut m, false, 500), "paused");
    assert_eq!(m.stats().view, view::PAUSED as u32);
}

#[test]
fn notes_do_not_block_sysex() {
    let mut m = boot_plugged();
    start_castaway(&mut m);
    m.run_ms(500);
    // three rounds: more notes than the device's MIDI-in ring holds (64) unless it is emptied
    for round in 1..=3 {
        for i in 0..40u8 {
            m.soc.usb.midi_send(&[0x90, 36 + i, 100]);
        }
        m.run_ms(100);
        assert_identity(handshake(&mut m, false, 500), &format!("after 40 notes, round {round}"));
    }
    assert_eq!(m.stats().view, view::GAME as u32);
}

/// A running game with no cable for 3 s, then the cable in for 3 s: 50 frames a second, no audio
/// underrun, enumerated within 1.5 s of the plug, the handshake answered.
fn plug_mid_game(m: &mut Machine, what: &str) {
    m.run_ms(200);
    let u0 = m.stats().underruns;
    m.run_ms(3000);
    let s0 = m.stats();
    m.soc.usb_plug();
    m.run_ms(1500);
    assert_eq!(m.soc.usb.product(), Some("FM-1 ZX"), "{what}: not enumerated 1.5 s after the cable went in");
    m.run_ms(1500);
    let s1 = m.stats();
    println!("{what}: no cable {s0:?}\n{what}: cable in 3 s {s1:?}");
    assert_eq!(m.resets(), 0, "{what}");
    assert!(s1.frames - s0.frames >= 148, "{what}: {} frames in the 3 s after the cable went in", s1.frames - s0.frames);
    assert_eq!(s1.underruns, u0, "{what}: audio underruns");
    assert_identity(handshake(m, false, 500), what);
}

#[test]
fn plug_after_boot_keeps_frames() {
    let mut m = Machine::boot(firmware(), OPT);
    m.run_ms(1500);
    start_castaway(&mut m);
    plug_mid_game(&mut m, "Castaway");
}

/// As above, in the heaviest sound load there is: `noise.sna` (250 speaker edges a frame) in
/// FM: sound, at the LCD divider that sends the most (1).
#[test]
fn plug_after_boot_keeps_frames_heavy() {
    let dir = build_dir().join("usb-noise-pack");
    std::fs::create_dir_all(&dir).expect("create the noise pack's folder");
    std::fs::write(dir.join("noise.sna"), fmzx_pack::testprogs::noise_sna()).expect("write noise.sna");
    std::fs::write(dir.join("games.toml"), "[[game]]\nid = \"noise\"\nname = \"Speaker noise\"\nfile = \"noise.sna\"\n")
        .expect("write the manifest");
    let pack = fmzx_pack::pack::build_pack(&dir.join("games.toml")).expect("build the noise pack");
    let fw = Firmware::load(&build_dir()).with_pack(&pack);
    let mut m = start_game_in(&fw, BootOpt { rgb565: false, divider: 1 }, 0, SOUND_FM, SCREEN_CROP);
    plug_mid_game(&mut m, "noise.sna, FM: sound");
}
