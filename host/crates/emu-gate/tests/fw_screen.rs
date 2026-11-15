// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Screen choices on the LCD in fm1-emu: each mode's picture equals the host's composition
//! (`compose_row_mode`, the same C) of the same machine state, border changes and mode switches
//! leave no stale rows, and the overlays (HOME menu, on-screen keyboard, PAUSED) look as they do
//! in Crop. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml` (1 border, 2 Castaway). The device and the host run the same frames
//! with no keys, so the host core, run for as many frames as the device has, is in the same
//! state. The LCD is compared where it has caught up: at the start of a frame (`zx_run_frame`)
//! with no LCD rows pending (`fmzx_stats.pending`, updated just before).

use emu_gate::budgets::{
    choose_screen, press_to, screen_mode, screen_name, SCREEN_BORDER, SCREEN_CROP, SCREEN_FIT, SCREEN_FIT_SMOOTH, VIEW_GAME,
    VIEW_MENU,
};
use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::view;

const PLAY: u8 = 12;
const HOME: u8 = 8;
const OCT_UP: u8 = 1;
const SELECT: u8 = 0;
const BORDER: u32 = 1;
const CASTAWAY: u32 = 2;
const MODES: [u32; 4] = [SCREEN_CROP, SCREEN_FIT, SCREEN_FIT_SMOOTH, SCREEN_BORDER];
/// RGB565 (what fm1-emu's LCD shows) at divider 0: a whole panel fits one LCD frame's budget.
const OPT: BootOpt = BootOpt { rgb565: true, divider: 0 };

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

/// Boots with `pack`, starts game `index` from the picker; returns once it runs.
fn start(pack: &[u8], index: u32) -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack), OPT);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    if index > 0 {
        m.turn(SELECT, index as i32);
        m.run_ms(200);
    }
    press_to(&mut m, PLAY, VIEW_GAME);
    assert_eq!(m.stats().game, index);
    m
}

/// The host side of a game: pack entry `index` loaded as the firmware loads it, run frame by
/// frame to match the device.
struct Host {
    zx: zx_sys::Machine,
    frames: u32,
    crop_x: usize,
}

impl Host {
    fn new(pack: &[u8], index: u32) -> Host {
        let g = zx_sys::pack_game_at(pack, index).expect("pack entry");
        let data = g.extract(zx_sys::PACK_MAX_RAW as usize).expect("extract");
        let mut zx = zx_sys::Machine::new();
        match g.kind() {
            zx_sys::PACK_SNA => zx.load_sna(&data).expect("host loads the snapshot"),
            zx_sys::PACK_Z80 => zx.load_z80(&data).expect("host loads the snapshot"),
            _ => {
                zx.tape_insert(&data).expect("host inserts the tape");
                zx.tape_autostart();
            }
        }
        Host { zx, frames: 0, crop_x: g.crop_x() as usize }
    }

    /// Runs the host to `frames` frames.
    fn run_to(&mut self, frames: u32) {
        assert!(self.frames <= frames, "the host is ahead: {} > {frames}", self.frames);
        while self.frames < frames {
            self.zx.run_frame();
            self.frames += 1;
        }
    }

    /// The whole panel in `mode`, RGB565 pixels row by row.
    fn lcd(&self, mode: u32) -> Vec<u16> {
        (0..240)
            .flat_map(|r| {
                let b = self.zx.compose_row_mode(r, mode, self.crop_x, zx_sys::PixFormat::Rgb565.raw());
                b.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect::<Vec<_>>()
            })
            .collect()
    }
}

/// The rows of `rows` where `got` and `want` differ.
fn diff_rows(got: &[u16], want: &[u16], rows: std::ops::Range<usize>) -> Vec<usize> {
    rows.filter(|&r| got[r * 240..r * 240 + 240] != want[r * 240..r * 240 + 240]).collect()
}

/// Asserts that LCD rows `rows` equal the host's composition in `mode`; writes both pictures to
/// the build directory when they do not.
fn assert_rows(m: &Machine, host: &Host, mode: u32, rows: std::ops::Range<usize>, what: &str) {
    let (got, want) = (m.lcd_rgb565(), host.lcd(mode));
    let bad = diff_rows(got, &want, rows);
    if !bad.is_empty() {
        let name = what.replace([' ', ',', ':'], "_");
        emu_gate::fw::write_png(&build_dir().join(format!("{name}.actual.png")), got);
        emu_gate::fw::write_png(&build_dir().join(format!("{name}.want.png")), &want);
    }
    assert!(bad.is_empty(), "{what}: after {} frames, LCD rows {bad:?} differ from the host's {}", host.frames, screen_name(mode));
}

/// Runs to the next frame start; returns `fmzx_stats` there (`frames` run, `pending` rows).
fn next_frame(m: &mut Machine) -> emu_gate::fw::FwStats {
    assert!(m.run_until("zx_run_frame", 100), "no Spectrum frame within 100 ms");
    m.stats()
}

