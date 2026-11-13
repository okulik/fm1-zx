// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Running a game in fm1-emu: frame pacing, the LCD's picture of the Spectrum screen, and a
//! game that fails to load. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml` (0 scroll, 1 border, 2 Castaway); the test programs `tape` and `tune`
//! are found by id.

use emu_gate::fw::{assert_golden, build_dir, BootOpt, Firmware, Machine};
use fm1_core::Bus as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PLAY: u8 = 12;
const HOME: u8 = 8;
const OCT_UP: u8 = 1;
const SELECT: u8 = 0;
const SCROLL: u32 = 0;
const BORDER: u32 = 1;

/// Index of the game `id` in `games/games.toml`.
fn game_index(id: &str) -> u32 {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read the manifest");
    let m = fmzx_pack::manifest::parse_manifest(&text).expect("parse the manifest");
    m.game.iter().position(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}")) as u32
}

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

/// Boots with `pack`, selects game `index` and presses PLAY; returns once the view changed.
fn start(fw: &Firmware, opt: BootOpt, index: u32) -> Machine {
    let mut m = Machine::boot(fw, opt);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    if index > 0 {
        m.turn(SELECT, index as i32);
        m.run_ms(200);
    }
    m.press(PLAY, 30);
    m.run_ms(20);
    m
}

#[test]
fn runs_at_50hz() {
    let mut m = start(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 4 }, SCROLL);
    assert_eq!(m.stats().view, zx_sys::view::GAME as u32);
    assert_eq!(m.stats().game, SCROLL);
    m.run_ms(200);
    let s0 = m.stats();
    m.run_ms(10_000);
    let s1 = m.stats();
    assert_eq!(m.resets(), 0);
    let frames = s1.frames - s0.frames;
    println!("10 s: {frames} frames, {s1:?}");
    assert!(frames.abs_diff(500) <= 2, "{frames} frames in 10 s");
    assert_eq!(s1.underruns, s0.underruns, "audio underruns after the first 200 ms");
    assert_eq!(s1.overflows, 0, "audio samples dropped");
}

#[test]
fn runs_without_audio_pll() {
    // fm1_audio_init refuses to start on anything but the stock audio PLL: the games still run,
    // paced by the timer, and silent.
    let mut m = Machine::boot(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 4 });
    m.soc.write32(emu_gate::fw::AUDIO_PLL_STOCK[0].0, 0);
    m.run_ms(1500);
    m.press(PLAY, 30);
    m.run_ms(200);
    let s0 = m.stats();
    assert_eq!(s0.view, zx_sys::view::GAME as u32);
    m.run_ms(2000);
    let frames = m.stats().frames - s0.frames;
    assert!(frames.abs_diff(100) <= 2, "{frames} frames in 2 s");
    assert!(m.audio().iter().all(|&x| x == 0), "audio without the PLL");
}

/// LCD row `row` of the host's composition of `m`, as RGB565 pixels.
fn host_row(m: &zx_sys::Machine, row: usize, crop_x: usize) -> Vec<u16> {
    let bytes = zx_sys::PixFormat::Rgb565.pack(&m.compose_row(row, crop_x));
    bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect()
}

/// Compares the LCD with the host's composition of the same game (`pack` entry `index`, run on
/// the host core for as many frames as the device has run) at the next `times` points where the
/// LCD has caught up. Those points are found at the start of a frame, before zx_run_frame: there
/// the firmware has just updated fmzx_stats (`frames` run, `pending` LCD rows not yet sent).
fn assert_screen_follows_host(m: &mut Machine, pack: &[u8], index: u32, times: usize, name: &str) {
    let g = zx_sys::pack_game_at(pack, index).expect("pack entry");
    let sna = g.extract(zx_sys::PACK_MAX_RAW as usize).expect("extract");
    let mut host = zx_sys::Machine::new();
    host.load_sna(&sna).expect("host loads the game");
    let mut host_frames = 0;
    let (mut checked, mut tries) = (0, 0);
    while checked < times {
        assert!(m.run_until("zx_run_frame", 100), "no Spectrum frame within 100 ms");
        let s = m.stats();
        tries += 1;
        assert!(tries <= 10 * times, "the LCD seldom catches up: {s:?}");
        if s.pending != 0 {
            continue;
        }
        while host_frames < s.frames {
            host.run_frame();
            host_frames += 1;
        }
        let want: Vec<u16> = (0..240).flat_map(|r| host_row(&host, r, g.crop_x() as usize)).collect();
        let got = m.lcd_rgb565();
        let bad: Vec<usize> = (0..240).filter(|&r| got[r * 240..r * 240 + 240] != want[r * 240..r * 240 + 240]).collect();
        if !bad.is_empty() {
            emu_gate::fw::write_png(&build_dir().join(format!("{name}.actual.png")), got);
            emu_gate::fw::write_png(&build_dir().join(format!("{name}.want.png")), &want);
        }
        assert!(bad.is_empty(), "after {} frames, LCD rows {bad:?} differ from the host's", s.frames);
        checked += 1;
    }
}

