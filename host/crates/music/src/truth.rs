// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ground truth for scoring: the notes a game's music engine plays, read from its RAM (operands
//! and registers of the sound loop) at the end of each frame. Test and corpus code only; the
//! decoder never sees any of this.
//!
//! - Castaway (title tune; code C7EAh-C8B6h, loop C868h-C88Ah, 154 T plus contention): two
//!   square channels, each a `DEC` counter reloaded from C86Dh / C87Ah, on while the masks at
//!   C875h / C885h are non-zero. The phase counter wraps at (C871h / C881h) + 1: 1 is a square
//!   wave, 3 reads ROM bytes 0-3 as the waveform, an octave down (a 75% pulse).
//!   f = 3.5 MHz / ((wrap + 1) x N x loop).
//! - Escape from Colony 8 (loop 94FCh-951Eh, 153 T plus contention): three 16-bit phase
//!   accumulators with steps BC / DE (swapped with the alternate set inside the loop) and SP,
//!   duty thresholds at 9500h / 950Ah / 9512h (0 = silent). f = step x 3.5 MHz / (loop x 65536).
//!   The drum routines at 9496h (a tone burst) and 94ADh (ROM noise) stop the loop.
//! - Cave Runner (for the local corpus tool only; never loaded by committed tests): the title
//!   tune (loop 92F8h-930Dh, 56.6 T per pass, IY pointing at [duration, N1, N2],
//!   f = 3.5 MHz / (2 x N x 56.6)) and the in-game tune (burst loop 8852h, 40 T per pass, note
//!   index 845Bh into the table at 858Ch, flag 845Ch bit 1 = off, f = 3.5 MHz / (2 x E x 40)).

use zx_sys::Machine;

const CPU_HZ: f64 = 3_500_000.0;

/// Mean engine loop lengths in T-states, measured from the edge streams (I/O contention
/// included).
pub const LOOP_CASTAWAY: f64 = 156.6;
pub const LOOP_COLONY8: f64 = 155.9;
pub const LOOP_MM_TITLE: f64 = 56.6;
pub const LOOP_MM_GAME: f64 = 40.0;

/// The engines with a note reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Castaway,
    Colony8,
    CaveRunner,
}

impl Engine {
    /// The engine of a game file, by its file name (as the pack and the usual releases name it).
    pub fn of_file(name: &str) -> Option<Engine> {
        let n = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
        match n.as_str() {
            "castawy.tap" | "castaway.tap" => Some(Engine::Castaway),
            "colony8.tap" => Some(Engine::Colony8),
            "mm.tap" | "cave.tap" | "caverunner.tap" | "cave runner.tap" => Some(Engine::CaveRunner),
            _ => None,
        }
    }

    /// The pitches (Hz) the engine plays at the end of the frame just run: `None` when the PC is
    /// outside the engine (unknown, not scored), an empty list when it is silent.
    pub fn read(self, m: &Machine) -> Option<Vec<f64>> {
        self.read_channels(m).map(|v| v.into_iter().map(|(_, hz)| hz).collect())
    }

    /// As [`Engine::read`], each pitch with its engine channel (0-based).
    pub fn read_channels(self, m: &Machine) -> Option<Vec<(usize, f64)>> {
        match self {
            Engine::Castaway => castaway(m),
            Engine::Colony8 => colony8(m),
            Engine::CaveRunner => cave_runner(m),
        }
    }
}

/// Castaway's title tune.
pub fn castaway(m: &Machine) -> Option<Vec<(usize, f64)>> {
    let r = m.regs();
    if !(0xc7ea..=0xc8b6).contains(&r.pc) {
        return None;
    }
    let mut v = Vec::new();
    for (ch, (n_at, mask_at, wrap_at)) in [(0xc86du16, 0xc875u16, 0xc871u16), (0xc87a, 0xc885, 0xc881)].into_iter().enumerate() {
        let n = m.peek(n_at) as f64;
        let wrap = m.peek(wrap_at) as f64 + 1.0;
        if m.peek(mask_at) != 0 && n > 0.0 {
            v.push((ch, CPU_HZ / (wrap * n * LOOP_CASTAWAY)));
        }
    }
    Some(v)
}

/// The PC is in Colony 8's music loop.
fn colony8_in_loop(pc: u16) -> bool {
    (0x94fc..=0x951e).contains(&pc)
}

