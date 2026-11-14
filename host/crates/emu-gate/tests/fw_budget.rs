// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware's budgets: RAMTEXT, image and pack in the app slot, POOL below the stack, the
//! CPU per Spectrum frame and the audio interrupt. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! CPU per frame is `game_frame` plus every `lcd_compose_band` call before the next frame, at
//! the LCD divider that sends the most bands (1), in RGB444, over 10 s of play after the games
//! are loaded and past their menus (`budgets::SETTLE_MS`), in both Sound choices: Beeper (the
//! default; the sound decoder still runs every frame for its look-ahead) and FM: sound. Besides
//! the pack's games, `noise.sna` (testprogs, in a pack of its own) gives the decoder its busiest
//! sound: 250 speaker edges a frame at pseudo-random spacings. `Machine::cost_of` counts at the
//! caller's interrupt depth, so the audio ISR (`fm1_alnk0_irq`, measured on its own) and the
//! key-scan timer interrupt are not in it.

use emu_gate::budgets::*;
use emu_gate::fw::{build_dir, BootOpt, Firmware, APP_SLOT};
use fmzx_pack::testprogs::noise_sna;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn firmware() -> Firmware {
    Firmware::load(&build_dir())
}

fn pack() -> Vec<u8> {
    fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack")
}

/// The manifest's games: id and demo.
fn games() -> Vec<(String, Vec<DemoStep>)> {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read games.toml");
    let m = fmzx_pack::manifest::parse_manifest(&text).expect("games.toml");
    m.game
        .iter()
        .map(|g| {
            (
                g.id.clone(),
                g.demo.as_deref().map_or(Vec::new(), |d| parse_demo(d).unwrap_or_else(|e| panic!("{}: {e}", g.id))),
            )
        })
        .collect()
}

/// A pack of one game, `noise.sna`, written to the build directory (never committed).
fn noise_pack() -> Vec<u8> {
    let dir = build_dir().join("noise-pack");
    std::fs::create_dir_all(&dir).expect("create the noise pack's folder");
    std::fs::write(dir.join("noise.sna"), noise_sna()).expect("write noise.sna");
    std::fs::write(dir.join("games.toml"), "[[game]]\nid = \"noise\"\nname = \"Speaker noise\"\nfile = \"noise.sna\"\n")
        .expect("write the noise manifest");
    fmzx_pack::pack::build_pack(&dir.join("games.toml")).expect("build the noise pack")
}

#[test]
fn view_game_value() {
    assert_eq!(zx_sys::view::PICKER as u32, VIEW_PICKER);
    assert_eq!(zx_sys::view::GAME as u32, VIEW_GAME);
    assert_eq!(zx_sys::view::MENU as u32, VIEW_MENU);
    assert_eq!(u32::from(zx_sys::SND_MODE_BEEPER), SOUND_BEEPER);
    assert_eq!(u32::from(zx_sys::SND_MODE_FM_A), SOUND_FM);
}

#[test]
fn ram_text_fits() {
    let n = ram_text_size(&firmware());
    println!("RAMTEXT {n} of {RAMTEXT_MAX} bytes");
    assert!(n <= RAMTEXT_MAX, "RAMTEXT is {n} bytes, over {RAMTEXT_MAX}");
}

#[test]
fn image_and_pack_fit_the_slot() {
    let (fw, pack) = (firmware(), pack());
    let n = image_and_pack_size(&fw, pack.len());
    println!("image {} + pack {} = {n} of {APP_SLOT:#x} ({}) bytes", fw.image.len(), pack.len(), APP_SLOT);
    assert!(n <= APP_SLOT, "image and pack are {n:#x} bytes, over {APP_SLOT:#x}");
    // fits the way the emulated flash lays it out, too
    let _ = fw.with_pack(&pack);
}

#[test]
fn pool_fits_below_the_stack() {
    let fw = firmware();
    let (used, free) = pool_use(&fw);
    println!("POOL {used} bytes, {free} below the stack guard {STACK_GUARD:#x}");
    assert!(free >= 0, "POOL runs {} bytes into the user stack", -free);
    // what must live in POOL: the ROM copy (16 KiB), the 128 KiB game buffer, the machine
    let game_buf = 128 * 1024;
    assert!(used >= 16 * 1024 + game_buf, "POOL is only {used} bytes: the ROM copy and the game buffer are missing");
}