#[test]
fn screen_matches_host() {
    let opt = BootOpt { rgb565: true, divider: 0 };
    let mut m = start(&firmware().with_pack(pack()), opt, BORDER);
    assert_eq!(m.stats().budget, 150_000);
    m.run_ms(3000);
    assert_eq!(m.stats().view, zx_sys::view::GAME as u32);
    assert_screen_follows_host(&mut m, pack(), BORDER, 8, "screen_matches_host");
}

/// `border.sna` with its main loop replaced: each frame, right after the interrupt, the border
/// steps to the next colour (INC A; AND 7; OUT (FEh),A), so every border row changes every frame.
fn border_steps_sna() -> Vec<u8> {
    let mut sna = fmzx_pack::testprogs::border_sna();
    let code = [0xFB, 0x76, 0x3C, 0xE6, 0x07, 0xD3, 0xFE, 0x18, 0xF7]; // EI; HALT; ...; JR 8000h
    let at = 27 + 0x8000 - 0x4000;
    sna[at..at + code.len()].copy_from_slice(&code);
    sna
}

#[test]
fn border_changes_reach_the_lcd() {
    let pack = one_game_pack("Border steps", &border_steps_sna());
    let mut m = start(&firmware().with_pack(&pack), BootOpt { rgb565: true, divider: 0 }, 0);
    m.run_ms(1000);
    assert_eq!(m.stats().view, zx_sys::view::GAME as u32);
    assert_screen_follows_host(&mut m, &pack, 0, 8, "border_changes_reach_the_lcd");
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// A pack of one stored `.sna` game, `data`, with the default key map.
fn one_game_pack(name: &str, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; zx_sys::PACK_HEADER_SIZE + zx_sys::PACK_ENTRY_SIZE];
    p[..8].copy_from_slice(b"FMZXPACK");
    p[8..10].copy_from_slice(&1u16.to_le_bytes());
    p[10..12].copy_from_slice(&1u16.to_le_bytes());
    let e = zx_sys::PACK_HEADER_SIZE;
    p[e..e + name.len()].copy_from_slice(name.as_bytes());
    p[e + 29] = zx_sys::PACK_SNA;
    p[e + 30] = zx_sys::PACK_STORED;
    p[e + 31] = 8;
    let off = p.len();
    put32(&mut p, e + 32, off as u32);
    put32(&mut p, e + 36, data.len() as u32);
    put32(&mut p, e + 40, data.len() as u32);
    put32(&mut p, e + 44, zx_sys::pack_crc32(data));
    for (k, a) in zx_sys::km_default().iter().enumerate() {
        p[e + 48 + 2 * k] = a[0];
        p[e + 49 + 2 * k] = a[1];
    }
    p.extend_from_slice(data);
    p.resize(p.len().next_multiple_of(4), 0);
    let total = p.len() as u32;
    put32(&mut p, 12, total);
    let crc = zx_sys::pack_crc32(&p[32..]);
    put32(&mut p, 16, crc);
    p
}

#[test]
fn load_failed_message() {
    // A 48K .sna in size and CRC, but with interrupt mode 3, which zx_load_sna rejects.
    let mut sna = fmzx_pack::testprogs::border_sna();
    sna[25] = 3;
    assert_eq!(zx_sys::Machine::new().load_sna(&sna), Err(zx_sys::ZX_ERR_FORMAT));
    let pack = one_game_pack("Broken", &sna);
    assert_eq!(zx_sys::pack_game_at(&pack, 0).and_then(|g| g.extract(zx_sys::PACK_MAX_RAW as usize)).as_deref(), Ok(&sna[..]));

    let mut m = start(&firmware().with_pack(&pack), BootOpt { rgb565: true, divider: 4 }, 0);
    m.run_ms(500);
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::MESSAGE as u32, "{s:?}");
    assert_eq!(s.frames, 0);
    // OK returns to the picker
    m.press(PLAY, 30);
    m.run_ms(100);
    assert_eq!(m.stats().view, zx_sys::view::PICKER as u32);
    assert_eq!(m.stats().frames, 0);
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

/// Spectrum frames keep coming at about 50 Hz for the next 300 ms.
fn assert_frames_run(m: &mut Machine, what: &str) {
    let f0 = m.stats().frames;
    m.run_ms(300);
    let f = m.stats().frames - f0;
    assert!((13..=17).contains(&f), "{what}: {f} frames in 300 ms");
}

