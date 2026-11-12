// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! In-game controls in fm1-emu: the game's key map, the HOME menu, pause, the on-screen
//! keyboard and the LCD speed item. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml` (0 scroll, 1 border, 2 Moon Caves, 3 Castaway); scroll has the default
//! key map (F3 = Kempston left).
//!
//! The Spectrum's key rows and Kempston byte are read from the firmware's `fmzx_keys_now` and
//! `fmzx_kempston_now`: copies of `fmzx_machine.keys` and `.kempston` made right after
//! `zx_set_keys`/`zx_set_kempston`. The machine itself cannot be read at a host-computed offset:
//! `zx_machine` starts with the Z80 core's pointers (8 bytes on the host, 4 on the device), so
//! the zx-sys layout of the fields after them differs from the device's.

use emu_gate::fw::{assert_golden, build_dir, BootOpt, Firmware, Machine};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::view;

const OCT_UP: u8 = 1;
const HOME: u8 = 8;
const SAVE: u8 = 9;
const PLAY: u8 = 12;
const NOTE_F3: u8 = 14;
const SELECT: u8 = 0;
const ALGORITHM: u8 = 1;
const SCROLL: u32 = 0;
const BORDER: u32 = 1;
const KEMPSTON_LEFT: u32 = 1 << 1;

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

/// Boots with the pack, starts game `index` from the picker and lets it run 300 ms.
fn start(opt: BootOpt, index: u32) -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack()), opt);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    if index > 0 {
        m.turn(SELECT, index as i32);
        m.run_ms(200);
    }
    press_to(&mut m, PLAY, view::GAME);
    assert_eq!(m.stats().game, index);
    m.run_ms(300);
    m
}

/// Presses `key` and waits (up to 300 ms) until fmzx_stats.view is `view`.
fn press_to(m: &mut Machine, key: u8, view: i32) {
    m.press(key, 30);
    for _ in 0..300 {
        if m.stats().view == view as u32 {
            return;
        }
        m.run_ms(1);
    }
    panic!("view {} after pressing {key}, expected {view}", m.stats().view);
}

/// Port 0x1F as the Spectrum reads it now.
fn kempston(m: &Machine) -> u32 {
    m.peek32(m.symbol("fmzx_kempston_now"))
}

/// The Spectrum's eight key rows (row r = A(8+r), bit set = pressed).
fn keys(m: &Machine) -> [u8; 8] {
    let a = m.symbol("fmzx_keys_now");
    let mut k = [0u8; 8];
    k[..4].copy_from_slice(&m.peek32(a).to_le_bytes());
    k[4..].copy_from_slice(&m.peek32(a + 4).to_le_bytes());
    k
}

/// Spectrum frames keep coming at about 50 Hz for the next 300 ms.
fn assert_frames_run(m: &mut Machine, what: &str) {
    let f0 = m.stats().frames;
    m.run_ms(300);
    let f = m.stats().frames - f0;
    assert!((13..=17).contains(&f), "{what}: {f} frames in 300 ms");
}

fn assert_frames_stopped(m: &mut Machine, what: &str) {
    let f = m.stats().frames;
    m.run_ms(200);
    assert_eq!(m.stats().frames, f, "{what}: frames ran");
}

/// From the game: HOME, then `downs` items down the menu (OCT+).
fn menu_item(m: &mut Machine, downs: u32) {
    press_to(m, HOME, view::MENU);
    for _ in 0..downs {
        m.press(OCT_UP, 30);
        m.run_ms(50);
    }
}

#[test]
fn note_key_reaches_kempston() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    assert_eq!(kempston(&m), 0);
    m.hold(NOTE_F3, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT, "F3 held: Kempston left");
    m.hold(NOTE_F3, false);
    m.run_ms(100);
    assert_eq!(kempston(&m), 0, "F3 released");
}

