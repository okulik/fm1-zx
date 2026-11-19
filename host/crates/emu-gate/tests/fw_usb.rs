// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! USB in fm1-emu, with a host at the other end of the cable (`Soc::usb_plug`): FM-1 ZX enumerates
//! as a USB-MIDI device named "FM-1 ZX" and answers the installers' handshake in every view, while
//! MIDI from the computer comes in, and after a cable plugged in while a game runs. The update
//! session against the release package (`build/fm1/fmzx.fwsc`, tools/make-packages): its loader
//! staged and committed on both flash parts, nothing written outside the OTA area, a session
//! abandoned, corrupted or for another flash layout leaving the OTA area erased and FM-1 ZX where
//! it was; the soft UBOOT key. Needs the device build (`FMZX_FM1_BUILD`).

use emu_gate::budgets::{start_game_in, SCREEN_CROP, SOUND_FM};
use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use emu_gate::upgrade::{handshake, logical_image, soft_uboot, upgrade_step1, Identity, Serve, ServeOpts};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::view;

const PLAY: u8 = 12;
const HOME: u8 = 8;
const SELECT: u8 = 0;
const OPT: BootOpt = BootOpt { rgb565: false, divider: 4 };
const OPT_565: BootOpt = BootOpt { rgb565: true, divider: 4 };

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
    boot_plugged_with(OPT)
}

