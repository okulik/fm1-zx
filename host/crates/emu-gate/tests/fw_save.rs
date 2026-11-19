// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing the saved game to flash in fm1-emu: PLAY (pause) and HOME save the running game in
//! steps of the main loop; the save area holds a valid header and the machine's RAM afterwards.
//! Needs the device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml` (0 scroll, a snapshot game).

use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zx_sys::{
    saving, view, SAVE_HEADER_SIZE, SAVE_LEN, SAVE_SECTORS, SAVE_SECTOR, SAVE_SECTOR_DATA, SAVE_Z80_LEN, Menu, Ui,
};

const HOME: u8 = 8;
const OCT_UP: u8 = 1;
const PLAY: u8 = 12;
const SCROLL: u32 = 0;
const OPT: BootOpt = BootOpt { rgb565: true, divider: 4 };
const AREA: usize = 0xEA000;
const AREA_LEN: usize = SAVE_SECTORS * SAVE_SECTOR;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn firmware() -> &'static Firmware {
    static FW: OnceLock<Firmware> = OnceLock::new();
    FW.get_or_init(|| Firmware::load(&build_dir()).with_pack(pack()))
}

fn pack() -> &'static [u8] {
    static PACK: OnceLock<Vec<u8>> = OnceLock::new();
    PACK.get_or_init(|| fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack"))
}

/// Boots, starts the first game and runs it for 2 s.
fn running(m: &mut Machine) {
    m.run_ms(1500);
    m.press(PLAY, 30);
    m.run_ms(2000);
    assert_eq!(m.stats().view, view::GAME as u32);
}

fn playing() -> Machine {
    let mut m = Machine::boot(firmware(), OPT);
    running(&mut m);
    m
}

/// The 48 KB of the running game's RAM: `fmzx_machine.ram_words` follows the CPU core's state and
/// the ROM pointer (`rom_copy`), at an offset that differs from the host's.
fn device_ram(m: &Machine) -> Vec<u8> {
    let base = m.symbol("fmzx_machine");
    let rom = m.symbol("rom_copy");
    let at = (0..64).map(|i| base + 4 * i).find(|&a| m.peek32(a) == rom).expect("the ROM pointer in fmzx_machine") + 4;
    (0..SAVE_Z80_LEN as u32 - 30).step_by(4).flat_map(|i| m.peek32(at + i).to_le_bytes()).collect()
}

/// Save bytes `0..SAVE_LEN` out of the area.
fn save_bytes(flash: &[u8]) -> Vec<u8> {
    (0..SAVE_LEN as u32).map(|p| flash[AREA + zx_sys::save_area_offset(p) as usize]).collect()
}

fn header_ok(flash: &[u8]) -> Option<zx_sys::SaveInfo> {
    let h: [u8; SAVE_HEADER_SIZE] = flash[AREA..AREA + SAVE_HEADER_SIZE].try_into().unwrap();
    zx_sys::save_header_check(&h, Some(pack())).ok()
}

/// Runs (up to 3 s) until the save area holds a valid header.
fn until_saved(m: &mut Machine) {
    for _ in 0..3000 {
        if header_ok(&m.soc.flash.mem).is_some() {
            return;
        }
        m.run_ms(1);
    }
    panic!("no save within 3 s, {:?}", m.stats());
}

#[test]
fn pause_writes_a_valid_save() {
    let mut m = playing();
    let before = m.soc.flash.mem.clone();
    assert!(header_ok(&before).is_none());
    m.press(PLAY, 30);
    let ram = device_ram(&m);
    until_saved(&mut m);
    m.run_ms(100);
    assert_eq!(m.resets(), 0);
    let info = header_ok(&m.soc.flash.mem).unwrap();
    assert_eq!(info.game, SCROLL);
    assert_eq!(info.tape_pos, 0);
    let bytes = save_bytes(&m.soc.flash.mem);
    let z80 = &bytes[SAVE_HEADER_SIZE..];
    assert_eq!(zx_sys::pack_crc32(z80), info.z80_crc);
    let mut host = zx_sys::Machine::new();
    host.load_z80(z80).expect("the host loads the save");
    for (i, b) in ram.iter().enumerate() {
        assert_eq!(host.peek(0x4000 + i as u16), *b, "RAM byte {i}");
    }
    // nothing outside the area changed; the sector tails are erased
    let now = &m.soc.flash.mem;
    assert!((0..now.len()).filter(|i| !(AREA..AREA + AREA_LEN).contains(i)).all(|i| now[i] == before[i]));
    for s in 0..SAVE_SECTORS {
        let tail = AREA + s * SAVE_SECTOR + SAVE_SECTOR_DATA;
        assert!(now[tail..tail + SAVE_SECTOR - SAVE_SECTOR_DATA].iter().all(|&b| b == 0xFF), "sector {s}'s tail");
    }
    // the menu's banner is back to PAUSED, and the picker will mark the game
    assert_eq!(m.stats().view, view::PAUSED as u32);
}

