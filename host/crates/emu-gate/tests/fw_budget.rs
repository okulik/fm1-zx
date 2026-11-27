// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware's budgets: RAMTEXT, the flash driver in the RAM region, image and pack in the app
//! slot, POOL below the stack, the CPU per Spectrum frame and the audio interrupt (and no audio
//! underruns while the games run). Needs the device build (`FMZX_FM1_BUILD`).
//!
//! The CPU per frame is measured with the USB cable out and again with it in (a host enumerates
//! the firmware and polls it while the game runs).
//!
//! By default the CPU per frame is measured in the two heaviest Sound and Screen pairs only
//! (`QUICK_RUNS`: FM: sound in Crop, FM: engines in Border; the learned-entries test in Border),
//! about 8 minutes; `FMZX_BUDGET_FULL=1` measures every pair below (`RUNS`), about 25 minutes,
//! before a release or after a change to per-frame code that one mode alone runs.
//!
//! CPU per frame is `game_frame` plus every `lcd_compose_band` call before the next frame, at
//! the LCD divider that sends the most bands (1), in RGB444, over 10 s of play after the games
//! are loaded and past their menus (`budgets::SETTLE_MS`), in Crop with all three Sound choices:
//! Beeper (the default; the mixer still runs both methods every frame for their look-ahead), FM:
//! sound and FM: engines; and in the other Screen choices (Fit, Fit smooth, Border) with FM:
//! engines (screen work and sound work are independent and add up, so not every pair is run;
//! the worst unmeasured pair, estimated, is Border with Black Star under FM: sound, about 2.60M). Besides the pack's games, three test programs (testprogs, in a pack of their
//! own, never committed): `noise.sna` gives the decoder its busiest sound, 250 speaker edges a
//! frame at pseudo-random spacings; `sites.sna` and `sites-var.sna` drive the speaker from four
//! OUT addresses a frame that match no engine, method B's matching worst case: in `sites.sna`
//! the code around them stays the same (B remembers that they match nothing), in
//! `sites-var.sna` it changes every frame (B looks every one up, every frame).
//! `Machine::cost_of` counts at the caller's interrupt depth, so the audio ISR (`fm1_alnk0_irq`,
//! measured on its own) and the key-scan timer interrupt are not in it.
//!
//! Learned entries (`cpu_per_frame_with_learned_entries`), in FM: engines, Crop and Border: the
//! personal pack of `tune.sna` (built as `fmzx-pack personal` builds it, the tune's
//! learned entry in its sound section) while the tune plays, its learned entry claiming it; and
//! the decoder's worst legitimate list (8 entries matching `SND_LEARNED_WORK_MAX` site-signature
//! bytes, each a relocated word every 2 bytes, never matching) on `sites.sna` and
//! `sites-var.sna`, whose four OUT sites no entry knows: method B tries every entry's every site
//! at them, every frame for `sites-var.sna`.

use emu_gate::budgets::*;
use emu_gate::fw::{build_dir, BootOpt, Firmware, APP_SLOT};
use fmzx_pack::testprogs::{noise_sna, out_sites_sna, tune_sna};
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

