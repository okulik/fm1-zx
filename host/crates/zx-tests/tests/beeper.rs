// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

const FM1: (u32, u32) = (24_000_000, 544);
const FRAME: u32 = 69_888;

fn rate(r: (u32, u32)) -> f64 {
    r.0 as f64 / r.1 as f64
}

/// Splits absolute-T-state edges (ascending) into per-frame edge lists.
fn per_frame(abs: &[(u32, u8)], frames: u32) -> Vec<Vec<zx_edge>> {
    (0..frames)
        .map(|f| {
            abs.iter()
                .filter(|(t, _)| *t >= f * FRAME && *t < (f + 1) * FRAME)
                .map(|&(t, level)| zx_edge { tstate: t - f * FRAME, level })
                .collect()
        })
        .collect()
}

fn render_all(b: &mut Beeper, frames: &[Vec<zx_edge>]) -> Vec<i16> {
    frames.iter().flat_map(|e| b.render(e)).collect()
}

fn rising_crossings(s: &[i16]) -> Vec<usize> {
    (1..s.len()).filter(|&i| s[i - 1] <= 0 && s[i] > 0).collect()
}

#[test]
fn exact_sample_counts() {
    let mut b = Beeper::new(FM1.0, FM1.1);
    let total: usize = (0..119).map(|_| b.render(&[]).len()).sum();
    assert_eq!(total, 104_832);
    let mut b = Beeper::new(48_000, 1);
    let total: usize = (0..125).map(|_| b.render(&[]).len()).sum();
    assert_eq!(total, 119_808);
}

#[test]
fn frame_samples_matches_render() {
    let mut b = Beeper::new(FM1.0, FM1.1);
    for _ in 0..40 {
        let n = b.frame_samples() as usize;
        assert_eq!(b.render(&[]).len(), n);
    }
}

#[test]
fn silence_is_zero() {
    let mut b = Beeper::new(FM1.0, FM1.1);
    for _ in 0..10 {
        assert!(b.render(&[]).iter().all(|&x| x == 0));
    }
}

#[test]
fn dc_decays() {
    let mut b = Beeper::new(FM1.0, FM1.1);
    let mut out = b.render(&[zx_edge { tstate: 0, level: 2 }]);
    for _ in 0..49 {
        out.extend(b.render(&[]));
    }
    assert!(out[0].abs() > 1000, "step passes through: {}", out[0]);
    assert!(out[out.len() - 100..].iter().all(|x| x.abs() < 200));
}

#[test]
fn square_wave_pitch() {
    let mut b = Beeper::new(FM1.0, FM1.1);
    let abs: Vec<(u32, u8)> = (1..=(50 * FRAME / 1750)).map(|k| (k * 1750, if k % 2 == 1 { 2 } else { 0 })).collect();
    let out = render_all(&mut b, &per_frame(&abs, 50));
    let crossings = out.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f64;
    let secs = out.len() as f64 / rate(FM1);
    let hz = crossings / secs;
    assert!((hz - 2000.0).abs() < 40.0, "crossings/s = {hz}");
}

#[test]
fn box_filter_half_level() {
    // 35 kHz: each sample spans exactly 100 T-states.
    let mut b = Beeper::new(35_000, 1);
    b.test_dc_block(false);
    let out = b.render(&[zx_edge { tstate: 250, level: 2 }]);
    assert_eq!(out[1], 0);
    assert!((out[2] as i32 - 6000).abs() <= 1, "{}", out[2]);
    assert_eq!(out[3], 12000);
    // a quarter-way edge, and levels MIC / both
    let mut b = Beeper::new(35_000, 1);
    b.test_dc_block(false);
    let out = b.render(&[zx_edge { tstate: 125, level: 1 }, zx_edge { tstate: 300, level: 3 }]);
    assert!((out[1] as i32 - 750).abs() <= 1, "{}", out[1]);
    assert_eq!(out[2], 1000);
    assert_eq!(out[3], 13000);
}

#[test]
fn level_carries_across_frames() {
    let mut b = Beeper::new(48_000, 1);
    b.test_dc_block(false);
    b.render(&[zx_edge { tstate: 100, level: 2 }]);
    let out = b.render(&[]);
    assert!(out.iter().all(|&x| x == 12000));
}

#[test]
fn matches_machine_edges() {
    let mut m = Machine::new();
    // LD HL,1642; LD DE,26; CALL 03B5h; JR $
    load(&mut m, 0x8000, &[0x21, 0x6A, 0x06, 0x11, 0x1A, 0x00, 0xCD, 0xB5, 0x03, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    let mut b = Beeper::new(FM1.0, FM1.1);
    let mut out = Vec::new();
    let mut edges = 0;
    for _ in 0..8 {
        m.run_frame();
        edges += m.edges().len();
        out.extend(b.render(m.edges()));
    }
    assert!(edges > 40, "beeper ran: {edges} edges");
    let rc = rising_crossings(&out);
    assert!(rc.len() >= 20, "{} rising crossings", rc.len());
    let hz = (rc.len() - 1) as f64 * rate(FM1) / (rc[rc.len() - 1] - rc[0]) as f64;
    assert!((hz - 261.6).abs() < 261.6 * 0.02, "pitch {hz}");
}