#[test]
fn no_second_write_without_play() {
    let mut m = playing();
    m.press(PLAY, 30);
    until_saved(&mut m);
    m.run_ms(100);
    let ops = m.soc.flash.ops;
    assert!(ops > 0);
    m.press(HOME, 30);
    m.run_ms(300);
    assert_eq!(m.stats().view, view::MENU as u32);
    assert_eq!(m.soc.flash.ops, ops, "HOME after no frames ran");
    m.press(PLAY, 30); // Resume
    m.run_ms(1000);
    assert_eq!(m.stats().view, view::GAME as u32);
    m.press(HOME, 30);
    m.run_ms(500);
    assert!(m.soc.flash.ops > ops, "HOME after the game ran");
}

#[test]
fn buttons_during_write_wait() {
    let mut m = playing();
    m.watch("fmsave_step");
    m.hold(PLAY, true);
    assert!(m.run_until("fmsave_step", 100));
    m.hold(PLAY, false);
    let ram = device_ram(&m);
    let frames = m.stats().frames;
    // fm1-emu's flash is instant: the whole write takes less time than a button needs to register,
    // so the presses below land right after it; they must neither corrupt the save nor run frames
    m.press(PLAY, 10); // resume
    m.press(HOME, 10);
    m.press(OCT_UP, 10); // Reset game
    m.press(PLAY, 10);
    m.run_ms(1000);
    assert_eq!(m.resets(), 0);
    let info = header_ok(&m.soc.flash.mem).expect("a save");
    let bytes = save_bytes(&m.soc.flash.mem);
    assert_eq!(zx_sys::pack_crc32(&bytes[SAVE_HEADER_SIZE..]), info.z80_crc);
    assert_eq!(&bytes[SAVE_HEADER_SIZE + 30..], &ram[..], "the RAM at the first pause");
    // the reset restarted the game: its frame counter is below what the paused game had run
    assert!(m.stats().frames < frames, "frames {} (was {frames})", m.stats().frames);
}

#[test]
fn power_cut_mid_write_leaves_no_save() {
    let (flash, _) = saved_flash();
    let mut m = boot_with(pack(), &flash);
    assert!(header_ok(&m.soc.flash.mem).is_some(), "starts from a save");
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, PLAY, view::PAUSED); // Resume
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(200);
    m.watch("fmsave_step");
    m.hold(PLAY, true);
    let mut cut = None;
    for _ in 0..20_000 {
        let h = m.hits("fmsave_step");
        if (2..=13).contains(&h) {
            cut = Some(m.soc.flash.mem.clone());
            break;
        }
        if h > 13 {
            break;
        }
        m.run_us(20);
    }
    let cut = cut.expect("no point between the first erase and the header");
    assert!(header_ok(&cut).is_none());
    let mut m2 = Machine::boot(firmware(), OPT);
    m2.soc.flash.mem = cut;
    m2.run_ms(1500);
    assert_eq!(m2.stats().view, view::PICKER as u32);
    assert!(header_ok(&m2.soc.flash.mem).is_none());
}

