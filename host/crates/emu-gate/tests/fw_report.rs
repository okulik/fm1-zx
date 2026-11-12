// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The display frame-rate report: for every game in `games/games.toml`, at LCD dividers 4, 2
//! and 1 (RGB444), 20 s of play after the game is loaded and past its menus (the manifest's
//! `demo` keys pressed throughout): display frames per second (`50 * row_sends /
//! pending_frames_sum`), LCD bytes per frame and the longest a row waited. Writes
//! `<build>/fmzx-report.json` and prints a table. Needs the device build (`FMZX_FM1_BUILD`).
//! Run: `cargo test --release -p emu-gate --test fw_report -- --nocapture`.

use emu_gate::budgets::*;
use emu_gate::fw::{build_dir, BootOpt, Firmware, APP_SLOT};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const DIVIDERS: [u8; 3] = [4, 2, 1];
const WINDOW_MS: u64 = 20_000;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

struct Cell {
    fps: Option<f64>,
    bytes_per_frame: u32,
    max_age: u32,
    m: Measure,
}

#[test]
fn display_fps() {
    let fw = Firmware::load(&build_dir());
    let pack = fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack");
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read games.toml");
    let manifest = fmzx_pack::manifest::parse_manifest(&text).expect("games.toml");
    let games: Vec<(String, Vec<DemoStep>)> = manifest
        .game
        .iter()
        .map(|g| {
            (
                g.id.clone(),
                g.demo.as_deref().map_or(Vec::new(), |d| parse_demo(d).unwrap_or_else(|e| panic!("{}: {e}", g.id))),
            )
        })
        .collect();
    let image_pack = image_and_pack_size(&fw, pack.len());
    let ram_text = ram_text_size(&fw);
    let (pool_used, pool_free) = pool_use(&fw);
    let fw = fw.with_pack(&pack);

    // every game at every divider, in parallel (each run is minutes of emulation)
    let cells: Vec<Vec<Cell>> = std::thread::scope(|s| {
        let hs: Vec<Vec<_>> = games
            .iter()
            .enumerate()
            .map(|(i, (_, demo))| {
                DIVIDERS
                    .iter()
                    .map(|&d| {
                        let fw = &fw;
                        s.spawn(move || {
                            let m = measure(
                                fw,
                                BootOpt { rgb565: false, divider: d },
                                i as u32,
                                demo,
                                SETTLE_MS,
                                WINDOW_MS,
                            );
                            Cell {
                                fps: fps(&m.lcd),
                                bytes_per_frame: m.lcd.bytes / m.lcd.frames.max(1),
                                max_age: m.lcd.max_age,
                                m,
                            }
                        })
                    })
                    .collect()
            })
            .collect();
        hs.into_iter().map(|row| row.into_iter().map(|h| h.join().expect("game run")).collect()).collect()
    });

    // CPU per frame: the divider-1 run (most bands per frame); the ISR over all runs
    let cpu: Vec<(u64, u64)> = cells
        .iter()
        .map(|row| {
            let c = &row.last().expect("divider 1").m.cpu;
            (*c.iter().max().unwrap_or(&0), c.iter().sum::<u64>() / c.len().max(1) as u64)
        })
        .collect();
    let isr_max = cells.iter().flatten().flat_map(|c| c.m.isr.iter().copied()).max().unwrap_or(0);

    let mut t = String::new();
    writeln!(t, "{:<10} {:>7} {:>8} {:>10} {:>8} {:>7}", "game", "divider", "fps", "bytes/frm", "max age", "frames")
        .unwrap();
    for ((id, _), row) in games.iter().zip(&cells) {
        for (d, c) in DIVIDERS.iter().zip(row) {
            writeln!(
                t,
                "{id:<10} {d:>7} {:>8} {:>10} {:>8} {:>7}",
                c.fps.map_or("static".to_string(), |f| format!("{f:.1}")),
                c.bytes_per_frame,
                c.max_age,
                c.m.frames
            )
            .unwrap();
        }
    }
    writeln!(t, "\n{:<10} {:>12} {:>12}", "game", "CPU max", "CPU avg").unwrap();
    for ((id, _), (max, avg)) in games.iter().zip(&cpu) {
        writeln!(t, "{id:<10} {max:>12} {avg:>12}").unwrap();
    }
    writeln!(t, "\nCPU budget per frame {CPU_PER_FRAME}; audio ISR max {isr_max} (budget {ISR_PER_BLOCK})").unwrap();
    writeln!(t, "RAMTEXT {ram_text} of {RAMTEXT_MAX} bytes").unwrap();
    writeln!(t, "image + pack {image_pack} of {APP_SLOT} ({APP_SLOT:#x}) bytes").unwrap();
    writeln!(t, "POOL {pool_used} bytes, {pool_free} bytes below the stack guard").unwrap();
    println!("\n{t}");

    let mut j = String::new();
    writeln!(j, "{{\n  \"window_ms\": {WINDOW_MS},\n  \"settle_ms\": {SETTLE_MS},").unwrap();
    writeln!(j, "  \"ram_text\": {ram_text},\n  \"ram_text_max\": {RAMTEXT_MAX},").unwrap();
    writeln!(j, "  \"image_and_pack\": {image_pack},\n  \"image_and_pack_max\": {APP_SLOT},").unwrap();
    writeln!(j, "  \"pool_used\": {pool_used},\n  \"pool_free\": {pool_free},").unwrap();
    writeln!(j, "  \"isr_max\": {isr_max},\n  \"isr_budget\": {ISR_PER_BLOCK},\n  \"cpu_budget\": {CPU_PER_FRAME},")
        .unwrap();
    writeln!(j, "  \"games\": [").unwrap();
    for (gi, ((id, _), row)) in games.iter().zip(&cells).enumerate() {
        writeln!(
            j,
            "    {{\n      \"id\": \"{id}\",\n      \"cpu_max\": {},\n      \"cpu_avg\": {},\n      \"dividers\": [",
            cpu[gi].0, cpu[gi].1
        )
        .unwrap();
        for (di, (d, c)) in DIVIDERS.iter().zip(row).enumerate() {
            let comma = if di + 1 < DIVIDERS.len() { "," } else { "" };
            writeln!(
                j,
                "        {{\"divider\": {d}, \"fps\": {}, \"bytes_per_frame\": {}, \"max_age\": {}, \"spectrum_frames\": {}, \"lcd_frames\": {}, \"row_sends\": {}, \"pending_frames_sum\": {}, \"bands\": {}}}{comma}",
                c.fps.map_or("null".to_string(), |f| format!("{f:.2}")), c.bytes_per_frame, c.max_age, c.m.frames, c.m.lcd.frames, c.m.lcd.row_sends, c.m.lcd.pending_frames_sum, c.m.lcd.bands
            )
            .unwrap();
        }
        writeln!(j, "      ]\n    }}{}", if gi + 1 < games.len() { "," } else { "" }).unwrap();
    }
    writeln!(j, "  ]\n}}").unwrap();
    let out = build_dir().join("fmzx-report.json");
    std::fs::write(&out, j).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

    // the runs were sound, then the budgets
    let demo_of = |id: &str| games.iter().find(|g| g.0 == id).expect("game").1.clone();
    for ((id, _), row) in games.iter().zip(&cells) {
        for (d, c) in DIVIDERS.iter().zip(row) {
            assert_eq!(c.m.resets, 0, "{id} at divider {d}: the firmware reset");
            assert!(c.m.in_game, "{id} at divider {d}: not in the game at the end");
            // the demo got into play and stayed there: rows went out in nearly every second (a
            // title screen, a menu or a game-over screen sends none)
            if !demo_of(id).is_empty() {
                let share = active_share(&c.m);
                assert!(
                    share >= GAMEPLAY_MIN_SHARE,
                    "{id} at divider {d}: rows sent in only {share:.2} of the 1 s sub-windows: {:?}",
                    c.m.rows_per_second
                );
            }
            // (a static screen, border.sna's, sends nothing: no frame rate to speak of)
            assert!(id == "border" || c.m.lcd.row_sends > 0, "{id} at divider {d}: nothing was sent");
        }
    }
    let fps_of = |id: &str, d: u8| {
        let (gi, _) = games.iter().enumerate().find(|(_, g)| g.0 == id).expect("game");
        cells[gi][DIVIDERS.iter().position(|&x| x == d).unwrap()].fps
    };
    let mut misses = Vec::new();
    let s4 = fps_of("scroll", 4).expect("scroll sends rows");
    if !(20.0..=23.0).contains(&s4) {
        misses.push(format!("scroll at divider 4: {s4:.1} fps, want 20-23"));
    }
    let s1 = fps_of("scroll", 1).expect("scroll sends rows");
    if s1 < 49.0 {
        misses.push(format!("scroll at divider 1: {s1:.1} fps, want >= 49"));
    }
    for (id, _) in &games {
        let Some(f) = fps_of(id, 1) else { continue };
        if f < 45.0 {
            misses.push(format!("{id} at divider 1: {f:.1} fps, want >= 45"));
        }
    }
    for ((id, _), (max, _)) in games.iter().zip(&cpu) {
        if *max > CPU_PER_FRAME {
            misses.push(format!("{id}: CPU per frame {max} > {CPU_PER_FRAME}"));
        }
    }
    if isr_max > ISR_PER_BLOCK {
        misses.push(format!("audio ISR {isr_max} > {ISR_PER_BLOCK}"));
    }
    assert!(misses.is_empty(), "budgets missed: {misses:#?}");
}

/// The gameplay check is not vacuous: Moon Caves stopped at its map menu and Castaway at its title
/// menu (the demos cut short) are in the game but send nothing, and the check rejects them.
#[test]
fn title_screens_are_not_gameplay() {
    let fw = Firmware::load(&build_dir());
    let pack = fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack");
    let fw = fw.with_pack(&pack);
    let opt = BootOpt { rgb565: false, divider: 1 };
    let (mooncaves, castaway) = std::thread::scope(|s| {
        let g = s.spawn(|| measure(&fw, opt, 2, &parse_demo("button.OCT-@8000").unwrap(), SETTLE_MS, 10_000));
        let c = s.spawn(|| measure(&fw, opt, 3, &parse_demo("button.OCT-@12000").unwrap(), SETTLE_MS, 10_000));
        (g.join().unwrap(), c.join().unwrap())
    });
    for (id, m) in [("mooncaves map menu", mooncaves), ("castaway menu", castaway)] {
        println!("{id}: rows per second {:?}", m.rows_per_second);
        assert!(m.in_game, "{id}: not in the game view");
        assert!(active_share(&m) < GAMEPLAY_MIN_SHARE, "{id} would pass as gameplay: {:?}", m.rows_per_second);
    }
}