/// Colony 8's music: the three channels' pitches while the loop runs.
pub fn colony8(m: &Machine) -> Option<Vec<(usize, f64)>> {
    let r = m.regs();
    if !colony8_in_loop(r.pc) {
        return None;
    }
    // ch1's step is BC and ch2's DE; the loop swaps register sets with EXX, so which set holds
    // them depends on where in the loop the frame ended
    let y_current = (0x94ff..=0x9515).contains(&r.pc);
    let (bc, de) = if y_current { (r.bc_, r.de) } else { (r.bc, r.de_) };
    let mut v = Vec::new();
    for (ch, (step, thr)) in [(bc, 0x9500u16), (de, 0x950a), (r.sp, 0x9512)].into_iter().enumerate() {
        if m.peek(thr) != 0 && step != 0 {
            v.push((ch, step as f64 * CPU_HZ / (LOOP_COLONY8 * 65536.0)));
        }
    }
    Some(v)
}

/// Colony 8's drums, from its sequencer's song pointer (the operand at 93ECh, stored after each
/// row it reads). A row is the 3 channels' fields, each 0 (silent), 1 (unchanged) or a
/// duty/step-high byte and a step-low byte (the first channel's first byte is 0, 1 or 80h-FEh); a
/// byte 02h-7Fh in place of a row is a drum (02h-1Bh the tone burst at 9496h, 1Ch-7Fh ROM noise at 94B1h), which
/// stops the music loop for about 23,000 T; FFh ends the pattern (the next pattern's address is
/// the word before the pattern-list pointer at 94CEh). Reading the rows passed since the last
/// frame tells exactly which frames a drum starts in, which sampling the PC at frame ends cannot.
#[derive(Clone, Debug, Default)]
pub struct Colony8Drums {
    ptr: Option<u16>,
}

impl Colony8Drums {
    pub fn new() -> Self {
        Colony8Drums { ptr: None }
    }

    /// After a frame: whether a drum started in it.
    pub fn frame(&mut self, m: &Machine) -> bool {
        let word = |a: u16| m.peek(a) as u16 | (m.peek(a.wrapping_add(1)) as u16) << 8;
        let now = word(0x93ec);
        let prev = self.ptr.replace(now);
        let Some(prev) = prev else { return false };
        if prev == now {
            return false;
        }
        // the rows from `from` up to `now`: Some(drum seen) if `now` is reached
        let scan = |from: u16| -> Option<bool> {
            let mut p = from;
            let mut drum = false;
            for _ in 0..64 {
                if p == now {
                    return Some(drum);
                }
                let b = m.peek(p);
                if (2..0x80).contains(&b) {
                    drum = true;
                    p = p.wrapping_add(1);
                    continue;
                }
                if b == 0xff {
                    return None;
                }
                for _ in 0..3 {
                    let c = m.peek(p);
                    p = p.wrapping_add(if c < 2 { 1 } else { 2 });
                }
            }
            None
        };
        if let Some(d) = scan(prev) {
            return d;
        }
        // a new pattern began: its rows start after its note length
        let list = word(0x94ce);
        let start = word(list.wrapping_sub(2)).wrapping_add(2);
        scan(start).unwrap_or(false)
    }
}

/// Cave Runner's title or in-game tune.
pub fn cave_runner(m: &Machine) -> Option<Vec<(usize, f64)>> {
    let r = m.regs();
    if (0x92dc..=0x932f).contains(&r.pc) {
        let mut v = Vec::new();
        for ch in 0..2u16 {
            let n = m.peek(r.iy.wrapping_add(1 + ch)) as f64;
            let n = if n == 0.0 { 256.0 } else { n };
            v.push((ch as usize, CPU_HZ / (2.0 * n * LOOP_MM_TITLE)));
        }
        return Some(v);
    }
    if (0x8000..0x9000).contains(&r.pc) && m.peek(0x845c) & 2 == 0 {
        let idx = m.peek(0x845b);
        let e = m.peek(0x858c + ((idx & 0x7e) >> 1) as u16) as f64;
        if e > 0.0 {
            return Some(vec![(0, CPU_HZ / (2.0 * e * LOOP_MM_GAME))]);
        }
    }
    None
}
