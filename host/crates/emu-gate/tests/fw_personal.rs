// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The personal pack in the firmware: `tools/make-personal-pack` on a temporary folder of our
//! own generated test programs (the turbo tape, `tape.tap`, `tune.sna`; never anyone's games)
//! into a temporary folder, then its release package booted in fm1-emu as fm1-emu's web page
//! boots one. The picker lists the three games, each starts, and the turbo game (its loaded
//! snapshot) shows its border at once. The pack learned the tune's music: in FM: engines the
//! device plays the host mixer's output with that learned entry, sample for sample, and it is FM,
//! not the delayed beeper.
//!
//! Needs the device builds (`build/fm1` and `build/fm1-preview`, from `tools/make-packages`)
//! and the SDK files cached in `build/ac79/` (the script runs offline). From `host/`,
//! `FMZX_FM1_BUILD=$PWD/../build/fm1` as for the other firmware tests.

use emu_gate::budgets::{choose_sound, SOUND_FM_B};
use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use fmzx_pack::inputs::Pins;
use fmzx_pack::package::{verify, PRODUCT};
use fmzx_pack::testprogs::{tape_tap, tune_sna, turbo_tzx, TAPE_DONE_BORDER};
use std::path::{Path, PathBuf};
use std::process::Command;

const PLAY: u8 = 12;
const SELECT: u8 = 0;
const BOOT_MS: u64 = 1500;
const MASTER_ADC: u8 = 4;
/// The audio out's rate: 750,000 / 17 frames a second.
const RATE: (u32, u32) = (750_000, 17);
const OPT: BootOpt = BootOpt { rgb565: true, divider: 0 };

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// A fresh temporary folder, removed when dropped.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fmzx-fw-personal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// LCD row `row` of the host's composition of `m`, as RGB565 pixels.
fn host_row(m: &zx_sys::Machine, row: usize, crop_x: usize) -> Vec<u16> {
    let bytes = zx_sys::PixFormat::Rgb565.pack(&m.compose_row(row, crop_x));
    bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect()
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

/// The LCD's border rows (the top and bottom 8), all one colour, which is returned.
fn lcd_border(m: &Machine) -> Option<u16> {
    let lcd = m.lcd_rgb565();
    let c = lcd[0];
    (lcd[..8 * 240].iter().chain(&lcd[232 * 240..]).all(|&p| p == c)).then_some(c)
}

/// The picker the host composes from `pack`.
fn host_picker(pack: &[u8]) -> Vec<u16> {
    let count = zx_sys::pack_open(pack).expect("pack opens on the host");
    let mut ui = zx_sys::Ui::new();
    let mut menu = zx_sys::Menu::new(count);
    menu.set_divider(u32::from(OPT.divider));
    ui.draw(&menu, Some(pack));
    (0..240)
        .flat_map(|r| {
            let bytes = zx_sys::PixFormat::Rgb565.pack(&ui.compose_row(r));
            bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()
        })
        .collect()
}

/// Boots the package, selects game `index` and presses PLAY.
fn start(fw: &Firmware, pkg: &[u8], index: u32) -> Machine {
    let mut m = Machine::boot_package(fw, pkg, Some(OPT));
    m.run_ms(BOOT_MS);
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
fn fw_personal() {
    let games = Tmp::new("games");
    std::fs::write(games.0.join("turbo.tzx"), turbo_tzx()).unwrap();
    std::fs::write(games.0.join("tape.tap"), tape_tap()).unwrap();
    std::fs::write(games.0.join("tune.sna"), tune_sna()).unwrap();
    let dir = Tmp::new("out");
    let run = Command::new(repo().join("tools/make-personal-pack"))
        .arg(&games.0)
        .arg("-o")
        .arg(&dir.0)
        .arg("--offline")
        .output()
        .expect("run tools/make-personal-pack");
    let said = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    println!("{said}");
    assert!(run.status.success(), "tools/make-personal-pack failed:\n{said}");
    assert!(said.contains("used ") && said.contains(" KB free"), "the table:\n{said}");

    let pkg = std::fs::read(dir.0.join("fmzx.fwsc")).expect("DIR/fmzx.fwsc");
    let pack = std::fs::read(dir.0.join("games.bin")).expect("DIR/games.bin");
    for name in ["fmzx.fwsc", "fmzx-preview.fwsc"] {
        let r = verify(&std::fs::read(dir.0.join(name)).unwrap(), &Pins::builtin(), Some(PRODUCT));
        assert!(r.problems.is_empty(), "{name}: {:?}", r.problems);
        assert_eq!(r.pack.map(|p| p.1), Some(3), "{name}: three games");
    }
    let names: Vec<String> = (0..3).map(|i| zx_sys::pack_game_at(&pack, i).unwrap().name()).collect();
    assert_eq!(names, ["tape", "tune", "turbo"]);

    // the picker lists the three games
    let fw = Firmware::load(&build_dir());
    let mut m = Machine::boot_package(&fw, &pkg, Some(OPT));
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0);
    assert_eq!(m.stats().view, zx_sys::view::PICKER as u32);
    assert!(m.lcd_rgb565() == &host_picker(&pack)[..], "the LCD is not the host's picker of the three games");

    // each game starts (not the load-failed message)
    for i in 0..3 {
        let mut m = start(&fw, &pkg, i);
        m.run_ms(500);
        let s = m.stats();
        assert_eq!((s.view, s.game), (zx_sys::view::GAME as u32, i), "{}: {s:?}", names[i as usize]);
        assert!(s.frames > 0);
    }

    // the turbo game is its loaded snapshot: border 4 within 2 s of PLAY
    let mut m = start(&fw, &pkg, 2);
    let want = border_pixel(TAPE_DONE_BORDER);
    let waited = (0..2000).find(|_| {
        let done = lcd_border(&m) == Some(want);
        if !done {
            m.run_ms(1);
        }
        done
    });
    let waited = waited.unwrap_or_else(|| panic!("no border {TAPE_DONE_BORDER} within 2 s (LCD {:04X?}, {:?})", lcd_border(&m), m.stats()));
    println!("turbo: border {TAPE_DONE_BORDER} {waited} ms after PLAY");
    assert_eq!(m.resets(), 0);

    fm_engines_plays_learned_tune(&fw, &pkg, &pack);
}

/// Index of `needle` in `hay`.
fn find(hay: &[i16], needle: &[i16]) -> Option<usize> {
    (0..=hay.len().checked_sub(needle.len())?).find(|&p| hay[p..p + needle.len()] == *needle)
}

/// The tune (game 1) with its learned entry: started with MASTER at full, FM: engines chosen
/// from the HOME menu, 3 s recorded. The device plays the host mixer's output in FM: engines
/// with the pack's learned list for the tune, sample for sample; the mixer plays FM, and the
/// audio is not the delayed beeper.
fn fm_engines_plays_learned_tune(fw: &Firmware, pkg: &[u8], pack: &[u8]) {
    use zx_sys::{SndMixer, SND_DELAY_FRAMES, SND_MODE_BEEPER, SND_MODE_FM_B};
    const TUNE: u32 = 1;
    const FRAMES: usize = 150; // 3 s
    let list = fmzx_pack::pack::sound_list(pack, TUNE).expect("the tune's learned list").to_vec();
    assert_eq!(list[0], 1, "one learned entry");

    let mut m = Machine::boot_package(fw, pkg, Some(OPT));
    m.set_adc(MASTER_ADC, 1023);
    m.run_ms(BOOT_MS);
    assert_eq!(m.resets(), 0);
    m.turn(SELECT, TUNE as i32);
    m.run_ms(200);
    m.press(PLAY, 30);
    m.run_ms(20);
    assert_eq!((m.stats().view, m.stats().game), (zx_sys::view::GAME as u32, TUNE));
    let fm_from = choose_sound(&mut m, SOUND_FM_B) as usize;
    m.run_ms(3300);
    let dev = m.audio_left();
    let s = m.stats();
    assert_eq!((s.underruns, s.overflows, m.resets()), (0, 0, 0), "{s:?}");

    // the host: the same frames through the beeper and the mixer with the learned list, FM:
    // engines from the same frame
    let g = zx_sys::pack_game_at(pack, TUNE).expect("the tune's entry");
    let sna = g.extract(zx_sys::PACK_MAX_RAW as usize).expect("extracts");
    let mut zx = zx_sys::Machine::new();
    zx.load_sna(&sna).expect("loads");
    let mut beeper = zx_sys::Beeper::new(RATE.0, RATE.1);
    let mut mix = SndMixer::new(RATE.0, RATE.1);
    mix.set_learned(&list).expect("the mixer takes the learned list");
    let (mut beeped, mut mixed, mut starts, mut fm_on) = (vec![], vec![], vec![0usize], 0);
    for i in 0..fm_from + FRAMES + 10 {
        zx.run_frame();
        let level = zx.edges_start_level();
        beeper.set_level(level);
        let b = beeper.render(zx.edges());
        let mode = if i >= fm_from { SND_MODE_FM_B } else { SND_MODE_BEEPER };
        let out = mix.frame(mode, Some(&zx), zx.edges(), level, &b);
        assert_eq!(out.len(), b.len(), "the mixer gives a frame's samples");
        mixed.extend(out);
        fm_on += usize::from(i >= fm_from + 10 && i < fm_from + FRAMES && mix.fm_on());
        beeped.extend(b);
        starts.push(mixed.len());
    }

    // sample for sample from the switch on, for 3 s
    let probe = starts[fm_from + 8];
    let p = find(&dev, &mixed[probe..probe + 2000]).expect("the host's FM: engines mix is not in the device audio");
    let (from, to) = (starts[fm_from], starts[fm_from + FRAMES]);
    assert!(dev.len() >= p + to - probe, "only {} samples recorded", dev.len());
    assert!(dev[p + from - probe..p + to - probe] == mixed[from..to], "the device's FM: engines audio differs from the host's mix");

    // FM: the mixer plays B's FM, and the frames are not the beeper delayed
    let scored = FRAMES - 10;
    println!("tune in FM: engines: FM on in {fm_on} of {scored} frames");
    assert!(fm_on * 10 >= scored * 9, "FM on in only {fm_on} of {scored} frames");
    let d = SND_DELAY_FRAMES as usize;
    let delayed = |t: usize| &beeped[starts[t - d]..starts[t - d + 1]];
    let differ = (fm_from + 10..fm_from + FRAMES).filter(|&t| dev[p + starts[t] - probe..p + starts[t + 1] - probe] != *delayed(t)).count();
    assert!(differ * 10 >= scored * 9, "only {differ} of {scored} frames differ from the delayed beeper");
}