#[test]
fn home_menu_pauses_and_releases() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    m.hold(NOTE_F3, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);

    press_to(&mut m, HOME, view::MENU);
    m.run_ms(50);
    assert_eq!(kempston(&m), 0, "Kempston released in the menu");
    assert_frames_stopped(&mut m, "in the HOME menu");
    m.hold(NOTE_F3, false);
    m.run_ms(100);
    assert_eq!(kempston(&m), 0);

    press_to(&mut m, HOME, view::GAME); // BACK resumes
    assert_frames_run(&mut m, "after HOME -> HOME");
    assert_eq!(kempston(&m), 0, "F3 released in the menu: still 0 after the resume");

    // the key map works again after the resume
    m.hold(NOTE_F3, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    m.hold(NOTE_F3, false);
}

#[test]
fn pause_releases_keys() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    m.hold(NOTE_F3, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    press_to(&mut m, PLAY, view::PAUSED);
    m.run_ms(50);
    assert_eq!(kempston(&m), 0, "Kempston released while paused");
    assert_frames_stopped(&mut m, "paused");
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT, "F3 still held after the pause");
}

/// The host's composition of the HOME menu over the same pack.
fn host_menu(pack: &[u8]) -> Vec<u16> {
    let count = zx_sys::pack_open(pack).expect("pack opens on the host");
    let mut ui = zx_sys::Ui::new();
    let mut menu = zx_sys::Menu::new(count);
    menu.set_divider(4);
    assert_eq!(menu.event(zx_sys::ev::OK).0, zx_sys::cmd::START);
    menu.set_view(view::GAME);
    menu.event(zx_sys::ev::HOME);
    assert_eq!(menu.view(), view::MENU);
    ui.draw(&menu, Some(pack));
    (0..240)
        .flat_map(|r| {
            let bytes = zx_sys::PixFormat::Rgb565.pack(&ui.compose_row(r));
            bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn menu_golden() {
    let mut m = start(BootOpt { rgb565: true, divider: 4 }, SCROLL);
    press_to(&mut m, HOME, view::MENU);
    m.run_ms(300);
    assert_eq!(m.stats().pending, 0, "LCD rows still pending");
    assert_golden(m.lcd_rgb565(), &golden("menu.png"), &build_dir().join("menu.actual.png"));
    assert!(m.lcd_rgb565() == &host_menu(pack())[..], "the LCD differs from the host's HOME menu");
}

#[test]
fn osk_types_key() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    menu_item(&mut m, 2); // "Keyboard"
    press_to(&mut m, PLAY, view::OSK);
    m.run_ms(100);
    assert_frames_run(&mut m, "under the keyboard");
    // from 1 (top left): ALGORITHM down to Q, SELECT left (wrapping) to P
    m.turn(ALGORITHM, 1);
    m.run_ms(50);
    m.turn(SELECT, -1);
    m.run_ms(50);
    assert_eq!(keys(&m), [0; 8], "the keyboard's encoders reached the key map");

    m.hold(PLAY, true);
    m.run_ms(100);
    assert_eq!(keys(&m), [0, 0, 0, 0, 0, 1, 0, 0], "P (row 5 bit 0) while PLAY is held");
    m.hold(PLAY, false);
    m.run_ms(100);
    assert_eq!(keys(&m), [0; 8], "P released with PLAY");
    assert_eq!(m.stats().view, view::OSK as u32);

    // SAVE is CAPS SHIFT for the next key (and not the key map's 9)
    m.press(SAVE, 30);
    m.run_ms(50);
    assert_eq!(keys(&m), [0; 8], "SAVE reached the key map");
    m.hold(PLAY, true);
    m.run_ms(100);
    assert_eq!(keys(&m), [1, 0, 0, 0, 0, 1, 0, 0], "CAPS SHIFT + P");
    m.hold(PLAY, false);
    m.run_ms(100);
    assert_eq!(keys(&m), [0; 8]);

    // mapped note keys keep working under the keyboard
    m.hold(NOTE_F3, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    m.hold(NOTE_F3, false);

    press_to(&mut m, HOME, view::GAME); // HOME closes the keyboard
    assert_frames_run(&mut m, "after the keyboard");
}

/// (Spectrum frames, LCD bytes sent) over the next `ms`.
fn lcd_bytes_over(m: &mut Machine, ms: u64) -> (u32, u32) {
    let s0 = m.stats();
    m.run_ms(ms);
    let s1 = m.stats();
    (s1.frames - s0.frames, s1.lcd.bytes - s0.lcd.bytes)
}

#[test]
fn osk_rows_not_resent() {
    // border.sna paints the same border stripes every frame, so the game itself sends next to
    // nothing. Under the keyboard, its 40 rows (paper rows 200-215, border rows 216-239) show
    // the unchanged keyboard grid and must not go out again every frame: at 360 bytes a row
    // (RGB444) the border rows alone would be 24 * 360 bytes a frame.
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, BORDER);
    m.run_ms(500);
    let (f, game) = lcd_bytes_over(&mut m, 1000);
    let game_pf = game / f;
    menu_item(&mut m, 2); // "Keyboard"
    press_to(&mut m, PLAY, view::OSK);
    m.run_ms(300);
    let (f, osk) = lcd_bytes_over(&mut m, 1000);
    let osk_pf = osk / f;
    println!("LCD bytes a frame: game {game_pf}, under the keyboard {osk_pf}");
    assert!(f >= 45, "{f} frames under the keyboard in 1 s");
    assert!(osk_pf < game_pf + 24 * 360 / 4, "the keyboard rows go out again every frame: {osk_pf} bytes a frame, {game_pf} in the game");
    // closing the keyboard repaints the whole panel, its rows with the game's picture
    let b0 = m.stats().lcd.bytes;
    press_to(&mut m, HOME, view::GAME);
    m.run_ms(300);
    let s = m.stats();
    assert_eq!(s.pending, 0, "LCD rows still pending");
    assert!(s.lcd.bytes - b0 >= 240 * 360, "no full repaint after the keyboard: {} bytes", s.lcd.bytes - b0);
}

#[test]
fn view_change_starts_a_frame_at_once() {
    // leaving the game, the menu's first LCD frame (and its UI rows) does not wait for the up to
    // 40 ms the game's audio pacing left on the frame timer: it comes in the same main-loop pass
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    for i in 0..5 {
        m.run_ms(7 * i);
        m.hold(HOME, true);
        let mut before = m.stats();
        let mut t = 0;
        while m.stats().view == view::GAME as u32 {
            assert!(t < 100, "HOME did not open the menu");
            before = m.stats();
            m.run_ms(1);
            t += 1;
        }
        let s = m.stats();
        m.run_ms(30);
        m.hold(HOME, false);
        m.run_ms(50);
        assert_eq!(s.view, view::MENU as u32);
        assert!(s.lcd.frames > before.lcd.frames, "no LCD frame within 1 ms of the menu (try {i})");
        press_to(&mut m, HOME, view::GAME);
        m.run_ms(100);
    }
}

#[test]
fn divider_menu() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    assert_eq!((m.stats().divider, m.stats().budget), (4, 30_000));
    press_to(&mut m, HOME, view::MENU);
    m.turn(SELECT, 4); // "LCD speed: 4"
    m.run_ms(100);
    m.press(PLAY, 30);
    m.run_ms(100);
    let s = m.stats();
    assert_eq!(s.view, view::MENU as u32, "{s:?}");
    assert_eq!((s.divider, s.budget), (2, 50_000), "{s:?}");
}

#[test]
fn leave_and_restart() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    assert_frames_run(&mut m, "running");
    press_to(&mut m, HOME, view::MENU);
    m.turn(ALGORITHM, -1); // wraps to "Back to games"
    m.run_ms(100);
    press_to(&mut m, PLAY, view::PICKER);
    let before = m.stats().frames;
    assert!(before > 15, "{before} frames before leaving");
    assert_frames_stopped(&mut m, "in the picker");

    // PLAY starts the game the picker still has selected; its first frames run back to back
    // until the audio FIFO holds AUDIO_LOW samples, so a few are in by the time the view changed
    press_to(&mut m, PLAY, view::GAME);
    let s = m.stats();
    assert_eq!(s.game, SCROLL);
    assert!(s.frames <= 5 && s.frames < before, "frames did not restart from 0: {s:?}");
    assert_frames_run(&mut m, "after the restart");
}
