// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware's budgets: RAMTEXT, image and pack in the app slot, POOL below the stack, the
//! CPU per Spectrum frame and the audio interrupt. Needs the device build (`FMZX_FM1_BUILD`).
//!
//! CPU per frame is `game_frame` plus every `lcd_compose_band` call before the next frame, at
//! the LCD divider that sends the most bands (1), in RGB444, over 10 s of play after the games
//! are loaded and past their menus (`budgets::SETTLE_MS`). `Machine::cost_of` counts at the
//! caller's interrupt depth, so the audio ISR (`fm1_alnk0_irq`, measured on its own) and the
//! key-scan timer interrupt are not in it.

use emu_gate::budgets::*;
use emu_gate::fw::{build_dir, BootOpt, Firmware, APP_SLOT};
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

#[test]
fn view_game_value() {
    assert_eq!(zx_sys::view::GAME, 1);
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

#[test]
fn cpu_per_frame_and_audio_isr() {
    let (fw, pack) = (firmware(), pack());
    let fw = fw.with_pack(&pack);
    let games = games();
    let opt = BootOpt { rgb565: false, divider: 1 };
    let runs: Vec<Measure> = std::thread::scope(|s| {
        let hs: Vec<_> = games
            .iter()
            .enumerate()
            .map(|(i, (_, demo))| {
                let fw = &fw;
                s.spawn(move || measure(fw, opt, i as u32, demo, SETTLE_MS, 10_000))
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("game run")).collect()
    });
    let mut isr_max = 0;
    let mut bad = Vec::new();
    for ((id, _), r) in games.iter().zip(&runs) {
        let (max, avg) = (*r.cpu.iter().max().unwrap_or(&0), r.cpu.iter().sum::<u64>() / r.cpu.len().max(1) as u64);
        let isr = *r.isr.iter().max().unwrap_or(&0);
        isr_max = isr_max.max(isr);
        println!("{id}: {} frames, CPU per frame max {max} avg {avg}, ISR max {isr} ({} calls)", r.frames, r.isr.len());
        assert_eq!(r.resets, 0, "{id}: the firmware reset");
        assert!(r.frames.abs_diff(500) <= 2, "{id}: {} frames in 10 s", r.frames);
        assert!(
            r.cpu.len() + 2 >= r.frames as usize,
            "{id}: game_frame seen {} times for {} frames",
            r.cpu.len(),
            r.frames
        );
        // (border.sna is static once the border has been drawn: it composes no bands)
        assert!(id == "border" || r.lcd.bands > 0, "{id}: no LCD bands composed");
        assert!(!r.isr.is_empty(), "{id}: the audio ISR never ran");
        if max > CPU_PER_FRAME {
            bad.push(format!("{id}: {max} > {CPU_PER_FRAME}"));
        }
    }
    assert!(isr_max <= ISR_PER_BLOCK, "audio ISR {isr_max} instructions, over {ISR_PER_BLOCK}");
    assert!(bad.is_empty(), "CPU per frame over budget: {bad:?}");
}
