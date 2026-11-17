// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! In-game controls in fm1-emu: the game's key map, the HOME menu, pause, the on-screen
//! keyboard, the LCD speed item, the Sound item and the Screen item. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml` (0 scroll, 1 border, 2 Castaway; the test programs `tape`/`tune` are
//! found by id); scroll has the default key map (C#4 = Kempston left).
//!
//! The Spectrum's key rows and Kempston byte are read from the firmware's `fmzx_keys_now` and
//! `fmzx_kempston_now`: copies of `fmzx_machine.keys` and `.kempston` made right after
//! `zx_set_keys`/`zx_set_kempston`. The machine itself cannot be read at a host-computed offset:
//! `zx_machine` starts with the Z80 core's pointers (8 bytes on the host, 4 on the device), so
//! the zx-sys layout of the fields after them differs from the device's.

use emu_gate::budgets::{
    choose_screen, choose_sound, screen_item_in, screen_mode, sound_mode, SCREEN_BORDER, SCREEN_CROP, SCREEN_FIT,
    SCREEN_FIT_SMOOTH, SCREEN_ITEM, SOUND_FM, SOUND_FM_B, SOUND_ITEM,
};
use emu_gate::fw::{assert_golden, build_dir, BootOpt, Firmware, Machine};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::view;

const OCT_UP: u8 = 1;
const HOME: u8 = 8;
const SAVE: u8 = 9;
const PLAY: u8 = 12;
const NOTE_CS4: u8 = 22;
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
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT, "C#4 held: Kempston left");
    m.hold(NOTE_CS4, false);
    m.run_ms(100);
    assert_eq!(kempston(&m), 0, "C#4 released");
}

#[test]
fn home_menu_pauses_and_releases() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);

    press_to(&mut m, HOME, view::MENU);
    m.run_ms(50);
    assert_eq!(kempston(&m), 0, "Kempston released in the menu");
    assert_frames_stopped(&mut m, "in the HOME menu");
    m.hold(NOTE_CS4, false);
    m.run_ms(100);
    assert_eq!(kempston(&m), 0);

    press_to(&mut m, HOME, view::GAME); // BACK resumes
    assert_frames_run(&mut m, "after HOME -> HOME");
    assert_eq!(kempston(&m), 0, "C#4 released in the menu: still 0 after the resume");

    // the key map works again after the resume
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    m.hold(NOTE_CS4, false);
}

#[test]
fn pause_releases_keys() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    press_to(&mut m, PLAY, view::PAUSED);
    m.run_ms(50);
    assert_eq!(kempston(&m), 0, "Kempston released while paused");
    assert_frames_stopped(&mut m, "paused");
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT, "C#4 still held after the pause");
}

/// The host's composition of the HOME menu over the same pack, with the Sound item cycled
/// `sound` times (0: Beeper), the Screen item `screen` times (0: Crop), and item `item` selected;
/// from Beeper and Crop, as the firmware starts with the test boot options.
fn host_menu_at(pack: &[u8], sound: u32, screen: u32, item: u32) -> Vec<u16> {
    let count = zx_sys::pack_open(pack).expect("pack opens on the host");
    let mut ui = zx_sys::Ui::new();
    let mut menu = zx_sys::Menu::new(count);
    menu.set_divider(4);
    menu.set_choices(0, 0);
    assert_eq!(menu.event(zx_sys::ev::OK).0, zx_sys::cmd::START);
    menu.set_view(view::GAME);
    menu.event(zx_sys::ev::HOME);
    // the Sound item cycles Beeper -> FM: sound -> FM: engines, whose Effects item moves Screen down
    let screen_at = screen_item_in(sound % 3);
    for (cycles, at, cmd) in [(sound, SOUND_ITEM, zx_sys::cmd::SOUND), (screen, screen_at, zx_sys::cmd::SCREEN)] {
        if cycles > 0 {
            for _ in 0..at {
                menu.event(zx_sys::ev::DOWN);
            }
            for _ in 0..cycles {
                assert_eq!(menu.event(zx_sys::ev::OK).0, cmd);
            }
            menu.set_view(view::GAME);
            menu.event(zx_sys::ev::HOME);
        }
    }
    for _ in 0..item {
        menu.event(zx_sys::ev::DOWN);
    }
    assert_eq!(menu.view(), view::MENU);
    ui.draw(&menu, Some(pack));
    (0..240)
        .flat_map(|r| {
            let bytes = zx_sys::ui_pack_row(zx_sys::PixFormat::Rgb565, &ui.compose_row(r));
            bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()
        })
        .collect()
}