#[test]
fn resumes_after_pause_and_menu() {
    use zx_sys::view;
    let mut m = start(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 4 }, BORDER);
    m.run_ms(500);
    assert_frames_run(&mut m, "running");

    press_to(&mut m, PLAY, view::PAUSED);
    m.run_ms(100);
    assert_eq!(m.stats().view, view::PAUSED as u32);
    let f = m.stats().frames;
    m.run_ms(200);
    assert_eq!(m.stats().frames, f, "frames ran while paused");
    press_to(&mut m, PLAY, view::GAME);
    assert_frames_run(&mut m, "after PLAY resumed");

    press_to(&mut m, HOME, view::MENU);
    let f = m.stats().frames;
    m.run_ms(300); // in the HOME menu: no frames, no underruns
    assert_eq!(m.stats().frames, f, "frames ran in the HOME menu");
    assert_eq!(m.stats().view, view::MENU as u32);
    press_to(&mut m, PLAY, view::GAME); // OK on "Resume"
    assert_frames_run(&mut m, "after HOME -> Resume");

    press_to(&mut m, HOME, view::MENU);
    m.run_ms(100);
    m.press(OCT_UP, 30);
    m.run_ms(50);
    m.press(OCT_UP, 30); // down to "Keyboard"
    m.run_ms(50);
    press_to(&mut m, PLAY, view::OSK);
    assert_frames_run(&mut m, "under the on-screen keyboard");
    press_to(&mut m, HOME, view::GAME); // BACK leaves the keyboard
    assert_frames_run(&mut m, "after the keyboard");

    let s = m.stats();
    assert_eq!((s.underruns, s.overflows), (0, 0), "{s:?}");
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

/// The LCD colour (RGB565) of Spectrum border colour `colour`, as the host composes it.
fn border_pixel(colour: u8) -> u16 {
    let mut sna = fmzx_pack::testprogs::scroll_sna();
    sna[26] = colour;
    let mut host = zx_sys::Machine::new();
    host.load_sna(&sna).expect("loads");
    host.run_frame();
    host_row(&host, 0, 0)[0]
}

/// The LCD's border rows (the top and bottom 8; the paper fills the width), all one colour,
/// which is returned; `None` for a mixed border.
fn lcd_border(m: &Machine) -> Option<u16> {
    let lcd = m.lcd_rgb565();
    let c = lcd[0];
    (lcd[..8 * 240].iter().chain(&lcd[232 * 240..]).all(|&p| p == c)).then_some(c)
}

/// Runs until the LCD's border is `colour` or `max_ms` have passed; returns the ms waited.
fn wait_for_border(m: &mut Machine, colour: u8, max_ms: u64) -> u64 {
    let want = border_pixel(colour);
    for ms in 0..max_ms {
        if lcd_border(m) == Some(want) {
            return ms;
        }
        m.run_ms(1);
    }
    panic!("the border did not become {colour} within {max_ms} ms (LCD {:04X?}, {:?})", lcd_border(m), m.stats());
}

/// Waits until the tape's data block has loaded (border 4, within 6 s) and checks the LCD.
fn tape_loaded(m: &mut Machine) {
    use fmzx_pack::testprogs::{TAPE_DONE_BORDER, TAPE_WAIT_FRAMES};
    let waited = wait_for_border(m, TAPE_DONE_BORDER, 6000);
    println!("tape: border {TAPE_DONE_BORDER} after {waited} more ms (the code waits {TAPE_WAIT_FRAMES} frames)");
    m.run_ms(300);
    assert_golden(m.lcd_rgb565(), &golden("tape_loaded.png"), &build_dir().join("tape_loaded.actual.png"));
}

#[test]
fn tape_multiload_in_firmware() {
    let mut m = start(&firmware().with_pack(pack()), BootOpt { rgb565: true, divider: 0 }, game_index("tape"));
    assert_eq!(m.stats().view, zx_sys::view::GAME as u32);
    assert_eq!(m.stats().game, game_index("tape"));
    // the ROM takes about 2.3 s to run the autostart, then the code waits 2 s before it loads
    m.run_ms(3000);
    assert_eq!(lcd_border(&m), Some(border_pixel(1)), "border 1 three seconds after the start");
    tape_loaded(&mut m);
    assert_eq!(m.resets(), 0);
}

#[test]
fn tape_reloads_after_reset() {
    let mut m = start(&firmware().with_pack(pack()), BootOpt { rgb565: true, divider: 0 }, game_index("tape"));
    tape_loaded(&mut m);
    // HOME -> "Reset game" (the item below Resume)
    press_to(&mut m, HOME, zx_sys::view::MENU as i32);
    m.run_ms(100);
    m.press(OCT_UP, 30);
    m.run_ms(50);
    press_to(&mut m, PLAY, zx_sys::view::GAME as i32);
    m.run_ms(3000);
    assert_eq!(lcd_border(&m), Some(border_pixel(1)), "border 1 again three seconds after the reset");
    tape_loaded(&mut m);
    assert_eq!(m.resets(), 0);
}