/// The test programs' pack: `noise.sna`, `sites.sna` and `sites-var.sna`, written to the build
/// directory (never committed, never in `games/games.toml`). Ids in pack order.
fn fixture_pack() -> (Vec<u8>, [&'static str; 3]) {
    let dir = build_dir().join("fixture-pack");
    std::fs::create_dir_all(&dir).expect("create the fixture pack's folder");
    let progs = [
        ("noise", "Speaker noise", noise_sna()),
        ("sites", "Four OUT sites", out_sites_sna(false)),
        ("sites-var", "Four OUT sites, changing", out_sites_sna(true)),
    ];
    let mut toml = String::new();
    for (id, name, sna) in &progs {
        std::fs::write(dir.join(format!("{id}.sna")), sna).expect("write a test program");
        toml += &format!("[[game]]\nid = \"{id}\"\nname = \"{name}\"\nfile = \"{id}.sna\"\n");
    }
    std::fs::write(dir.join("games.toml"), toml).expect("write the fixture manifest");
    (fmzx_pack::pack::build_pack(&dir.join("games.toml")).expect("build the fixture pack"), progs.map(|p| p.0))
}

#[test]
fn view_game_value() {
    assert_eq!(zx_sys::view::PICKER as u32, VIEW_PICKER);
    assert_eq!(zx_sys::view::GAME as u32, VIEW_GAME);
    assert_eq!(zx_sys::view::MENU as u32, VIEW_MENU);
    assert_eq!(u32::from(zx_sys::SND_MODE_BEEPER), SOUND_BEEPER);
    assert_eq!(u32::from(zx_sys::SND_MODE_FM_A), SOUND_FM);
    assert_eq!(u32::from(zx_sys::SND_MODE_FM_B), SOUND_FM_B);
    assert_eq!(zx_sys::PIX_CROP, SCREEN_CROP);
    assert_eq!(zx_sys::PIX_FIT, SCREEN_FIT);
    assert_eq!(zx_sys::PIX_FIT_SMOOTH, SCREEN_FIT_SMOOTH);
    assert_eq!(zx_sys::PIX_BORDER, SCREEN_BORDER);
}

#[test]
fn ram_text_fits() {
    let n = ram_text_size(&firmware());
    println!("RAMTEXT {n} of {RAMTEXT_MAX} bytes");
    assert!(n <= RAMTEXT_MAX, "RAMTEXT is {n} bytes, over {RAMTEXT_MAX}");
}

/// felucca-bsp's flash driver (fm1_flash.h's RAM functions, usbota.c) runs from its own section in
/// the RAM region, beside `.data` and `.bss`, not from RAMTEXT.
#[test]
fn flash_driver_in_ram() {
    let fw = firmware();
    let (addr, size) = section(&fw, ".ram_flash").expect("no .ram_flash section: the flash driver is not linked");
    let used = ram_use(&fw);
    println!("flash driver {size} bytes at {addr:#x}; RAM {used} of {RAM_MAX} bytes");
    assert!(size > 0, "the .ram_flash section is empty");
    assert!(
        addr >= RAM_START && addr + size <= RAM_START + RAM_MAX,
        "the flash driver ({addr:#x}, {size} bytes) is outside the RAM region"
    );
    assert!(used <= RAM_MAX, "the RAM region holds {used} bytes, over {RAM_MAX}");
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

/// The (Sound, Screen) choices measured: Crop in every Sound choice, then the other Screen
/// choices in FM: engines.
const RUNS: [(u32, u32); 6] = [
    (SOUND_BEEPER, SCREEN_CROP),
    (SOUND_FM, SCREEN_CROP),
    (SOUND_FM_B, SCREEN_CROP),
    (SOUND_FM_B, SCREEN_FIT),
    (SOUND_FM_B, SCREEN_FIT_SMOOTH),
    (SOUND_FM_B, SCREEN_BORDER),
];
/// The quick default: the two heaviest pairs, FM: sound in Crop (method A's decoder, the most
/// sound work) and FM: engines in Border (the most screen work, with the default sound).
const QUICK_RUNS: [(u32, u32); 2] = [(SOUND_FM, SCREEN_CROP), (SOUND_FM_B, SCREEN_BORDER)];

/// Every pair (`FMZX_BUDGET_FULL=1`), or the quick default.
fn full() -> bool {
    std::env::var_os("FMZX_BUDGET_FULL").is_some_and(|v| v != "0")
}

/// The Sound and Screen pairs measured: RUNS in full, QUICK_RUNS by default.
fn pairs() -> &'static [(u32, u32)] {
    if full() {
        &RUNS
    } else {
        &QUICK_RUNS
    }
}

fn mode_name(sound: u32) -> &'static str {
    match sound {
        SOUND_BEEPER => "Beeper",
        SOUND_FM => "FM: sound",
        _ => "FM: engines",
    }
}

fn run_name((sound, screen): (u32, u32)) -> String {
    format!("{}, {}", mode_name(sound), screen_name(screen))
}

/// A CPU-per-frame case: the (Sound, Screen) pair and whether the USB cable is in.
type Case = ((u32, u32), bool);

/// The CPU-per-frame cases: each pair with the USB cable out, then in (a host enumerates the firmware
/// and polls it while the game runs).
fn cases() -> Vec<Case> {
    [false, true].iter().flat_map(|&cable| pairs().iter().map(move |&p| (p, cable))).collect()
}

fn case_name((run, cable): Case) -> String {
    format!("{}, cable {}", run_name(run), if cable { "in" } else { "out" })
}

