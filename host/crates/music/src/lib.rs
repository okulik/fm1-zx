// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host side of the FM music library (`core/sound`): helpers shared by its tests and tools.

use std::path::{Path, PathBuf};
use zx_sys::{zx_edge, Beeper, Machine, SndDecoder, SndFrame, SND_DELAY_FRAMES};

pub mod corpus;
pub mod preview;
pub mod score;
pub mod synthetic;
pub mod truth;
pub use fmzx_pack::tzx;
pub mod wav;

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
    pack_tape("castaway/castawy.tap")
}

/// A machine autostarting the pack's Escape from Colony 8 tape.
pub fn colony8() -> Machine {
    pack_tape("colony8/colony8.tap")
}

/// A machine autostarting the pack's tape `third_party/games/<path>`.
pub fn pack_tape(path: &str) -> Machine {
    let tap = std::fs::read(repo().join("third_party/games").join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut m = Machine::new();
    m.tape_insert(&tap).expect("inserts");
    m.tape_autostart();
    m
}

/// The beeper samples of a generated stream, frame by frame, rendered as the firmware does.
pub fn render_stream(s: &synthetic::Stream) -> Vec<Frame> {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    beeper.set_level(s.start_level);
    let mut level = s.start_level;
    s.frames
        .iter()
        .map(|edges| {
            let start_level = level;
            level = edges.last().map_or(level, |e| e.level);
            Frame { edges: edges.clone(), start_level, beeper: beeper.render(edges) }
        })
        .collect()
}

/// The decoder's decision about each frame of `frames` (edges and the level they start from):
/// frame t's decision comes with frame t + SND_DELAY_FRAMES, so empty frames follow to flush it.
pub fn decode<'a>(frames: impl IntoIterator<Item = (&'a [zx_edge], u8)>) -> Vec<SndFrame> {
    let mut d = SndDecoder::new();
    let mut out = Vec::new();
    let mut level = 0;
    for (edges, start_level) in frames {
        out.push(d.frame(edges, start_level));
        level = edges.last().map_or(start_level, |e| e.level);
    }
    for _ in 0..SND_DELAY_FRAMES {
        out.push(d.frame(&[], level));
    }
    out.drain(..SND_DELAY_FRAMES as usize);
    out
}

/// The decisions about a generated stream's frames.
pub fn decode_stream(s: &synthetic::Stream) -> Vec<SndFrame> {
    let mut level = s.start_level;
    let starts: Vec<u8> = s
        .frames
        .iter()
        .map(|e| {
            let l = level;
            level = e.last().map_or(level, |x| x.level);
            l
        })
        .collect();
    decode(s.frames.iter().zip(starts).map(|(e, l)| (e.as_slice(), l)))
}

/// The pitches (Hz) of a decision's sounding tonal voices.
pub fn voices_hz(fr: &SndFrame) -> Vec<f64> {
    fr.tone.iter().filter(|v| v.period_q8 != 0).map(|v| hz_of_period_q8(v.period_q8)).collect()
}

/// The pitches played in each frame: the voices of confident decisions only (the others play
/// the beeper).
pub fn played_hz(frames: &[SndFrame]) -> Vec<Vec<f64>> {
    frames.iter().map(|f| if f.confident != 0 { voices_hz(f) } else { Vec::new() }).collect()
}
