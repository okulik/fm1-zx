// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The speaker's spectrum: whether a pitch sounds in the speaker's output over the last few
//! frames, for the observing learner ([`crate::observe`]).
//!
//! # The signal
//!
//! A frame's speaker level comes from its edges (`zx_edge`: bit 1 of the level is the speaker,
//! EAR) and the level the frame starts at. It is sampled every `dt` T-states, each sample the
//! fraction of its `dt` T-states the speaker is high: a box filter, so a pulse a few T-states
//! wide still counts for its width, and the harmonics far above the sample rate do not fold
//! back at full strength. With `dt` = 128 the sample rate is 27,344 Hz, far above the pitches
//! music uses. [`Signal`] keeps the last N frames; the tests run over a [`View`] of the last few
//! of them ([`Signal::last`]): a longer view tells closer pitches apart, a shorter one fits a
//! shorter note.
//!
//! # The test
//!
//! The power at a frequency is one Goertzel filter over the view: the mean taken out, a Hann
//! window applied (its sidelobes are 31 dB down, so the strong harmonics of several mixed pulse
//! channels do not leak into each other's bins as a plain window lets them), and normalised so a
//! sine of amplitude `a` has power `a² / 4`. A pitch is *present* ([`View::present`]) when its
//! power is more than [`SALIENCE`] times the power one semitone either side, and at least
//! [`FLOOR`] of the view's variance. When a semitone is narrower than the Hann window's main
//! lobe (over 4 frames: below about 420 Hz) the neighbours are taken that lobe's half width away
//! (two bins, `2 / view` Hz), where a pure tone has no power. [`View::octave_below`] asks the
//! same of half the frequency: a pulse train has a strong second harmonic, so a pitch predicted
//! an octave too high is present too, but then so is the octave below it.

use std::f64::consts::PI;
use zx_sys::{zx_edge, ZX_FRAME_TSTATES};

/// The Spectrum's clock, Hz.
pub const CLOCK_HZ: f64 = 3_500_000.0;
/// A present pitch's power over the larger of its neighbours' (on real multi-channel tunes a
/// channel's pitch passes it in most of the windows its note fills, a wrong pitch in few).
pub const SALIENCE: f64 = 3.0;
/// A present pitch's least power, as a share of the window's variance (a pulse train of duty
/// `d` has about `d` of its variance in the fundamental: 1% pulses still pass).
pub const FLOOR: f64 = 0.001;
/// One semitone, as a frequency ratio.
const SEMITONE: f64 = 1.059_463_094_359_295_3;

/// The speaker level over the last frames, sampled every `dt` T-states.
#[derive(Clone, Debug)]
pub struct Signal {
    dt: u32,
    per_frame: usize,
    window: usize,
    /// The last `window` frames' samples (fewer at the start), oldest first.
    samples: Vec<f32>,
    /// The Hann window over `k + 1` frames, for each `k` below `window`.
    hann: Vec<Vec<f64>>,
}

/// The last few frames of a [`Signal`]: the spectral tests run over a view.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    samples: &'a [f32],
    hann: &'a [f64],
    rate: f64,
}

/// One frame's samples: each the fraction of its `dt` T-states the speaker is high. `dt` must
/// divide the frame's T-states (a power of two up to 256 does).
pub fn frame_levels(edges: &[zx_edge], start_level: u8, dt: u32) -> Vec<f32> {
    let n = (ZX_FRAME_TSTATES / dt) as usize;
    let mut out = vec![0f32; n];
    let mut high = (start_level >> 1) & 1 != 0;
    let mut t = 0u32;
    let mut add = |from: u32, to: u32| {
        // [from, to) is high
        let mut a = from;
        while a < to {
            let i = (a / dt) as usize;
            let end = ((i as u32 + 1) * dt).min(to);
            if let Some(s) = out.get_mut(i) {
                *s += (end - a) as f32 / dt as f32;
            }
            a = end;
        }
    };
    for e in edges {
        let at = e.tstate.min(ZX_FRAME_TSTATES);
        if high {
            add(t, at.max(t));
        }
        t = at.max(t);
        high = (e.level >> 1) & 1 != 0;
    }
    if high {
        add(t, ZX_FRAME_TSTATES);
    }
    out
}

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / n as f64).cos()).collect()
}