#[test]
fn cpu_per_frame_and_audio_isr() {
    let (fw, pack, (fixtures, fixture_ids)) = (firmware(), pack(), fixture_pack());
    let (fw_fix, fw) = (fw.with_pack(&fixtures), fw.with_pack(&pack));
    let mut games: Vec<(String, Vec<DemoStep>, &Firmware, u32)> =
        games().into_iter().enumerate().map(|(i, (id, demo))| (id, demo, &fw, i as u32)).collect();
    games.extend(fixture_ids.iter().enumerate().map(|(i, id)| (id.to_string(), Vec::new(), &fw_fix, i as u32)));
    let opt = BootOpt { rgb565: false, divider: 1 };
    let cases = cases();
    let runs: Vec<(Case, Measure)> = std::thread::scope(|s| {
        let hs: Vec<_> = cases
            .iter()
            .flat_map(|&case| games.iter().map(move |g| (case, g)))
            .map(|(case, (_, demo, fw, index))| {
                let ((sound, screen), cable) = case;
                s.spawn(move || (case, measure_with(fw, opt, *index, demo, SETTLE_MS, 10_000, sound, screen, cable)))
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("game run")).collect()
    });
    let mut isr_max = 0;
    let mut bad = Vec::new();
    let mut worst = vec![(0u64, String::new()); cases.len()];
    let mut snd_worst = vec![(0u64, String::new()); cases.len()];
    // per case: the mixer's most in sites.sna and sites-var.sna
    let mut sites = vec![(0u64, 0u64); cases.len()];
    for ((id, demo, ..), (case, r)) in games.iter().cycle().zip(&runs) {
        let (run, cable) = case;
        let mode = case_name(*case);
        let (max, avg) = (*r.cpu.iter().max().unwrap_or(&0), r.cpu.iter().sum::<u64>() / r.cpu.len().max(1) as u64);
        let (snd_max, snd_avg) = (*r.snd.iter().max().unwrap_or(&0), r.snd.iter().sum::<u64>() / r.snd.len().max(1) as u64);
        let isr = *r.isr.iter().max().unwrap_or(&0);
        isr_max = isr_max.max(isr);
        println!(
            "{id} ({mode}): {} frames, CPU per frame max {max} avg {avg}, of it the mixer max {snd_max} avg {snd_avg}, ISR max {isr} ({} calls)",
            r.frames,
            r.isr.len()
        );
        let k = cases.iter().position(|c| c == case).expect("a measured case");
        if max > worst[k].0 {
            worst[k] = (max, id.clone());
        }
        if snd_max > snd_worst[k].0 {
            snd_worst[k] = (snd_max, id.clone());
        }
        match id.as_str() {
            "sites" => sites[k].0 = snd_max,
            "sites-var" => sites[k].1 = snd_max,
            _ => {}
        }
        assert_eq!(r.resets, 0, "{id} ({mode}): the firmware reset");
        assert_eq!(r.usb_configured, *cable, "{id} ({mode}): configured by the USB host");
        if r.underruns != 0 {
            bad.push(format!("{id} ({mode}): {} audio underruns", r.underruns));
        }
        assert_eq!((r.sound, r.screen), *run, "{id} ({mode}): the Sound and Screen choices");
        assert!(r.frames.abs_diff(500) <= 2, "{id} ({mode}): {} frames in 10 s", r.frames);
        assert!(
            r.cpu.len() + 2 >= r.frames as usize,
            "{id} ({mode}): game_frame seen {} times for {} frames",
            r.cpu.len(),
            r.frames
        );
        assert!(r.snd.len() + 2 >= r.frames as usize, "{id} ({mode}): the mixer ran {} times for {} frames", r.snd.len(), r.frames);
        // (a game with no demo stays where it starts, which may be static and compose no bands:
        // border.sna, tape.tap, tune.sna and the test programs once the border has been drawn /
        // the image has loaded, and the title screens Black Star and Micro INC play their tunes on)
        assert!(demo.is_empty() || r.lcd.bands > 0, "{id} ({mode}): no LCD bands composed");
        assert!(!r.isr.is_empty(), "{id} ({mode}): the audio ISR never ran");
        if max > CPU_PER_FRAME {
            bad.push(format!("{id} ({mode}): {max} > {CPU_PER_FRAME}"));
        }
    }
    for (k, case) in cases.iter().enumerate() {
        println!(
            "{}: worst frame {} ({}), the mixer at most {} ({}); with four unknown OUT sites the mixer at most {} (code unchanged), {} (changing)",
            case_name(*case),
            worst[k].0,
            worst[k].1,
            snd_worst[k].0,
            snd_worst[k].1,
            sites[k].0,
            sites[k].1
        );
    }
    println!("audio ISR at most {isr_max}");
    assert!(isr_max <= ISR_PER_BLOCK, "audio ISR {isr_max} instructions, over {ISR_PER_BLOCK}");
    assert!(bad.is_empty(), "CPU per frame over budget, or audio underruns: {bad:?}");
    // method B remembers addresses that match no engine: unchanged code around four unknown OUT
    // sites costs the mixer less than half what looking them up every frame does
    for (k, &(same, var)) in sites.iter().enumerate() {
        assert!(same * 2 < var, "{}: four unknown OUT sites cost the mixer {same} with the code unchanged, {var} changing", case_name(cases[k]));
    }
}

/// The worst frame the Border and scroll measurements gave before learned entries (Border with
/// scroll.sna): a learned entry playing must cost no more.
const BORDER_SCROLL_WORST: u64 = 2_500_000;
/// The audio out's rate: 750,000 / 17 frames a second.
const RATE: (u32, u32) = (750_000, 17);

/// The personal pack of `tune.sna` alone, as `fmzx-pack personal` builds it (its learned
/// list in the sound section), in the build directory.
fn tune_personal_pack() -> Vec<u8> {
    let dir = build_dir().join("learned-pack");
    let (games, out) = (dir.join("games"), dir.join("out"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&games).expect("create the learned pack's folder");
    std::fs::write(games.join("tune.sna"), tune_sna()).expect("write tune.sna");
    let r = fmzx_pack::personal::run(&games, &out).expect("the personal pack of tune.sna");
    assert_eq!(r.sound, ["tune: music learned (1 routine, heard, 1 channel; 1 shadow score)"], "{r:?}");
    std::fs::read(out.join("games.bin")).expect("games.bin")
}

/// The decoder's worst legitimate list for the matching: 8 entries of `SND_LEARNED_LEN_MAX`
/// bytes and one site each (`SND_LEARNED_WORK_MAX` in all), a relocated word every 2 bytes (two
/// reads a byte), half with their site first and half last (the window method B hashes is two
/// signatures long), and digests no code has.
fn stress_list() -> Vec<u8> {
    use soundlearn::{encode_list, LearnedEntry, Op};
    let len = zx_sys::SND_LEARNED_LEN_MAX;
    let entries: Vec<LearnedEntry> = (0..8u16)
        .map(|k| LearnedEntry {
            instrument: [0; 3],
            digest: 0x5EED_0000_0000_0000 | u64::from(k),
            len,
            wild: vec![],
            reloc: (0..255).map(|i| 2 * i).collect(),
            sites: vec![(if k % 2 == 0 { 0 } else { len - 1 }, zx_sys::SND_SITE_TONAL, 0)],
            pass: 0,
            pos: vec![0],
            flips: false,
            observed: false,
            drum: 0,
            stale: None,
            level: vec![200],
            progs: vec![vec![Op::K(300)]],
            timing: None,
        })
        .collect();
    assert_eq!(entries.iter().map(LearnedEntry::work).sum::<usize>(), zx_sys::SND_LEARNED_WORK_MAX);
    let list = encode_list(&entries);
    let table = zx_sys::SndEngines::with_learned(&list).expect("the decoder takes the stress list");
    assert_eq!(table.entries().len(), zx_sys::SndEngines::new().entries().len() + 8);
    list
}

/// `sites.sna` and `sites-var.sna` in a pack of their own, each with the stress list. Ids in
/// pack order.
fn stress_pack() -> (Vec<u8>, [&'static str; 2]) {
    let dir = build_dir().join("stress-pack");
    std::fs::create_dir_all(&dir).expect("create the stress pack's folder");
    let progs = [("sites", "Four OUT sites", out_sites_sna(false)), ("sites-var", "Four OUT sites, changing", out_sites_sna(true))];
    let mut toml = String::new();
    for (id, name, sna) in &progs {
        std::fs::write(dir.join(format!("{id}.sna")), sna).expect("write a test program");
        toml += &format!("[[game]]\nid = \"{id}\"\nname = \"{name}\"\nfile = \"{id}.sna\"\n");
    }
    std::fs::write(dir.join("games.toml"), toml).expect("write the stress manifest");
    let mut pack = fmzx_pack::pack::build_pack(&dir.join("games.toml")).expect("build the stress pack");
    let list = stress_list();
    fmzx_pack::pack::attach_sound(&mut pack, &[Some(list.clone()), Some(list)]).expect("attach the stress lists");
    (pack, progs.map(|p| p.0))
}

/// Frames of `frames` (from the tune's start) in which the host mixer, in FM: engines with the
/// tune's learned list, plays FM: the learned entry claims the tune then (no built-in matches it).
fn tune_fm_frames(list: &[u8], frames: std::ops::Range<usize>) -> usize {
    let mut zx = zx_sys::Machine::new();
    zx.load_sna(&tune_sna()).expect("loads");
    let mut beeper = zx_sys::Beeper::new(RATE.0, RATE.1);
    let mut mix = zx_sys::SndMixer::new(RATE.0, RATE.1);
    mix.set_learned(list).expect("the mixer takes the learned list");
    let mut on = 0;
    for i in 0..frames.end {
        zx.run_frame();
        let level = zx.edges_start_level();
        beeper.set_level(level);
        let b = beeper.render(zx.edges());
        mix.frame(zx_sys::SND_MODE_FM_B, Some(&zx), zx.edges(), level, &b);
        on += usize::from(frames.contains(&i) && mix.fm_on());
    }
    on
}

#[test]
fn cpu_per_frame_with_learned_entries() {
    let fw = firmware();
    let tune_pack = tune_personal_pack();
    let list = fmzx_pack::pack::sound_list(&tune_pack, 0).expect("the tune's learned list").to_vec();
    assert_eq!(list[0], 1, "one learned entry");
    // the measured window, in the tune's frames: its learned entry plays it throughout
    let window = (SETTLE_MS / 20) as usize..((SETTLE_MS + 10_000) / 20) as usize;
    let on = tune_fm_frames(&list, window.clone());
    println!("tune: the learned entry plays FM in {on} of the window's {} frames (host)", window.len());
    assert!(on * 10 >= window.len() * 9, "the learned entry plays the tune in only {on} of {} frames", window.len());

    let (stress, stress_ids) = stress_pack();
    let (fw_tune, fw_stress, fw_fix) = (fw.with_pack(&tune_pack), fw.with_pack(&stress), fw.with_pack(&fixture_pack().0));
    // (name, firmware, game index): sites-var without a learned list too, for the comparison
    let mut games: Vec<(String, &Firmware, u32)> = vec![("tune (learned)".into(), &fw_tune, 0)];
    games.extend(stress_ids.iter().enumerate().map(|(i, id)| (format!("{id} (stress list)"), &fw_stress, i as u32)));
    games.push(("sites-var (no list)".into(), &fw_fix, 2));
    let opt = BootOpt { rgb565: false, divider: 1 };
    // (Border only by default: the more screen work; Crop too in full)
    let screens: &[u32] = if full() { &[SCREEN_CROP, SCREEN_BORDER] } else { &[SCREEN_BORDER] };
    let runs: Vec<(String, u32, Measure)> = std::thread::scope(|s| {
        let hs: Vec<_> = screens
            .iter()
            .flat_map(|&screen| games.iter().map(move |g| (screen, g)))
            .map(|(screen, (name, fw, index))| {
                s.spawn(move || (name.clone(), screen, measure_in(fw, opt, *index, &[], SETTLE_MS, 10_000, SOUND_FM_B, screen)))
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("game run")).collect()
    });
    let (mut tune_worst, mut stress_worst) = (0u64, 0u64);
    for (name, screen, r) in &runs {
        let mode = run_name((SOUND_FM_B, *screen));
        let (max, avg) = (*r.cpu.iter().max().unwrap_or(&0), r.cpu.iter().sum::<u64>() / r.cpu.len().max(1) as u64);
        let snd_max = *r.snd.iter().max().unwrap_or(&0);
        println!("{name} ({mode}): {} frames, CPU per frame max {max} avg {avg}, of it the mixer max {snd_max}", r.frames);
        assert_eq!(r.resets, 0, "{name} ({mode}): the firmware reset");
        assert_eq!((r.sound, r.screen), (SOUND_FM_B, *screen), "{name} ({mode}): the Sound and Screen choices");
        assert!(r.frames.abs_diff(500) <= 2, "{name} ({mode}): {} frames in 10 s", r.frames);
        assert!(r.cpu.len() + 2 >= r.frames as usize && r.snd.len() + 2 >= r.frames as usize, "{name} ({mode}): frames missed");
        assert!(max <= CPU_PER_FRAME, "{name} ({mode}): {max} > {CPU_PER_FRAME}");
        if name.starts_with("tune") {
            tune_worst = tune_worst.max(max);
        } else if name.contains("stress") {
            stress_worst = stress_worst.max(max);
        }
    }
    println!("learned entry playing: worst frame {tune_worst}; the stress list: worst frame {stress_worst} (cap {CPU_PER_FRAME}, Border with scroll {BORDER_SCROLL_WORST})");
    assert!(tune_worst < BORDER_SCROLL_WORST, "the tune with its learned entry: worst frame {tune_worst}, over {BORDER_SCROLL_WORST}");
}