/// The grid text of the paused banner when the save state is `state`, as the LCD would show rows 0-23.
fn banner_rows(state: i32) -> Vec<u16> {
    let mut ui = Ui::new();
    let mut mn = Menu::new(1);
    mn.set_view(view::GAME);
    mn.event(zx_sys::ev::PAUSE);
    mn.set_saving(state);
    ui.draw(&mn, None);
    (0..24)
        .flat_map(|r| {
            let b = zx_sys::ui_pack_row(zx_sys::PixFormat::Rgb565, &ui.compose_row(r));
            b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn failed_write_says_so() {
    let mut m = Machine::boot(firmware(), OPT);
    m.soc.flash.jedec = [0xEF, 0x40, 0x14];
    running(&mut m);
    let ops = m.soc.flash.ops;
    m.press(PLAY, 30);
    m.run_ms(300);
    assert_eq!(m.soc.flash.ops, ops, "no flash operation on an unknown part");
    assert_eq!(m.stats().view, view::PAUSED as u32);
    let lcd = &m.lcd_rgb565()[..24 * 240];
    assert!(lcd == &banner_rows(saving::FAILED)[..], "SAVE FAILED on the banner");
    assert!(lcd != &banner_rows(saving::IDLE)[..]);
    // PLAY resumes the game
    m.press(PLAY, 30);
    let f = m.stats().frames;
    m.run_ms(500);
    assert_eq!(m.stats().view, view::GAME as u32);
    assert!(m.stats().frames > f + 10);
}

/// The first erase waits until the audio DMA has played silence: at least 10 ms after PLAY went
/// down, and the game's audio is muted (no samples) from the press on.
#[test]
fn first_erase_waits_for_silence() {
    let mut m = playing();
    m.watch("fmsave_step");
    let (ops, t0) = (m.soc.flash.ops, m.ms());
    m.hold(PLAY, true);
    let mut at = None;
    for _ in 0..2000 {
        m.run_us(250);
        if m.soc.flash.ops > ops {
            at = Some(m.ms());
            break;
        }
    }
    m.hold(PLAY, false);
    let at = at.expect("no erase");
    assert!(at - t0 >= 10, "the first erase {} ms after the press", at - t0);
}

// ------------------------------------------------------------------ finding and resuming ---

const SELECT: u8 = 0;

fn view_is(m: &Machine, v: i32) -> bool {
    m.stats().view == v as u32
}

/// Presses `key` and waits (up to 300 ms) until the view is `v`.
fn press_to(m: &mut Machine, key: u8, v: i32) {
    m.press(key, 30);
    for _ in 0..300 {
        if view_is(m, v) {
            m.run_ms(60); // the key is released
            return;
        }
        m.run_ms(1);
    }
    panic!("view {} after pressing {key}, expected {v}", m.stats().view);
}

/// Boots `pack` with `flash` in place and runs to the picker.
fn boot_with(pack: &[u8], flash: &[u8]) -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack), OPT);
    m.soc.flash.mem = flash.to_vec();
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    assert_eq!(m.stats().view, view::PICKER as u32);
    m
}

/// The flash after playing game 0 for 2 s and pausing it, and the RAM it saved.
fn saved_flash() -> (Vec<u8>, Vec<u8>) {
    let mut m = playing();
    m.press(PLAY, 30);
    let ram = device_ram(&m);
    until_saved(&mut m);
    m.run_ms(100);
    (m.soc.flash.mem.clone(), ram)
}

fn empty_flash() -> Vec<u8> {
    Machine::boot(firmware(), OPT).soc.flash.mem.clone()
}

fn picker_lcd(m: &Machine) -> Vec<u16> {
    m.lcd_rgb565().to_vec()
}

#[test]
fn resume_after_power_cycle() {
    let (flash, ram) = saved_flash();
    let marked = boot_with(pack(), &flash);
    let plain = boot_with(pack(), &empty_flash());
    assert!(picker_lcd(&marked) != picker_lcd(&plain), "the picker marks the saved game");
    let mut m = marked;
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, PLAY, view::PAUSED); // Resume
    assert_eq!(device_ram(&m), ram, "the saved RAM");
    m.run_ms(500);
    assert_eq!(m.stats().frames, 0, "no frame ran");
    assert_eq!(m.stats().game, SCROLL);
    let ops = m.soc.flash.ops;
    press_to(&mut m, HOME, view::MENU);
    m.run_ms(300);
    assert_eq!(m.soc.flash.ops, ops, "HOME after resuming writes nothing");
    press_to(&mut m, PLAY, view::GAME); // Resume in the menu
    m.run_ms(500);
    assert!(m.stats().frames > 10);
    assert_eq!(m.resets(), 0);
}

#[test]
fn pausing_after_resume_and_play_saves() {
    let (flash, _) = saved_flash();
    let mut m = boot_with(pack(), &flash);
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, PLAY, view::PAUSED);
    let ops = m.soc.flash.ops;
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(200);
    press_to(&mut m, PLAY, view::PAUSED);
    m.run_ms(300);
    assert!(m.soc.flash.ops > ops, "pausing after frames ran saves");
}