/// As [`host_menu_at`] with the first item selected.
fn host_menu(pack: &[u8], sound: u32, screen: u32) -> Vec<u16> {
    host_menu_at(pack, sound, screen, 0)
}

#[test]
fn menu_golden() {
    let mut m = start(BootOpt { rgb565: true, divider: 4 }, SCROLL);
    press_to(&mut m, HOME, view::MENU);
    m.run_ms(300);
    assert_eq!(m.stats().pending, 0, "LCD rows still pending");
    assert_golden(m.lcd_rgb565(), &golden("menu.png"), &build_dir().join("menu.actual.png"));
    assert!(m.lcd_rgb565() == &host_menu(pack(), 0, 0)[..], "the LCD differs from the host's HOME menu");
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
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    m.hold(NOTE_CS4, false);

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

/// Holds HOME in the game at phase `i` of [`VIEW_PHASES`]: `i` ms and `i % 4` quarter-ms after a
/// Spectrum frame starts, so the phases cover the 20 ms LCD frame twice, the 40 ms the game's
/// audio pacing leaves on the frame timer, and four points of each millisecond (the 1 ms steps the
/// tests read fmzx_stats at against the main loop's passes).
fn home_at_phase(m: &mut Machine, i: u64) {
    assert!(m.run_until("game_frame", 100), "no game frame");
    m.run_us(i * 1000 + i % 4 * 250);
    m.hold(HOME, true);
}

const VIEW_PHASES: u64 = 41;

/// Releases HOME in the menu and resumes the game, its pacing settled again.
fn menu_back_to_game(m: &mut Machine) {
    m.run_ms(30);
    m.hold(HOME, false);
    m.run_ms(50);
    press_to(m, HOME, view::GAME);
    m.run_ms(100);
}

#[test]
fn view_change_starts_a_frame_at_once() {
    // leaving the game, the menu's first LCD frame (and its UI rows) does not wait for the up to
    // 40 ms the game's audio pacing left on the frame timer: it comes in the same main-loop pass,
    // so fmzx_stats, read every 1 ms, never shows the menu before that frame (at any phase)
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    let mut late = Vec::new();
    for i in 0..VIEW_PHASES {
        home_at_phase(&mut m, i);
        let mut before = m.stats();
        let mut t = 0;
        while m.stats().view == view::GAME as u32 {
            assert!(t < 100, "HOME did not open the menu (phase {i})");
            before = m.stats();
            m.run_ms(1);
            t += 1;
        }
        let s = m.stats();
        assert_eq!(s.view, view::MENU as u32);
        if s.lcd.frames == before.lcd.frames {
            late.push(i);
        }
        menu_back_to_game(&mut m);
    }
    assert!(late.is_empty(), "no LCD frame within 1 ms of the menu at phases {late:?} of {VIEW_PHASES}");
}

#[test]
fn view_change_frame_latency() {
    // the same, timed: from the HOME press's event to the menu's first LCD frame (the scheduler's
    // new frame) at most 1 ms, at every phase
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    let per_ms = m.cycles_per_ms();
    let mut worst = 0;
    for i in 0..VIEW_PHASES {
        home_at_phase(&mut m, i);
        assert!(m.run_until("handle_event", 100), "HOME was not handled (phase {i})");
        let c0 = m.cycles();
        assert!(m.run_until("lcd_sched_begin_frame", 100), "no LCD frame after HOME (phase {i})");
        let us = (m.cycles() - c0) * 1000 / per_ms;
        m.run_ms(1);
        assert_eq!(m.stats().view, view::MENU as u32, "phase {i}");
        assert!(us <= 1000, "the menu's first LCD frame {us} us after HOME (phase {i})");
        worst = worst.max(us);
        menu_back_to_game(&mut m);
    }
    println!("HOME to the menu's first LCD frame: at most {worst} us over {VIEW_PHASES} phases");
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
fn note_key_held_through_the_picker() {
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, SCROLL);
    m.hold(NOTE_CS4, true);
    m.run_ms(100);
    assert_eq!(kempston(&m), KEMPSTON_LEFT);
    press_to(&mut m, HOME, view::MENU);
    m.turn(ALGORITHM, -1); // wraps to "Back to games"
    m.run_ms(100);
    press_to(&mut m, PLAY, view::PICKER);
    for _ in 0..10 {
        m.run_ms(30);
        assert_eq!(kempston(&m), 0, "Kempston in the picker with C#4 held");
        assert_eq!(keys(&m), [0; 8], "Spectrum keys in the picker with C#4 held");
    }

    // a new game with C#4 still held: left only while C#4 is down, released with it
    press_to(&mut m, PLAY, view::GAME);
    for _ in 0..5 {
        m.run_ms(20);
        assert!(matches!(kempston(&m), 0 | KEMPSTON_LEFT), "Kempston {:#x} with C#4 held", kempston(&m));
    }
    m.hold(NOTE_CS4, false);
    m.run_ms(100);
    assert_eq!(kempston(&m), 0, "C#4 released in the new game");
    assert_frames_run(&mut m, "the new game");
    assert_eq!(kempston(&m), 0);
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

/// The MASTER pot (fm1-emu ADC channel).
const MASTER_ADC: u8 = 4;

/// MASTER at full (unity gain), in effect within 300 ms.
fn master_full(m: &mut Machine) {
    m.set_adc(MASTER_ADC, 1023);
    m.run_ms(300);
}

/// From the game: HOME, "Back to games" (the last item), the picker.
fn leave_game(m: &mut Machine) {
    press_to(m, HOME, view::MENU);
    m.turn(ALGORITHM, -1);
    m.run_ms(100);
    press_to(m, PLAY, view::PICKER);
}

/// The host's render of pack entry `index` (a snapshot) from its load, `frames` frames through
/// the beeper and the mixer in the FM mode `mode` (`SND_MODE_FM_*`).
fn host_fm(index: u32, mode: u8, frames: usize) -> Vec<i16> {
    let g = zx_sys::pack_game_at(pack(), index).expect("pack entry");
    let sna = g.extract(zx_sys::PACK_MAX_RAW as usize).expect("extract");
    let mut zx = zx_sys::Machine::new();
    zx.load_sna(&sna).expect("load");
    let mut b = zx_sys::Beeper::new(750_000, 17);
    let mut mix = zx_sys::SndMixer::new(750_000, 17);
    let mut out = Vec::new();
    for _ in 0..frames {
        zx.run_frame();
        let level = zx.edges_start_level();
        b.set_level(level);
        let beeper = b.render(zx.edges());
        out.extend(mix.frame(mode, Some(&zx), zx.edges(), level, &beeper));
    }
    out
}

/// Samples of a 20 ms frame, about (750000/17 Hz).
const FRAME_SAMPLES: usize = 882;

/// The audio from `mark` (a sample count taken before the game was (re)started in the FM mode
/// `mode`) is silence until the new game's first sample, then exactly the host's FM render of
/// `index` from its load: three frames of silence (the mixer's delay ring and decoder start
/// empty) and then the game's own sound, 60 ms late. Nothing of the game before plays.
fn assert_fresh_fm_start(m: &mut Machine, mark: usize, index: u32, mode: u8, what: &str) {
    m.run_ms(1000);
    let host = host_fm(index, mode, 60);
    let dev = &m.audio_left()[mark..];
    let loud = |a: &[i16]| a.iter().position(|x| x.unsigned_abs() > 64);
    let h = loud(&host).expect("the host render is silent");
    assert!(h >= 3 * FRAME_SAMPLES - 3, "{what}: the host render starts {h} samples in, not 3 frames");
    let d = loud(dev).unwrap_or_else(|| panic!("{what}: no sound after the start"));
    assert!(d >= h, "{what}: sound {d} samples after the start, before the host's first at {h}");
    let off = d - h;
    let before = dev[..off].iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
    assert!(before <= 15, "{what}: a sample of {before} before the new game's first one");
    let n = 40 * FRAME_SAMPLES;
    assert!(dev.len() >= off + n, "{what}: only {} samples after the start", dev.len() - off);
    let first_diff = dev[off..off + n].iter().zip(&host[..n]).position(|(a, b)| a != b);
    assert_eq!(first_diff, None, "{what}: the device differs from the host's FM render of a fresh start");
}

#[test]
fn sound_choice_survives_reset_and_new_game() {
    let mut m = start(BootOpt { rgb565: true, divider: 4 }, BORDER);
    master_full(&mut m);
    assert_eq!(sound_mode(&m), 0, "Beeper with the test boot options");
    // FM: sound, then FM: engines, each chosen in border (with scroll the other game)
    sound_choice_holds(&mut m, SOUND_FM, BORDER, SCROLL);
    press_to(&mut m, HOME, view::GAME);
    m.run_ms(300);
    leave_game(&mut m);
    m.turn(SELECT, BORDER as i32 - SCROLL as i32);
    m.run_ms(200);
    press_to(&mut m, PLAY, view::GAME);
    assert_eq!((m.stats().game, sound_mode(&m)), (BORDER, SOUND_FM));
    m.run_ms(300);
    sound_choice_holds(&mut m, SOUND_FM_B, BORDER, SCROLL);
}

/// In game `game` (running), the Sound choice `sound` (an FM one) is made; it holds across Reset
/// game (the game restarting afresh in it), the picker and another game, `other`, and the HOME
/// menu shows it (ends there).
fn sound_choice_holds(m: &mut Machine, sound: u32, game: u32, other: u32) {
    let (name, mode, cycles) = if sound == SOUND_FM { ("FM: sound", zx_sys::SND_MODE_FM_A, 1) } else { ("FM: engines", zx_sys::SND_MODE_FM_B, 2) };
    choose_sound(m, sound);
    m.run_ms(300);

    // reset game: the choice holds, and the restarted game starts afresh in it (the mark once
    // the menu has silenced the game)
    press_to(m, HOME, view::MENU);
    m.run_ms(100);
    let mark = m.audio_left().len();
    m.press(OCT_UP, 30); // "Reset game"
    m.run_ms(50);
    press_to(m, PLAY, view::GAME);
    assert_eq!(sound_mode(m), sound, "after Reset game ({name})");
    assert_fresh_fm_start(m, mark, game, mode, &format!("reset ({name})"));

    // back to the picker and another game
    leave_game(m);
    assert_eq!(sound_mode(m), sound, "in the picker ({name})");
    m.turn(SELECT, other as i32 - game as i32);
    m.run_ms(200);
    press_to(m, PLAY, view::GAME);
    assert_eq!((m.stats().game, sound_mode(m)), (other, sound), "after starting another game ({name})");
    m.run_ms(300);

    // the menu shows it
    press_to(m, HOME, view::MENU);
    m.run_ms(300);
    assert_eq!(m.stats().pending, 0, "LCD rows still pending");
    assert!(m.lcd_rgb565() == &host_menu(pack(), cycles, 0)[..], "the LCD differs from the host's HOME menu with {name}");
}

#[test]
fn fm_new_game_starts_silent() {
    // Castaway's title music in FM, then border.sna: none of the music (in the audio FIFO, the
    // mixer's delay ring or the decoder's history) plays into the new game's first frames
    let castaway = 2;
    let mut m = start(BootOpt { rgb565: false, divider: 4 }, castaway);
    master_full(&mut m);
    choose_sound(&mut m, SOUND_FM);
    for _ in 0..1000 {
        if m.stats().frames >= 230 {
            break;
        }
        m.run_ms(10);
    }
    assert!(m.stats().frames >= 230, "Castaway stopped at frame {}", m.stats().frames);
    let a = m.audio_left();
    let tail = &a[a.len() - 4 * FRAME_SAMPLES..];
    assert!(tail.iter().any(|x| x.unsigned_abs() > 1000), "no music playing before leaving");
    leave_game(&mut m);
    m.turn(SELECT, BORDER as i32 - castaway as i32);
    m.run_ms(200);
    let mark = m.audio_left().len();
    press_to(&mut m, PLAY, view::GAME);
    assert_eq!((m.stats().game, sound_mode(&m)), (BORDER, SOUND_FM));
    assert_fresh_fm_start(&mut m, mark, BORDER, zx_sys::SND_MODE_FM_A, "new game");
}

#[test]
fn screen_menu_cycles_four() {
    let mut m = start(BootOpt { rgb565: true, divider: 4 }, SCROLL);
    assert_eq!(screen_mode(&m), SCREEN_CROP, "Crop with the test boot options");
    menu_item(&mut m, SCREEN_ITEM);
    m.run_ms(300);
    assert!(m.lcd_rgb565() == &host_menu_at(pack(), 0, 0, SCREEN_ITEM)[..], "the LCD differs from the host's menu with Screen: Crop");
    for (cycles, mode) in [(1, SCREEN_FIT), (2, SCREEN_FIT_SMOOTH), (3, SCREEN_BORDER), (4, SCREEN_CROP)] {
        m.press(PLAY, 30);
        m.run_ms(300);
        let s = m.stats();
        assert_eq!((s.view, screen_mode(&m)), (view::MENU as u32, mode), "after {cycles} presses");
        assert_eq!(s.pending, 0, "LCD rows still pending");
        assert!(
            m.lcd_rgb565() == &host_menu_at(pack(), 0, cycles, SCREEN_ITEM)[..],
            "the LCD differs from the host's menu after {cycles} presses (Screen: {mode})"
        );
    }
    assert_eq!(sound_mode(&m), 0, "the Screen item changed the Sound choice");
}

#[test]
fn screen_choice_holds() {
    let mut m = start(BootOpt { rgb565: true, divider: 4 }, BORDER);
    choose_screen(&mut m, SCREEN_FIT_SMOOTH);
    m.run_ms(300);
    assert_eq!(screen_mode(&m), SCREEN_FIT_SMOOTH);

    // reset game
    press_to(&mut m, HOME, view::MENU);
    m.run_ms(100);
    m.press(OCT_UP, 30); // "Reset game"
    m.run_ms(50);
    press_to(&mut m, PLAY, view::GAME);
    assert_eq!(screen_mode(&m), SCREEN_FIT_SMOOTH, "after Reset game");
    assert!(m.stats().frames < 10, "the game did not restart: {:?}", m.stats());
    m.run_ms(300);

    // back to the picker and another game
    leave_game(&mut m);
    assert_eq!(screen_mode(&m), SCREEN_FIT_SMOOTH, "in the picker");
    m.turn(SELECT, SCROLL as i32 - BORDER as i32);
    m.run_ms(200);
    press_to(&mut m, PLAY, view::GAME);
    assert_eq!((m.stats().game, screen_mode(&m)), (SCROLL, SCREEN_FIT_SMOOTH), "after starting another game");
    m.run_ms(300);

    // the menu shows it, and the next press cycles on from it
    press_to(&mut m, HOME, view::MENU);
    m.run_ms(300);
    assert_eq!(m.stats().pending, 0, "LCD rows still pending");
    assert!(m.lcd_rgb565() == &host_menu(pack(), 0, 2)[..], "the LCD differs from the host's HOME menu with Screen: Fit smooth");
    for _ in 0..SCREEN_ITEM {
        m.press(OCT_UP, 30);
        m.run_ms(50);
    }
    m.press(PLAY, 30);
    m.run_ms(100);
    assert_eq!(screen_mode(&m), SCREEN_BORDER);
}