impl Signal {
    /// Samples every `dt` T-states (dividing the frame's T-states), over the last `window`
    /// frames.
    pub fn new(dt: u32, window: usize) -> Self {
        assert!(dt > 0 && ZX_FRAME_TSTATES.is_multiple_of(dt), "dt must divide the frame");
        let per_frame = (ZX_FRAME_TSTATES / dt) as usize;
        let window = window.max(1);
        let hann = (1..=window).map(|k| hann(k * per_frame)).collect();
        Signal { dt, per_frame, window, samples: Vec::with_capacity(per_frame * window), hann }
    }

    /// Adds a frame from its edges and the level it starts at
    /// ([`Machine::edges_start_level`](zx_sys::Machine::edges_start_level)), dropping the
    /// oldest frame once the window is full.
    pub fn push_frame(&mut self, edges: &[zx_edge], start_level: u8) {
        let f = frame_levels(edges, start_level, self.dt);
        self.push_samples(&f);
    }

    /// Adds a frame of samples (one per `dt` T-states; a shorter or longer frame is padded with
    /// its last sample or cut).
    pub fn push_samples(&mut self, frame: &[f32]) {
        if self.samples.len() >= self.per_frame * self.window {
            self.samples.drain(..self.per_frame);
        }
        let last = frame.last().copied().unwrap_or(0.0);
        self.samples.extend((0..self.per_frame).map(|i| frame.get(i).copied().unwrap_or(last)));
    }

    /// Frames held (at most the window's).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.per_frame
    }

    /// The window holds its N frames.
    pub fn full(&self) -> bool {
        self.frames() >= self.window
    }

    /// The samples, oldest first.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Samples a second.
    pub fn sample_rate(&self) -> f64 {
        CLOCK_HZ / f64::from(self.dt)
    }

    /// The last `frames` frames (as many as are held, if fewer).
    pub fn last(&self, frames: usize) -> View<'_> {
        let k = frames.min(self.frames());
        let samples = &self.samples[self.samples.len() - k * self.per_frame..];
        View { samples, hann: k.checked_sub(1).and_then(|i| self.hann.get(i)).map_or(&[], Vec::as_slice), rate: self.sample_rate() }
    }

    /// All the frames held.
    pub fn view(&self) -> View<'_> {
        self.last(self.window)
    }

    /// [`View::variance`] over all the frames held.
    pub fn variance(&self) -> f64 {
        self.view().variance()
    }

    /// [`View::power`] over all the frames held.
    pub fn power(&self, hz: f64) -> f64 {
        self.view().power(hz)
    }

    /// [`View::salience`] over all the frames held.
    pub fn salience(&self, hz: f64) -> f64 {
        self.view().salience(hz)
    }

    /// [`View::present`] over all the frames held.
    pub fn present(&self, hz: f64) -> bool {
        self.view().present(hz)
    }

    /// [`View::octave_below`] over all the frames held.
    pub fn octave_below(&self, hz: f64) -> bool {
        self.view().octave_below(hz)
    }
}