#[test]
fn start_new_keeps_the_save() {
    let (flash, ram) = saved_flash();
    let mut m = boot_with(pack(), &flash);
    let marked = picker_lcd(&m);
    // BACK from the page: the picker, the mark still there
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, HOME, view::PICKER);
    m.run_ms(100);
    assert!(picker_lcd(&m) == marked);
    press_to(&mut m, PLAY, view::SAVED);
    m.press(OCT_UP, 30);
    m.run_ms(50);
    press_to(&mut m, PLAY, view::GAME); // Start new
    m.run_ms(200);
    assert!(m.stats().frames < 30, "a fresh start");
    assert!(device_ram(&m) != ram, "the RAM is the new game's, not the saved one");
    assert_eq!(&m.soc.flash.mem[AREA..AREA + AREA_LEN], &flash[AREA..AREA + AREA_LEN], "the area is untouched");
    assert!(header_ok(&m.soc.flash.mem).is_some());
}

#[test]
fn other_pack_shows_no_save() {
    let (flash, _) = saved_flash();
    let other = one_game_pack("Alpha", &fmzx_pack::testprogs::border_sna());
    let marked = boot_with(&other, &flash);
    let plain = boot_with(&other, &empty_flash());
    assert!(picker_lcd(&marked) == picker_lcd(&plain), "no mark for a pack whose entry differs");
    let mut m = marked;
    press_to(&mut m, PLAY, view::GAME);
}

#[test]
fn random_area_is_no_save() {
    let mut flash = empty_flash();
    let mut x = 0x1234_5678u32;
    for b in &mut flash[AREA..AREA + AREA_LEN] {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x as u8;
    }
    let m = boot_with(pack(), &flash);
    let plain = boot_with(pack(), &empty_flash());
    assert!(picker_lcd(&m) == picker_lcd(&plain));
}

#[test]
fn damaged_save_is_forgotten() {
    let (mut flash, _) = saved_flash();
    let at = AREA + zx_sys::save_area_offset(20_000) as usize;
    flash[at] ^= 0x01;
    let plain = picker_lcd(&boot_with(pack(), &empty_flash()));
    let mut m = boot_with(pack(), &flash);
    assert!(picker_lcd(&m) != plain, "the header is fine, the mark shows");
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, PLAY, view::MESSAGE); // Resume
    m.run_ms(100);
    let text = m.lcd_rgb565().to_vec();
    assert!(text != plain);
    assert_eq!(&m.soc.flash.mem[AREA..AREA + 8], &[0u8; 8], "the magic is zeros");
    press_to(&mut m, PLAY, view::PICKER);
    m.run_ms(100);
    assert!(picker_lcd(&m) == plain, "the mark is gone");
    let again = boot_with(pack(), &m.soc.flash.mem.clone());
    assert!(picker_lcd(&again) == plain, "still none after a power cycle");
}

/// Index of the game `id` in `games/games.toml`.
fn game_index(id: &str) -> u32 {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read the manifest");
    let m = fmzx_pack::manifest::parse_manifest(&text).expect("parse the manifest");
    m.game.iter().position(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}")) as u32
}

#[test]
fn tape_game_resumes_mid_tape() {
    let tape = game_index("tape");
    let mut m = Machine::boot(firmware(), OPT);
    m.run_ms(1500);
    m.turn(SELECT, tape as i32);
    m.run_ms(200);
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(3000); // the autostart is over, the code waits for its image block
    m.press(PLAY, 30);
    let ram = device_ram(&m);
    until_saved(&mut m);
    m.run_ms(100);
    let info = header_ok(&m.soc.flash.mem).unwrap();
    assert!(info.tape_pos > 0);
    let flash = m.soc.flash.mem.clone();

    let mut m = boot_with(pack(), &flash);
    m.turn(SELECT, tape as i32);
    m.run_ms(200);
    press_to(&mut m, PLAY, view::SAVED);
    press_to(&mut m, PLAY, view::PAUSED);
    assert_eq!(device_ram(&m), ram);
    press_to(&mut m, PLAY, view::GAME);
    m.run_ms(8000);
    let after = device_ram(&m);
    assert!(after[..6144] != ram[..6144], "the second block loaded after the resume");
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
