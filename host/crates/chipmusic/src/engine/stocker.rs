// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Stocker: two tone channels (phase accumulators: a channel adds its step to a 16-bit sum each
//! pass and sounds while the sum's carry says so) with a volume each from its envelope, updated
//! once a tick (a fiftieth of a second), and four drums (kick, hat, snare, tom) that interrupt
//! them.
//!
//! The tick's sound loop is found by its code (`ADD IX,DE; SBC A,A; AND vol1; LD B,A; EXX; ADD
//! HL,DE; EXX; SBC A,A; AND vol2; OR B; JR Z`), after `LD HL,nn` (H the passes of a tick): at
//! that `LD HL` the tap reads channel 1's step from DE, channel 2's from DE' and the volumes from
//! the `AND` operands. The pass time is a tick's (the median time between two ticks) over its
//! passes (the loop's pass time varies with the volumes: the tick's average is what sets the
//! pitch). A note starts when a channel sounds (step and volume above 0) after silence, when its
//! step moves by more than 1/32, or when its volume rises (the envelope restarts); its level is
//! 255 x volume / 30 at its start. Period: 65,536 / step passes. The drums are their code's
//! `OUT`s: kick `LD A,(HL); AND C; OUT (FEh),A; INC HL; LD A,6Eh`, hat `LD A,(HL); INC HL; AND
//! 10h; OUT (FEh),A; XOR A`, snare `LD A,(HL); AND 10h; INC HL; OUT (FEh),A; LD A,(HL)`, tom `XOR
//! 10h; OUT (FEh),A; LD D,H; LD E,L`, a hit's `OUT`s at most 15 ms apart.

use super::{byte, find_near, find_ram, median, ntropic::drums_text, Chan, Engine, EngineNote, Hit, Mem, Row, Sites};
use zx_sys::zx_regs;

pub struct Stocker;

/// The tick's sound loop; the tick's `LD HL,nn` 3 bytes before it.
const LOOP: &str = "DD 19 9F E6 ?? 47 D9 19 D9 9F E6 ?? B0 28";
/// The drums: their code, the `OUT`'s offset in it and the drum.
const DRUMS: [(&str, u16, &str); 4] =
    [("7E A1 D3 FE 23 3E 6E", 2, "kick"), ("7E 23 E6 10 D3 FE AF", 4, "hat"), ("7E E6 10 23 D3 FE 7E", 4, "snare"), ("EE 10 D3 FE 54 5D", 2, "tom")];
const SPAN: u16 = 0x200;
/// The volume of a full level.
const FULL: f64 = 30.0;

impl Engine for Stocker {
    fn name(&self) -> &'static str {
        "Stocker"
    }

    fn channels(&self) -> usize {
        2
    }

    fn locate(&self, mem: &Mem) -> Option<Sites> {
        let g = find_ram(mem, LOOP)?;
        let row_pc = g - 3;
        if byte(mem, row_pc) != 0x21 {
            return None;
        }
        let drums = DRUMS.iter().filter_map(|&(p, at, kind)| find_near(mem, p, g, SPAN).map(|d| (d + at, kind))).collect();
        Some(Sites { anchor: g, row_pc, check: vec![(row_pc, 0x21), (g, 0xDD), (g + 1, 0x19)], pass_out: None, drums, ops: vec![g + 4, g + 11, g - 1] })
    }

    fn read(&self, s: &Sites, regs: &zx_regs, mem: &Mem) -> Option<Row> {
        let ch = |step: u16, vol: u8| Chan { value: u32::from(step), duty: 128, volume: vol };
        let passes = u32::from(byte(mem, s.ops[2])).max(1);
        Some(Row { ch: vec![ch(regs.de, byte(mem, s.ops[0])), ch(regs.de_, byte(mem, s.ops[1]))], passes, ..Row::default() })
    }

    fn period_t(&self, value: u32, pass_t: f64) -> f64 {
        65536.0 * pass_t / f64::from(value.max(1))
    }

    fn level(&self, c: &Chan) -> u8 {
        (255.0 * f64::from(c.volume) / FULL).round().clamp(1.0, 255.0) as u8
    }

    fn pass_t(&self, rows: &[Row]) -> Option<f64> {
        let tick = median(rows.windows(2).map(|w| (w[1].t - w[0].t) as f64).collect())?;
        let passes = median(rows.iter().map(|r| f64::from(r.passes)).collect())?;
        Some(tick / passes)
    }

    fn notes(&self, rows: &[Row], ends: &[u64], pass_t: f64) -> (Vec<EngineNote>, Vec<Hit>) {
        let mut out = Vec::new();
        for c in 0..2 {
            let mut cur: Option<EngineNote> = None;
            let mut prev_vol = 0;
            for (r, &end) in rows.iter().zip(ends) {
                let ch = r.ch[c];
                let on = ch.value > 0 && ch.volume > 0;
                let new = on && cur.is_none_or(|n| ch.value.abs_diff(n.value) > n.value / 32 || ch.volume > prev_vol || n.end_t != r.t);
                if let Some(n) = cur.as_mut() {
                    if on && !new {
                        n.end_t = end;
                    } else {
                        out.push(*n);
                        cur = None;
                    }
                }
                if new {
                    cur = Some(EngineNote {
                        channel: c as u8,
                        start_t: r.t,
                        end_t: end,
                        value: ch.value,
                        period_t: self.period_t(ch.value, pass_t),
                        level: self.level(&ch),
                        duty: ch.duty,
                    });
                }
                prev_vol = if on { ch.volume } else { 0 };
            }
            out.extend(cur);
        }
        out.retain(|n| n.end_t > n.start_t);
        out.sort_by_key(|n| (n.start_t, n.channel));
        (out, Vec::new())
    }

    fn drum_gap_t(&self) -> u64 {
        52_500
    }

    fn what(&self, s: &Sites) -> String {
        format!(
            "its ticks at {:04X}h (steps in DE and DE', volumes from the sound loop's operands), the pass time from the ticks, {}",
            s.row_pc,
            drums_text(s)
        )
    }
}