impl View<'_> {
    /// The view's length in seconds.
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / self.rate
    }

    fn mean(&self) -> f64 {
        let n = self.samples.len().max(1) as f64;
        self.samples.iter().map(|&x| f64::from(x)).sum::<f64>() / n
    }

    /// The variance of the samples.
    pub fn variance(&self) -> f64 {
        let m = self.mean();
        let n = self.samples.len().max(1) as f64;
        self.samples.iter().map(|&x| (f64::from(x) - m).powi(2)).sum::<f64>() / n
    }

    /// The power at `hz` (Goertzel, mean removed, Hann window): a sine of amplitude `a` gives
    /// about `a² / 4`. Zero for an empty view or a frequency at or past half the sample rate.
    pub fn power(&self, hz: f64) -> f64 {
        if self.samples.is_empty() || self.hann.len() != self.samples.len() || !(hz > 0.0 && hz < self.rate / 2.0) {
            return 0.0;
        }
        let m = self.mean();
        let w = 2.0 * PI * hz / self.rate;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        let mut wsum = 0.0;
        for (&x, &h) in self.samples.iter().zip(self.hann) {
            wsum += h;
            let s = (f64::from(x) - m) * h + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - c * s1 * s2) / (wsum * wsum)
    }

    /// How far the neighbours of `hz` are taken: one semitone, or the Hann main lobe's half
    /// width if that is wider.
    pub fn spread(&self, hz: f64) -> f64 {
        let lobe = 2.0 / self.seconds().max(1e-9);
        (hz * (SEMITONE - 1.0)).max(lobe)
    }

    fn sides(&self, hz: f64) -> f64 {
        let d = self.spread(hz);
        self.power(hz + d).max(if hz - d > 0.0 { self.power(hz - d) } else { 0.0 })
    }

    /// The power at `hz` over the larger of the powers one semitone (or the main lobe's half
    /// width) either side.
    pub fn salience(&self, hz: f64) -> f64 {
        self.power(hz) / self.sides(hz).max(1e-15)
    }

    /// `hz` sounds: its power is more than [`SALIENCE`] times its neighbours' and at least
    /// [`FLOOR`] of the variance.
    pub fn present(&self, hz: f64) -> bool {
        self.present_in(hz, self.variance())
    }

    /// [`View::present`] with the variance given (callers testing many frequencies over one
    /// view compute it once).
    pub fn present_in(&self, hz: f64, variance: f64) -> bool {
        if variance <= 0.0 {
            return false;
        }
        let p = self.power(hz);
        p >= FLOOR * variance && p > SALIENCE * self.sides(hz)
    }

    /// Half of `hz` sounds as well (a pitch predicted an octave too high).
    pub fn octave_below(&self, hz: f64) -> bool {
        self.present(hz / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: u32 = 128;

    /// Frame `frame`'s edges and start level for pulse channels `(period, width)` (T-states:
    /// high `width` every `period` from T-state 0 of frame 0), OR-ed.
    fn pulses(frame: u32, chans: &[(f64, f64)]) -> (Vec<zx_edge>, u8) {
        let t0 = f64::from(frame) * f64::from(ZX_FRAME_TSTATES);
        let high = |t: f64| chans.iter().any(|&(period, width)| (t % period) < width);
        let mut edges = Vec::new();
        let start = high(t0);
        let mut level = start;
        for t in 1..ZX_FRAME_TSTATES {
            let h = high(t0 + f64::from(t));
            if h != level {
                level = h;
                edges.push(zx_edge { tstate: t, level: if h { 2 } else { 0 }, pc: 0x8000 });
            }
        }
        (edges, if start { 2 } else { 0 })
    }

    fn signal(chans: &[(f64, f64)]) -> Signal {
        let mut s = Signal::new(DT, 4);
        for f in 0..4 {
            let (e, l) = pulses(f, chans);
            s.push_frame(&e, l);
        }
        assert!(s.full());
        s
    }

    fn hz(period_t: f64) -> f64 {
        CLOCK_HZ / period_t
    }

    #[test]
    fn frame_levels_integrate_each_sample() {
        let e = [zx_edge { tstate: 64, level: 2, pc: 0 }, zx_edge { tstate: 192, level: 0, pc: 0 }, zx_edge { tstate: 300, level: 3, pc: 0 }];
        let f = frame_levels(&e, 0, 128);
        assert_eq!(f.len(), (ZX_FRAME_TSTATES / 128) as usize);
        assert_eq!(&f[..3], &[0.5, 0.5, 84.0 / 128.0]);
        assert!(f[3..].iter().all(|&x| x == 1.0));
        // MIC alone (bit 0) is not the speaker
        let f = frame_levels(&[zx_edge { tstate: 0, level: 1, pc: 0 }], 2, 128);
        assert!(f.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn a_square_wave_is_present_at_its_pitch_only() {
        let period = 7954.0; // 440 Hz
        let s = signal(&[(period, period / 2.0)]);
        assert!(s.present(hz(period)), "salience {}", s.salience(hz(period)));
        assert!(!s.octave_below(hz(period)));
        // a square wave has no second harmonic
        assert!(!s.present(2.0 * hz(period)));
        assert!(!s.present(hz(period) * SEMITONE * SEMITONE));
        let p = s.power(hz(period));
        // fundamental of a 0/1 square: amplitude 2/pi, power 1/pi²
        assert!((p - 1.0 / (PI * PI)).abs() < 0.01, "{p}");
    }

    #[test]
    fn narrow_pulse_trains_are_present_and_their_octave_below_is_not() {
        for &period in &[174.0 * 40.0, 174.0 * 100.0, 174.0 * 200.0, 174.0 * 250.0] {
            let s = signal(&[(period, 40.0)]);
            let f = hz(period);
            assert!(s.present(f), "{period}: salience {}", s.salience(f));
            assert!(!s.octave_below(f), "{period}");
            // its harmonics are there too: an octave too high a prediction passes, but then its
            // own octave below does as well
            assert!(s.present(2.0 * f) && s.octave_below(2.0 * f), "{period}");
        }
    }

    #[test]
    fn two_mixed_pulse_channels_are_both_present() {
        let (a, b) = (174.0 * 60.0, 174.0 * 89.0);
        let s = signal(&[(a, 120.0), (b, 120.0)]);
        assert!(s.present(hz(a)), "a {}", s.salience(hz(a)));
        assert!(s.present(hz(b)), "b {}", s.salience(hz(b)));
        for wrong in [174.0 * 70.0, 174.0 * 75.0, 174.0 * 110.0, 174.0 * 53.0] {
            assert!(!s.present(hz(wrong)), "{wrong}: {}", s.salience(hz(wrong)));
        }
    }

    /// Two low notes a whole tone apart: over 4 frames each lies within the other's main lobe,
    /// over 8 both stand out.
    #[test]
    fn close_low_notes_need_a_longer_window() {
        let (a, b) = (174.0 * 107.0, 174.0 * 95.0); // 188 and 212 Hz
        let mut s = Signal::new(DT, 8);
        for f in 0..8 {
            let (e, l) = pulses(f, &[(a, 120.0), (b, 120.0)]);
            s.push_frame(&e, l);
        }
        let long = s.last(8);
        assert!(long.present(hz(a)) && long.present(hz(b)), "{} {}", long.salience(hz(a)), long.salience(hz(b)));
        let short = s.last(4);
        assert!(!short.present(hz(a)) || !short.present(hz(b)));
    }

    #[test]
    fn silence_and_noise_have_no_pitch() {
        let s = signal(&[]);
        assert_eq!(s.variance(), 0.0);
        assert!(!s.present(440.0));
        // white noise: random levels
        let mut x = 0x1234_5678u32;
        let mut s = Signal::new(DT, 4);
        for _ in 0..4 {
            let f: Vec<f32> = (0..ZX_FRAME_TSTATES / DT)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x & 1) as f32
                })
                .collect();
            s.push_samples(&f);
        }
        let hits = (0..40).filter(|i| s.present(100.0 * 1.1f64.powi(*i))).count();
        assert!(hits <= 2, "{hits}");
    }

    #[test]
    fn the_window_keeps_the_last_frames() {
        let mut s = Signal::new(DT, 2);
        s.push_samples(&[1.0]);
        assert!(!s.full());
        s.push_samples(&[0.0]);
        s.push_samples(&[0.5]);
        assert!(s.full());
        assert_eq!(s.frames(), 2);
        assert_eq!(s.samples().len(), 2 * (ZX_FRAME_TSTATES / DT) as usize);
        assert_eq!(s.samples()[0], 0.0);
        assert_eq!(*s.samples().last().unwrap(), 0.5);
        let v = s.last(1);
        assert!((v.seconds() - f64::from(ZX_FRAME_TSTATES) / CLOCK_HZ).abs() < 1e-9);
        assert_eq!(v.variance(), 0.0);
    }
}
