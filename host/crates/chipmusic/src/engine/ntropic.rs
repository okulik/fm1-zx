// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! ntropic: two tone channels (counters: a channel's level flips every `value` passes of the
//! sound loop), a noise channel and a click drum.
//!
//! A row is 4 bytes at HL: the speed with the drum flag in bit 0, channel 1's counter, channel
//! 2's (0: silent) and the noise's length (bit 7 set: no noise). The tap reads it at the row
//! read, `LD A,(HL)` after `LD IYH,10h; POP HL` (the signature `FD 26 10 E1 7E 3C C8`; `INC A;
//! RET Z` ends the song at FFh). The pass time is the spacing of the sound loop's `OUT` in `EX
//! AF,AF'; OUT (FEh),A; DEC D; JP NZ` (once a pass), the click drum the `OUT` of `LD A,(HL); AND
//! F8h; OUT (FEh),A; DEC HL; DEC BC`. Period: 2 x counter x pass time; the noise a drum hit at
//! the row, (length x 256 + 1) passes long at most.

use super::{byte, find_near, find_ram, Chan, Engine, EngineNote, Hit, Mem, Row, Sites};
use zx_sys::zx_regs;

pub struct Ntropic;

/// The row read's signature; the row read itself is 4 bytes in.
const ROW: &str = "FD 26 10 E1 7E 3C C8";
/// The sound loop's `OUT`, once a pass (the `OUT` 1 byte in).
const PASS: &str = "08 D3 FE 15 C2";
/// The click drum's `OUT` (3 bytes in).
const DRUM: &str = "7E E6 F8 D3 FE 2B 0B";
/// How far from the row read the sound loop and the drum lie.
const SPAN: u16 = 0x180;

impl Engine for Ntropic {
    fn name(&self) -> &'static str {
        "ntropic"
    }

    fn channels(&self) -> usize {
        2
    }

    fn locate(&self, mem: &Mem) -> Option<Sites> {
        let r = find_ram(mem, ROW)?;
        let pass = find_near(mem, PASS, r, SPAN)? + 1;
        let drums = find_near(mem, DRUM, r, SPAN).map(|d| vec![(d + 3, "kick")]).unwrap_or_default();
        let row_pc = r + 4;
        Some(Sites { anchor: r, row_pc, check: vec![(row_pc, 0x7E), (row_pc + 1, 0x3C), (row_pc + 2, 0xC8)], pass_out: Some(pass), drums, ops: Vec::new() })
    }

    fn read(&self, _sites: &Sites, regs: &zx_regs, mem: &Mem) -> Option<Row> {
        let hl = regs.hl;
        let at = |k: u16| byte(mem, hl.wrapping_add(k));
        if at(0) == 0xFF {
            return None;
        }
        let ch = |v: u8| Chan { value: u32::from(v), duty: 128, volume: 0 };
        let nz = at(3);
        Some(Row { ch: vec![ch(at(1)), ch(at(2))], noise: (nz & 0x80 == 0).then_some(u32::from(nz)), ..Row::default() })
    }

    fn period_t(&self, value: u32, pass_t: f64) -> f64 {
        2.0 * f64::from(value) * pass_t
    }

    fn notes(&self, rows: &[Row], ends: &[u64], pass_t: f64) -> (Vec<EngineNote>, Vec<Hit>) {
        let notes = super::split_notes(self, rows, ends, pass_t);
        let hits = rows
            .iter()
            .zip(ends)
            .filter_map(|(r, &end)| {
                let len = (f64::from(r.noise? * 256 + 1) * pass_t) as u64;
                Some(Hit { t: r.t, end_t: (r.t + len).min(end), kind: "noise" })
            })
            .collect();
        (notes, hits)
    }

    fn what(&self, s: &Sites) -> String {
        format!(
            "its rows at {:04X}h (4 bytes at HL: drum flag, two counters, noise), the pass time from the OUT at {:04X}h, {}",
            s.row_pc,
            s.pass_out.unwrap_or(0),
            drums_text(s)
        )
    }
}

/// "the drum OUTs at 9D14h (kick)", or "no drum OUT found".
pub(crate) fn drums_text(s: &Sites) -> String {
    if s.drums.is_empty() {
        return "no drum OUT found".into();
    }
    let list: Vec<String> = s.drums.iter().map(|(a, k)| format!("{a:04X}h ({k})")).collect();
    format!("the drum OUT{} at {}", if list.len() == 1 { "" } else { "s" }, list.join(", "))
}