fn mode_name(sound: u32) -> &'static str {
    if sound == SOUND_BEEPER {
        "Beeper"
    } else {
        "FM: sound"
    }
}

#[test]
fn cpu_per_frame_and_audio_isr() {
    let (fw, pack) = (firmware(), pack());
    let (fw_noise, fw) = (fw.with_pack(&noise_pack()), fw.with_pack(&pack));
    let mut games: Vec<(String, Vec<DemoStep>, &Firmware, u32)> =
        games().into_iter().enumerate().map(|(i, (id, demo))| (id, demo, &fw, i as u32)).collect();
    games.push(("noise".to_string(), Vec::new(), &fw_noise, 0));
    let runs_of = [SOUND_BEEPER, SOUND_FM];
    let opt = BootOpt { rgb565: false, divider: 1 };
    let runs: Vec<(u32, Measure)> = std::thread::scope(|s| {
        let hs: Vec<_> = runs_of
            .iter()
            .flat_map(|&sound| games.iter().map(move |g| (sound, g)))
            .map(|(sound, (_, demo, fw, index))| {
                s.spawn(move || (sound, measure_in(fw, opt, *index, demo, SETTLE_MS, 10_000, sound)))
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("game run")).collect()
    });
    let mut isr_max = 0;
    let mut bad = Vec::new();
    let mut worst = [(0u64, String::new()), (0, String::new())];
    let mut snd_worst = [(0u64, String::new()), (0, String::new())];
    for ((id, ..), (sound, r)) in games.iter().cycle().zip(&runs) {
        let mode = mode_name(*sound);
        let (max, avg) = (*r.cpu.iter().max().unwrap_or(&0), r.cpu.iter().sum::<u64>() / r.cpu.len().max(1) as u64);
        let (snd_max, snd_avg) = (*r.snd.iter().max().unwrap_or(&0), r.snd.iter().sum::<u64>() / r.snd.len().max(1) as u64);
        let isr = *r.isr.iter().max().unwrap_or(&0);
        isr_max = isr_max.max(isr);
        println!(
            "{id} ({mode}): {} frames, CPU per frame max {max} avg {avg}, of it the mixer max {snd_max} avg {snd_avg}, ISR max {isr} ({} calls)",
            r.frames,
            r.isr.len()
        );
        let k = usize::from(*sound != SOUND_BEEPER);
        if max > worst[k].0 {
            worst[k] = (max, id.clone());
        }
        if snd_max > snd_worst[k].0 {
            snd_worst[k] = (snd_max, id.clone());
        }
        assert_eq!(r.resets, 0, "{id} ({mode}): the firmware reset");
        assert_eq!(r.sound, *sound, "{id} ({mode}): the Sound choice");
        assert!(r.frames.abs_diff(500) <= 2, "{id} ({mode}): {} frames in 10 s", r.frames);
        assert!(
            r.cpu.len() + 2 >= r.frames as usize,
            "{id} ({mode}): game_frame seen {} times for {} frames",
            r.cpu.len(),
            r.frames
        );
        assert!(r.snd.len() + 2 >= r.frames as usize, "{id} ({mode}): the mixer ran {} times for {} frames", r.snd.len(), r.frames);
        // (border.sna, tape.tap, tune.sna and noise.sna are static once the border has been drawn / the image
        // has loaded: they compose no bands)
        assert!(matches!(id.as_str(), "border" | "tape" | "tune" | "noise") || r.lcd.bands > 0, "{id} ({mode}): no LCD bands composed");
        assert!(!r.isr.is_empty(), "{id} ({mode}): the audio ISR never ran");
        if max > CPU_PER_FRAME {
            bad.push(format!("{id} ({mode}): {max} > {CPU_PER_FRAME}"));
        }
    }
    for (k, sound) in runs_of.iter().enumerate() {
        println!(
            "{}: worst frame {} ({}), the mixer at most {} ({})",
            mode_name(*sound),
            worst[k].0,
            worst[k].1,
            snd_worst[k].0,
            snd_worst[k].1
        );
    }
    println!("audio ISR at most {isr_max}");
    assert!(isr_max <= ISR_PER_BLOCK, "audio ISR {isr_max} instructions, over {ISR_PER_BLOCK}");
    assert!(bad.is_empty(), "CPU per frame over budget: {bad:?}");
}