/// As [`boot_plugged`], with boot options `opt` (RGB565 for a panel compared pixel by pixel: fm1-emu's
/// LCD reads every pixel as RGB565, so an RGB444 picture depends on how its rows went out).
fn boot_plugged_with(opt: BootOpt) -> Machine {
    let mut m = Machine::boot(firmware(), opt);
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

/// The OTA area, where the update stages the package's loader: felucca-bsp's chip/ota.c.
const OTA: std::ops::Range<usize> = 0xE0000..0xE5000;
/// The release package's flash.bin length: the flash up to the app area's end.
const FLASH_BIN: usize = 0x93000;
/// A flash.bin with a longer app area (the stock firmware's V16 layout ends at 0xA9000).
const FLASH_BIN_LONG: usize = 0xA9000;
/// The idle time after which the test host gives up on a session.
const IDLE_MS: u64 = 3000;

fn package() -> Vec<u8> {
    std::fs::read(build_dir().join("fmzx.fwsc")).expect("read build/fm1/fmzx.fwsc (tools/make-packages)")
}

/// The UFW entry of type `typ` (0 flash.bin, 100 ota.bin) of a package: its data in the logical
/// image.
fn ufw_entry(fwsc: &[u8], typ: u16) -> Vec<u8> {
    let img = logical_image(fwsc);
    let mut hdr = img[..0x40].to_vec();
    fmzx_pack::fwsc::enc(&mut hdr, 0xFFFF);
    let n = u16::from_le_bytes([hdr[8], hdr[9]]) as usize;
    for i in 0..n {
        let mut e = img[0x40 + i * 0x50..0x90 + i * 0x50].to_vec();
        fmzx_pack::fwsc::enc(&mut e, 0xFFFF);
        if u16::from_le_bytes([e[0], e[1]]) == typ {
            let off = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let len = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            return img[off..off + len].to_vec();
        }
    }
    panic!("no UFW entry of type {typ}");
}

fn opts() -> ServeOpts {
    ServeOpts { stop_after: None, corrupt_at: None, max_ms: IDLE_MS, midi_noise: 0 }
}

/// The first flash offset outside the OTA area that differs from `before`.
fn changed_outside_ota(m: &Machine, before: &[u8]) -> Option<usize> {
    (0..before.len()).find(|&i| !OTA.contains(&i) && m.soc.flash.mem[i] != before[i])
}

fn ota_erased(m: &Machine) -> bool {
    m.soc.flash.mem[OTA].iter().all(|&b| b == 0xFF)
}

/// Runs until the session is over (`usbota_run_update`, watched before it started, returned) and
/// fmzx_stats shows view `v`, for at most `max_ms`.
fn wait_view(m: &mut Machine, v: i32, max_ms: u64) {
    let end = m.ms() + max_ms;
    while m.ms() < end {
        m.run_ms(10);
        if m.cost_of("usbota_run_update").len() == 1 && m.stats().view == v as u32 {
            return;
        }
    }
    panic!("not back in view {v} {max_ms} ms on (view {}, sessions over {})", m.stats().view, m.cost_of("usbota_run_update").len());
}

/// A whole session from the picker: step 1 served, the commit reached, flash changed in the OTA
/// area only, the package's loader staged there.
fn installs(m: &mut Machine, what: &str) {
    let fwsc = package();
    let before = m.soc.flash.mem.clone();
    m.watch("fmzx_enter_update");
    assert_eq!(upgrade_step1(m, &fwsc, &opts()), Serve::Success, "{what}");
    assert!(m.run_until("fmzx_enter_update", 2000), "{what}: fmzx_enter_update not reached");
    assert_eq!(changed_outside_ota(m, &before), None, "{what}: flash written outside the OTA area");
    let ota = ufw_entry(&fwsc, 100);
    let fl = &m.soc.flash.mem;
    // the loader's body after its own 32-byte head; a head naming it LOADER.BIN in front
    assert_eq!(&fl[0xE0020..0xE0000 + ota.len()], &ota[0x20..], "{what}: the staged loader");
    assert_eq!(&fl[0xE0010..0xE001A], b"LOADER.BIN", "{what}: the staged loader's head");
    assert_ne!(&fl[0xE4F00..0xE4F08], &[0xFF; 8], "{what}: no update record");
}

#[test]
fn midi_during_the_session_does_not_fail_it() {
    let mut m = boot_plugged();
    m.watch("fmzx_enter_update");
    let o = ServeOpts { midi_noise: 3, ..opts() };
    assert_eq!(upgrade_step1(&mut m, &package(), &o), Serve::Success, "notes and clock sent during the session");
    assert!(m.run_until("fmzx_enter_update", 2000), "fmzx_enter_update not reached");
}

#[test]
fn update_stages_loader_and_commits() {
    let mut m = boot_plugged();
    let ops = m.soc.flash.ops;
    installs(&mut m, "Puya");
    println!("flash erases and programs in the session: {}", m.soc.flash.ops - ops);
}

#[test]
fn abandoned_session_returns_paused() {
    let mut m = boot_plugged();
    start_castaway(&mut m);
    // the upgrade message while the game sounds: every 10 ms of the last 100 ms (its tape's tones)
    let loud = |m: &Machine| {
        let a = m.audio_left();
        a.len() > 4410 && a[a.len() - 4410..].chunks(441).all(|c| c.iter().any(|s| s.unsigned_abs() > 1000))
    };
    m.run_ms(1000);
    for _ in 0..1000 {
        if loud(&m) {
            break;
        }
        m.run_ms(10);
    }
    assert!(loud(&m), "Castaway silent for 10 s");
    let before = m.soc.flash.mem.clone();
    m.watch("usbota_run_update");
    let (ms0, a0) = (m.ms(), m.audio_left().len());
    let o = ServeOpts { stop_after: Some(20), ..opts() };
    assert_eq!(upgrade_step1(&mut m, &package(), &o), Serve::Abandoned);
    // ota.c gives up a read after 3 tries of 5 s; the failure is shown for 1.5 s
    wait_view(&mut m, view::PAUSED, 20_000);
    let start = m.first_hit_ms("usbota_run_update").expect("no session");
    m.run_ms(300);
    assert_eq!(m.stats().view, view::PAUSED as u32, "the game paused");
    assert!(ota_erased(&m), "the OTA area not erased");
    assert_eq!(changed_outside_ota(&m, &before), None, "flash written outside the OTA area");
    assert_eq!(m.resets(), 0);
    // silent from the session's start: after the two DMA halves already filled (256 samples) and
    // the output's decay to within 15 of 0 (under 256 more)
    let rate = 44_117.647 / 1000.0;
    let from = a0 + ((start - ms0) as f64 * rate) as usize + 512;
    let quiet = &m.audio_left()[from..];
    assert!(quiet.len() > 44_117 * 15, "the capture is short: {} samples", quiet.len());
    let loud = quiet.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    assert!(loud <= 16, "audio during the session: peak {loud}");
}

/// The reply with the loader's head corrupted (request 6: requests 1-3 are the layout check's reads
/// of the entry list and the loader's head, 4-5 the session's entry list): refused before anything
/// is erased, the picker on the panel again as it was.
#[test]
fn corrupted_reply_refused() {
    let mut m = boot_plugged_with(OPT_565);
    m.watch("fmzx_enter_update");
    m.watch("usbota_run_update");
    let (before, ops) = (m.soc.flash.mem.clone(), m.soc.flash.ops);
    let panel = m.lcd_rgb565().to_vec();
    let o = ServeOpts { corrupt_at: Some(6), ..opts() };
    assert_ne!(upgrade_step1(&mut m, &package(), &o), Serve::Success);
    wait_view(&mut m, view::PICKER, 20_000);
    m.run_ms(500);
    assert!(m.lcd_rgb565() == &panel[..], "the panel is not the picker it showed before the session");
    // ota_session's unstaging erases (the area is erased already): no program
    assert!(m.soc.flash.ops - ops <= 5, "{} flash operations", m.soc.flash.ops - ops);
    assert_eq!(m.hits("fmzx_enter_update"), 0, "committed");
    assert!(ota_erased(&m), "the OTA area not erased");
    assert_eq!(changed_outside_ota(&m, &before), None, "flash written outside the OTA area");
    assert_identity(handshake(&mut m, false, 500), "after the refused session");
}

/// A reply inside the loader's body corrupted: staged, the CRC wrong, nothing committed.
#[test]
fn corrupted_body_refused() {
    let mut m = boot_plugged();
    m.watch("fmzx_enter_update");
    m.watch("usbota_run_update");
    let (before, ops) = (m.soc.flash.mem.clone(), m.soc.flash.ops);
    // requests 1-7: the layout check's reads, the entry list, the loader's head twice; 8-33 its body
    let o = ServeOpts { corrupt_at: Some(25), ..opts() };
    assert_ne!(upgrade_step1(&mut m, &package(), &o), Serve::Success);
    wait_view(&mut m, view::PICKER, 20_000);
    assert!(m.soc.flash.ops - ops > 10, "not refused while staging: {} flash operations", m.soc.flash.ops - ops);
    assert_eq!(m.hits("fmzx_enter_update"), 0, "committed");
    assert!(ota_erased(&m), "the OTA area not erased");
    assert_eq!(changed_outside_ota(&m, &before), None, "flash written outside the OTA area");
    assert_identity(handshake(&mut m, false, 500), "after the refused session");
}

/// A failed session in a paused game: the panel afterwards is the paused game as before it.
#[test]
fn failed_session_repaints_paused() {
    let mut m = boot_plugged_with(OPT_565);
    start_castaway(&mut m);
    m.run_ms(1000);
    press_to(&mut m, PLAY, view::PAUSED);
    m.run_ms(500);
    m.watch("usbota_run_update");
    let panel = m.lcd_rgb565().to_vec();
    let o = ServeOpts { corrupt_at: Some(6), ..opts() };
    assert_ne!(upgrade_step1(&mut m, &package(), &o), Serve::Success);
    wait_view(&mut m, view::PAUSED, 20_000);
    m.run_ms(500);
    assert!(m.lcd_rgb565() == &panel[..], "the panel is not the paused game it showed before the session");
    assert!(ota_erased(&m), "the OTA area not erased");
}

#[test]
fn second_flash_chip_installs() {
    let mut m = Machine::boot(firmware(), OPT);
    m.soc.flash.jedec = [0xB3, 0x60, 0x14];
    m.soc.usb_plug();
    m.run_ms(1500);
    installs(&mut m, "UC25HQ80");
}

#[test]
fn unknown_flash_refused() {
    let mut m = Machine::boot(firmware(), OPT);
    m.soc.flash.jedec = [0xEF, 0x40, 0x14];
    m.soc.usb_plug();
    m.run_ms(1500);
    let ops = m.soc.flash.ops;
    m.watch("usbota_run_update");
    assert!(matches!(upgrade_step1(&mut m, &package(), &opts()), Serve::Refused(_)), "a session on an unknown part");
    assert_eq!(m.hits("usbota_run_update"), 0, "a session on an unknown part");
    assert_eq!(m.soc.flash.ops, ops, "flash erased or programmed");
    let id = handshake(&mut m, true, 500).expect("no answer to the handshake");
    assert_eq!(id.text, "FM-1_048");
    assert_eq!(id.jedec, Some([0xEF, 0x40, 0x14]));
}

#[test]
fn soft_key_enters_uboot() {
    let mut m = boot_plugged();
    m.watch("fmzx_enter_uboot");
    let t0 = m.ms();
    soft_uboot(&mut m);
    m.run_ms(200);
    let at = m.first_hit_ms("fmzx_enter_uboot").expect("fmzx_enter_uboot not reached within 200 ms");
    assert!(at - t0 <= 200);
}

/// The package with its flash.bin replaced by `flash`, the same loader.
fn with_flash(fwsc: &[u8], flash: &[u8]) -> Vec<u8> {
    fmzx_pack::fwsc::ufw(flash, &ufw_entry(fwsc, 100), "FM-1_048").expect("build the package")
}

/// The package with no flash.bin: its entry's type changed (0 to 1), the entry list's and the
/// header's CRCs made again, the identity markers kept.
fn without_flash(fwsc: &[u8]) -> Vec<u8> {
    use fmzx_pack::fwsc::{crc16, enc};
    let mut img = logical_image(fwsc);
    let mut hdr = img[..0x40].to_vec();
    enc(&mut hdr, 0xFFFF);
    let n = u16::from_le_bytes([hdr[8], hdr[9]]) as usize;
    for i in 0..n {
        let r = 0x40 + i * 0x50..0x90 + i * 0x50;
        let mut e = img[r.clone()].to_vec();
        enc(&mut e, 0xFFFF);
        if e[..2] == [0, 0] {
            e[0] = 1;
        }
        enc(&mut e, 0xFFFF);
        img[r].copy_from_slice(&e);
    }
    let c = crc16(&img[0x40..0x40 + n * 0x50]);
    hdr[2..4].copy_from_slice(&c.to_le_bytes());
    let c = crc16(&hdr[2..]);
    hdr[..2].copy_from_slice(&c.to_le_bytes());
    enc(&mut hdr, 0xFFFF);
    img[..0x40].copy_from_slice(&hdr);
    let mut out = Vec::new();
    for i in 0..20 {
        out.extend_from_slice(&img[i * 0x2F..(i + 1) * 0x2F]);
        out.push(fwsc[i * 0x30 + 0x2F]);
    }
    out.extend_from_slice(&img[20 * 0x2F..]);
    assert_eq!(logical_image(&out), img);
    out
}

/// `pkg` (felucca-bsp's loader, a flash.bin that loader refuses after the restart) refused by the
/// layout check: back in the picker, no flash erased or programmed.
fn refused_before_erase(pkg: &[u8], what: &str) {
    let mut m = boot_plugged();
    m.watch("fmzx_enter_update");
    m.watch("usbota_run_update");
    let ops = m.soc.flash.ops;
    assert_ne!(upgrade_step1(&mut m, pkg, &opts()), Serve::Success, "{what}");
    wait_view(&mut m, view::PICKER, 20_000);
    assert_eq!(m.hits("fmzx_enter_update"), 0, "{what}: committed");
    assert_eq!(m.soc.flash.ops, ops, "{what}: flash erased or programmed");
}

/// A flash.bin felucca-bsp's loader takes, longer than FM-1 ZX's: past the check, the session reads
/// on after the entry list.
#[test]
fn longer_app_area_passes_the_layout_check() {
    let fwsc = package();
    let mut flash = ufw_entry(&fwsc, 0);
    flash.resize(FLASH_BIN_LONG, 0xFF);
    let mut m = boot_plugged();
    // requests 1-3: the check's reads of the entry list and the loader's head; 4-5 the session's
    let o = ServeOpts { stop_after: Some(3), ..opts() };
    assert_eq!(upgrade_step1(&mut m, &with_flash(&fwsc, &flash), &o), Serve::Abandoned, "refused by the check");
}

#[test]
fn other_board_refused_before_erase() {
    let fwsc = package();
    let flash = ufw_entry(&fwsc, 0);
    assert_eq!(flash.len(), FLASH_BIN);
    refused_before_erase(&with_flash(&fwsc, &flash[..0x80000]), "flash.bin 0x80000 bytes");
}

#[test]
fn flash_bin_past_the_flash_refused_before_erase() {
    let fwsc = package();
    let mut flash = ufw_entry(&fwsc, 0);
    flash.resize(0x101000, 0xFF);
    refused_before_erase(&with_flash(&fwsc, &flash), "flash.bin 0x101000 bytes");
}

#[test]
fn no_flash_bin_refused_before_erase() {
    refused_before_erase(&without_flash(&package()), "no flash.bin");
}