/// At the next `times` frame starts where the LCD has caught up, the whole panel equals the
/// host's composition in `mode`.
fn assert_follows(m: &mut Machine, host: &mut Host, mode: u32, times: usize, what: &str) {
    let (mut checked, mut tries) = (0, 0);
    while checked < times {
        let s = next_frame(m);
        tries += 1;
        assert!(tries <= 10 * times, "{what}: the LCD seldom catches up: {s:?}");
        if s.pending != 0 {
            continue;
        }
        host.run_to(s.frames);
        assert_rows(m, host, mode, 0..240, what);
        checked += 1;
    }
}

/// Runs the game until `ms` after `t0`.
fn run_until_ms(m: &mut Machine, t0: u64, ms: u64) {
    if m.ms() < t0 + ms {
        m.run_ms(t0 + ms - m.ms());
    }
}

#[test]
fn screen_modes_match_host() {
    // Castaway's title at 6 s and border.sna's stripes at 1 s, in each mode
    let cases: Vec<(u32, u64, u32)> =
        [(CASTAWAY, 6000), (BORDER, 1000)].iter().flat_map(|&(g, ms)| MODES.iter().map(move |&mode| (g, ms, mode))).collect();
    std::thread::scope(|s| {
        for &(game, ms, mode) in &cases {
            s.spawn(move || {
                let mut m = start(pack(), game);
                let t0 = m.ms();
                if mode != SCREEN_CROP {
                    choose_screen(&mut m, mode);
                }
                assert_eq!(screen_mode(&m), mode);
                run_until_ms(&mut m, t0, ms);
                let mut host = Host::new(pack(), game);
                assert_follows(&mut m, &mut host, mode, 3, &format!("game {game} in {}", screen_name(mode)));
            });
        }
    });
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// A pack of one stored `.sna` game, `data`, with the default key map.
fn one_game_pack(name: &str, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; zx_sys::PACK_HEADER_SIZE + zx_sys::PACK_ENTRY_SIZE];
    p[..8].copy_from_slice(b"FMZXPACK");
    p[8..10].copy_from_slice(&2u16.to_le_bytes());
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

/// `border.sna` with its main loop replaced: each frame, right after the interrupt, the border
/// steps to the next colour (INC A; AND 7; OUT (FEh),A), so every border line changes every frame.
fn border_steps_sna() -> Vec<u8> {
    let mut sna = fmzx_pack::testprogs::border_sna();
    let code = [0xFB, 0x76, 0x3C, 0xE6, 0x07, 0xD3, 0xFE, 0x18, 0xF7]; // EI; HALT; ...; JR 8000h
    let at = 27 + 0x8000 - 0x4000;
    sna[at..at + code.len()].copy_from_slice(&code);
    sna
}

#[test]
fn screen_border_change_redraws() {
    // in Border, a border that changes every frame feeds every row (the side columns are
    // blended into the picture rows): after each of 10 consecutive frames, every row is the host's
    let pack = one_game_pack("Border steps", &border_steps_sna());
    let mut m = start(&pack, 0);
    choose_screen(&mut m, SCREEN_BORDER);
    m.run_ms(1000);
    let mut host = Host::new(&pack, 0);
    let mut s = next_frame(&mut m);
    // the first frame start with the LCD caught up, then the 10 frames after it
    for _ in 0..10 {
        if s.pending == 0 {
            break;
        }
        s = next_frame(&mut m);
    }
    for i in 0..=10 {
        assert_eq!(s.pending, 0, "frame {i} of 10: LCD rows still pending at the next frame ({s:?})");
        host.run_to(s.frames);
        assert_rows(&m, &host, SCREEN_BORDER, 0..240, &format!("border steps, frame {i}"));
        let f = s.frames;
        s = next_frame(&mut m);
        assert_eq!(s.frames, f + 1, "not consecutive frames");
    }
    // the border did change every frame: the picture differs from the frame before
    let before = host.lcd(SCREEN_BORDER);
    host.run_to(host.frames + 1);
    assert_ne!(before, host.lcd(SCREEN_BORDER), "the border did not change between frames");
}

/// After a Screen choice made in the menu, the first frame start where the LCD has caught up
/// comes within `max` frame starts of the resume; there the whole panel is in `mode`.
fn assert_switched(m: &mut Machine, host: &mut Host, mode: u32, max: usize) {
    for i in 0..max {
        let s = next_frame(m);
        if s.pending == 0 {
            host.run_to(s.frames);
            assert_rows(m, host, mode, 0..240, &format!("switch to {}", screen_name(mode)));
            println!("{}: the whole panel in {} frame starts after the resume", screen_name(mode), i + 1);
            return;
        }
    }
    panic!("{}: LCD rows still pending {max} frames after the resume", screen_name(mode));
}

#[test]
fn screen_switch_redraws_all() {
    // Castaway's title: Crop, then Border, then Fit smooth; each switch repaints every row in the
    // new mode at once (the view change back to the game marks the whole panel)
    let mut m = start(pack(), CASTAWAY);
    let t0 = m.ms();
    let mut host = Host::new(pack(), CASTAWAY);
    run_until_ms(&mut m, t0, 5000);
    assert_follows(&mut m, &mut host, SCREEN_CROP, 1, "Castaway in Crop");
    choose_screen(&mut m, SCREEN_BORDER);
    assert_switched(&mut m, &mut host, SCREEN_BORDER, 2);
    m.run_ms(1000);
    choose_screen(&mut m, SCREEN_FIT_SMOOTH);
    assert_switched(&mut m, &mut host, SCREEN_FIT_SMOOTH, 2);
    assert_follows(&mut m, &mut host, SCREEN_FIT_SMOOTH, 3, "Castaway in Fit smooth");
}

/// The HOME menu over the game, as the LCD shows it once settled.
fn menu_lcd(m: &mut Machine) -> Vec<u16> {
    press_to(m, HOME, VIEW_MENU);
    m.run_ms(300);
    assert_eq!(m.stats().pending, 0, "LCD rows still pending in the menu");
    let lcd = m.lcd_rgb565().to_vec();
    press_to(m, HOME, VIEW_GAME);
    m.run_ms(100);
    lcd
}

/// The on-screen keyboard over the game: its rows (200-239) once settled, and the frame state
/// where the rest was compared with the host in `mode`.
fn osk_rows(m: &mut Machine, host: &mut Host, mode: u32) -> Vec<u16> {
    press_to(m, HOME, VIEW_MENU);
    m.press(OCT_UP, 30);
    m.run_ms(50);
    m.press(OCT_UP, 30); // "Keyboard"
    m.run_ms(50);
    press_to(m, PLAY, view::OSK as u32);
    m.run_ms(300);
    let mut tries = 0;
    let s = loop {
        let s = next_frame(m);
        if s.pending == 0 {
            break s;
        }
        tries += 1;
        assert!(tries < 10, "the LCD does not catch up under the keyboard: {s:?}");
    };
    host.run_to(s.frames);
    assert_rows(m, host, mode, 0..200, &format!("game rows under the keyboard in {}", screen_name(mode)));
    let rows = m.lcd_rgb565()[200 * 240..].to_vec();
    press_to(m, HOME, VIEW_GAME);
    m.run_ms(100);
    rows
}

/// PAUSED over the game: its rows (0-23) once settled; the rest compared with the host in `mode`.
fn paused_rows(m: &mut Machine, host: &mut Host, mode: u32) -> Vec<u16> {
    press_to(m, PLAY, view::PAUSED as u32);
    m.run_ms(300);
    let s = m.stats();
    assert_eq!(s.pending, 0, "LCD rows still pending while paused");
    host.run_to(s.frames);
    assert_rows(m, host, mode, 24..240, &format!("game rows while paused in {}", screen_name(mode)));
    let rows = m.lcd_rgb565()[..24 * 240].to_vec();
    press_to(m, PLAY, VIEW_GAME);
    m.run_ms(100);
    rows
}

#[test]
fn screen_overlays_unchanged() {
    // border.sna (the stripes reach into every Border row): the overlays in Border are the
    // overlays in Crop, and the game rows around them are Border's
    let mut m = start(pack(), BORDER);
    let mut host = Host::new(pack(), BORDER);
    m.run_ms(500);
    let menu_crop = menu_lcd(&mut m);
    let osk_crop = osk_rows(&mut m, &mut host, SCREEN_CROP);
    let paused_crop = paused_rows(&mut m, &mut host, SCREEN_CROP);

    choose_screen(&mut m, SCREEN_BORDER);
    m.run_ms(300);
    let menu_border = menu_lcd(&mut m);
    // the menu differs only in its Screen line (grid row 16: LCD rows 128-135)
    let bad: Vec<usize> = diff_rows(&menu_border, &menu_crop, 0..240).into_iter().filter(|r| !(128..136).contains(r)).collect();
    assert!(bad.is_empty(), "the HOME menu in Border differs from Crop's in rows {bad:?}");
    assert!(!diff_rows(&menu_border, &menu_crop, 128..136).is_empty(), "the Screen line did not change");
    let osk_border = osk_rows(&mut m, &mut host, SCREEN_BORDER);
    assert!(osk_border == osk_crop, "the on-screen keyboard in Border differs from Crop's");
    let paused_border = paused_rows(&mut m, &mut host, SCREEN_BORDER);
    assert!(paused_border == paused_crop, "PAUSED in Border differs from Crop's");
    assert_eq!(m.stats().view, VIEW_GAME);
    assert_eq!(screen_mode(&m), SCREEN_BORDER);
}
