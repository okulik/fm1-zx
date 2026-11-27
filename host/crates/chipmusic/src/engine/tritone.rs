// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tritone: three tone channels (phase accumulators: a channel adds its step to a 16-bit sum
//! each pass of the sound loop and sounds high while the sum's high byte is under its duty),
//! with an interrupting drum (a tone burst or a noise burst) and a tempo per pattern.
//!
//! The sound loop is found by its code (`ADD HL,BC; LD A,H; EXX; CP duty0; SBC A,A; AND C; ADD
//! IX,DE; OUT (FEh),A; ...`, the volume form, or the form with one volume, `ADD HL,BC; LD A,H; CP
//! duty0; SBC A,A; EXX; AND C; OUT (FEh),A; ...`), the duties as the `CP` operands in it. The row
//! read is the row's set-up before it, `LD DE,speed; LD BC,step0; LD HL,..; EXX; LD DE,step1; LD
//! SP,step2; EXX`: at `LD BC` the tap reads the three steps from the operands (a step 0 or a
//! duty 0 is silence). The pass time is the spacing of channel 0's `OUT` (once a pass); the drums
//! are the `OUT`s of the tone burst (`XOR D; OUT (FEh),A; DJNZ`) and of the noise burst (`LD
//! A,(HL); AND D; OUT (FEh),A; AND (IX+0); DEC E; OUT (FEh),A`). Period: 65,536 / step passes.
//! Level: the duty's pulse width, 255 x sqrt(sin(pi x w)) with w the narrower side's share.

use super::{byte, find_near, find_ram, ntropic::drums_text, word, Chan, Engine, Mem, Row, Sites};
use zx_sys::zx_regs;

pub struct Tritone;

/// The sound loop's two forms: its signature, the duties' operands and channel 0's `OUT`, as
/// offsets from its start.
const LOOPS: [(&str, [u16; 3], u16); 2] = [
    ("09 7C D9 FE ?? 9F A1 DD 19 D3 FE DD 7C FE ?? 9F A1 D3 FE 39 7C FE ?? 9F A1 D9 1D D3 FE", [0x04, 0x0E, 0x16], 0x09),
    ("09 7C FE ?? 9F D9 A1 D3 FE DD 19 DD 7C FE ?? 9F A1 D3 FE 39 7C FE ?? 9F A1 D9 1D D3 FE", [0x03, 0x0E, 0x16], 0x07),
];
/// The row's set-up before the loop: `LD DE,nn; LD BC,nn; LD HL,nn; EXX; LD DE,nn; LD SP,nn;
/// EXX`, from 11h bytes before it.
const SETUP: [(u16, u8); 7] = [(0x11, 0x11), (0x0E, 0x01), (0x0B, 0x21), (0x08, 0xD9), (0x07, 0x11), (0x04, 0x31), (0x01, 0xD9)];
/// The tone burst's `OUT` (1 byte in) and the noise burst's two (2 and 8 bytes in).
const TONE: &str = "AA D3 FE 10";
const NOISE: &str = "7E A2 D3 FE DD A6 00 1D D3 FE";
const SPAN: u16 = 0x180;

impl Engine for Tritone {
    fn name(&self) -> &'static str {
        "Tritone"
    }

    fn channels(&self) -> usize {
        3
    }

    fn locate(&self, mem: &Mem) -> Option<Sites> {
        let (l, duty, out) = LOOPS.iter().find_map(|&(p, duty, out)| {
            let l = find_ram(mem, p)?;
            SETUP.iter().all(|&(back, b)| byte(mem, l.wrapping_sub(back)) == b).then_some((l, duty, out))
        })?;
        let mut drums = Vec::new();
        if let Some(d) = find_near(mem, TONE, l, SPAN) {
            drums.push((d + 1, "tone"));
        }
        if let Some(d) = find_near(mem, NOISE, l, SPAN) {
            drums.extend([(d + 2, "noise"), (d + 8, "noise")]);
        }
        let row_pc = l - 0x0E;
        // the steps' operands, then the duties'
        let ops = vec![l - 0x0D, l - 0x06, l - 0x03, l + duty[0], l + duty[1], l + duty[2]];
        Some(Sites { anchor: l, row_pc, check: vec![(row_pc, 0x01), (l, 0x09)], pass_out: Some(l + out), drums, ops })
    }

    fn read(&self, s: &Sites, regs: &zx_regs, mem: &Mem) -> Option<Row> {
        let ch = |c: usize| {
            let (step, duty) = (u32::from(word(mem, s.ops[c])), byte(mem, s.ops[3 + c]));
            Chan { value: if duty == 0 { 0 } else { step }, duty, volume: 0 }
        };
        let (e, d) = (u32::from(regs.de & 0xFF), u32::from(regs.de >> 8));
        let passes = if e == 0 { 256 } else { e } + d.saturating_sub(1) * 256;
        Some(Row { ch: (0..3).map(ch).collect(), passes, ..Row::default() })
    }

    fn period_t(&self, value: u32, pass_t: f64) -> f64 {
        65536.0 * pass_t / f64::from(value.max(1))
    }

    fn level(&self, c: &Chan) -> u8 {
        let w = f64::from(c.duty.min(255 - c.duty).max(1)) / 256.0;
        (255.0 * (std::f64::consts::PI * w).sin().sqrt()).round().clamp(1.0, 255.0) as u8
    }

    fn what(&self, s: &Sites) -> String {
        format!(
            "its rows at {:04X}h (three steps and duties from the sound loop's operands), the pass time from the OUT at {:04X}h, {}",
            s.row_pc,
            s.pass_out.unwrap_or(0),
            drums_text(s)
        )
    }
}
