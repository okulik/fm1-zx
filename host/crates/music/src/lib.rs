// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host side of the FM music library (`core/sound`): helpers shared by its tests and tools.

use std::path::{Path, PathBuf};
use zx_sys::{zx_edge, Beeper, Machine};

/// The FM-1's audio rate, 750000 / 17 Hz (24 MHz / 544).
pub const RATE: (u32, u32) = (750_000, 17);
/// Z80 clock: `snd_voice.period_q8` counts its T-states x 256.
pub const CPU_HZ: f64 = 3_500_000.0;

/// The sample rate in Hz.
pub fn sample_rate() -> f64 {
    RATE.0 as f64 / RATE.1 as f64
}

/// The repository root.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// `snd_voice.period_q8` of a frequency.
pub fn period_q8(hz: f64) -> u32 {
    (CPU_HZ * 256.0 / hz).round() as u32
}

/// The frequency of a `snd_voice.period_q8`.
pub fn hz_of_period_q8(p: u32) -> f64 {
    CPU_HZ * 256.0 / p as f64
}

/// One emulated frame as the firmware sees it: the beeper edges, the level they start from and
/// the beeper's rendered samples.
pub struct Frame {
    pub edges: Vec<zx_edge>,
    pub start_level: u8,
    pub beeper: Vec<i16>,
}

/// Runs `frames` frames of `m`, rendering the beeper the way the firmware does.
pub fn capture(m: &mut Machine, beeper: &mut Beeper, frames: usize) -> Vec<Frame> {
    (0..frames)
        .map(|_| {
            m.run_frame();
            let start_level = m.edges_start_level();
            beeper.set_level(start_level);
            let edges = m.edges().to_vec();
            let samples = beeper.render(&edges);
            Frame { edges, start_level, beeper: samples }
        })
        .collect()
}

/// A machine with `games/test/<name>` (a `.sna`) loaded.
pub fn test_sna(name: &str) -> Machine {
    let data = std::fs::read(repo().join("games/test").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut m = Machine::new();
    m.load_sna(&data).expect("loads");
    m
}

/// A machine autostarting the pack's Castaway tape.
pub fn castaway() -> Machine {
    let tap = std::fs::read(repo().join("third_party/games/castaway/castawy.tap")).expect("castawy.tap");
    let mut m = Machine::new();
    m.tape_insert(&tap).expect("inserts");
    m.tape_autostart();
    m
}
