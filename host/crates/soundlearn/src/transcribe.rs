// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The transcriber: the notes a stretch of beeper music plays, up to three voices, read from the
//! speaker's one bit alone. For engines nothing else reads: those that mix several channels into
//! the bit by interleaving narrow pulses, by pulse-width shares of phase accumulators, by summed
//! counters or by toggles. The notes are recorded at pack time as a score the FM synth plays in
//! step with the game, so each note's period is measured as precisely as the signal allows and
//! snapped to nothing: the pitch the speaker plays, the time the interrupt takes from the music
//! loop included.
//!
//! # The signal
//!
//! Each frame's edges become the speaker's level sampled every 32 T-states, each sample the
//! share of its T-states the speaker is high ([`frame_levels`]), then low-pass filtered (a
//! Blackman-windowed sinc) and taken every [`DT`] T-states (54,688 Hz). A narrow pulse's
//! harmonics reach far past any sample rate; sampled by the box filter alone they fold back into
//! the band as strong lines at no harmonic of anything.
//!
//! # The spectrum
//!
//! Every [`Params::hop_frames`] frames (half a frame) a Hann window of [`Params::window_frames`]
//! frames centred there (160 ms) is transformed (a radix-2 FFT, zero-padded to at least twice
//! the window, two real windows packed into one complex transform), the magnitudes scaled so a
//! sine of amplitude `a` peaks at `a`. A window that long separates the harmonics of a 30 Hz note
//! and tells a semitone apart from 210 Hz up; lower notes are told apart by their harmonics. The
//! spectrum's *peaks* (local maxima at least [`Params::peak_share`] of the strongest, up to
//! [`Params::f_top`]) are kept with their frequency and amplitude interpolated by a parabola
//! through the log magnitudes of the three bins around them. The magnitude a fifth of the band's
//! bins lie under is the window's noise floor.
//!
//! A peak may be a fundamental only when it is shaped as the window's main lobe: no narrower
//! (the ripples of a step in the level, where a note starts or stops), no wider (the lobes of a
//! short rest's dip in the level), and standing out of the magnitudes 2-3 bins past it on one
//! side at least (a longer rest's field of ripples).
//!
//! # Pitches in a window
//!
//! Every engine here makes each channel a pulse train or a square wave, so a note's harmonics
//! lie at whole multiples of its pitch. A candidate is a main-lobe peak between
//! [`Params::f_min`] and [`Params::f_max`]. Its harmonics are found ever closer around a pitch
//! refined by those found so far (a least-squares fit weighted by amplitude): the first twelve
//! within [`Params::tolerance`], the first forty within four times [`Params::fine_tolerance`],
//! then all within it (or [`Params::tolerance_hz`]). Its *salience* is
//!
//! `S(f0) = sum over h of h^-p x min(A_h, cap x A_1)`, `p` = [`Params::weight_power`].
//!
//! - Summing every harmonic up to a fixed frequency, not a fixed count, makes the true pitch beat
//!   its octave above (which collects only every other harmonic): the octave errors pulse trains
//!   invite, their harmonics all as strong as the fundamental.
//! - Each harmonic counts at most [`Params::cap`] times the fundamental, which keeps out a weak
//!   peak (an intermodulation product of the 1-bit mix, a common divisor of two voices' pitches)
//!   whose harmonics are other voices' notes.
//! - About 40% of its harmonics 2-8 in the band (one at least) must be found: a pulse train has
//!   all, a square the odd ones, noise and the images of an engine's pass rate few.
//! - A pulse train whose fundamental is buried (under the ripples of another voice's rest, in an
//!   accumulator engine's jitter) shows as its second or third harmonic: the pitch is taken half
//!   or a third as high when three in five of the lower pitch's harmonics that are not the
//!   candidate's (four at least in the band) are there too.
//!
//! The strongest candidate is taken, its harmonics are subtracted from the peaks (each by its
//! expected amplitude: at most the larger of its neighbouring harmonics', so a peak another voice
//! shares keeps that voice's part), and the next is sought in what is left, up to
//! [`Params::voices`]. A pitch is taken only while its salience is at least
//! [`Params::min_salience`] of the first pitch's and its fundamental at least
//! [`Params::min_amp_share`] of the first's, [`Params::floor_ratio`] times the noise floor and
//! [`Params::min_amp`]. A pitch near (half a semitone) one already taken is not sought; one near
//! a whole multiple of one taken needs a peak standing out on both sides (not one of the split
//! harmonics of two channels near unison) and twice the shares. The window's *tonality* is the
//! share of its peaks' energy the pitches taken hold: noise spreads it over many peaks.
//!
//! # Notes
//!
//! The windows' pitches are joined into tracks: each pitch continues the track whose pitch (the
//! median of its last five) is nearest, within [`Params::track_cents`]; a track missing for more
//! than [`Params::gap_hops`] windows ends. A track splits where its fundamental's amplitude dips
//! below [`Params::dip`] of the peaks on both sides (a note played again). A piece's edges are
//! where its amplitude crosses half its median (interpolated between windows: a window centred
//! on an onset holds the note for half its weight). A piece shorter than
//! [`Params::min_note_frames`], or whose windows' median tonality is under
//! [`Params::min_tonal`], is dropped. A note's pitch is the amplitude-weighted mean (in cents)
//! of its windows' pitches between its edges, those more than 25 cents from their median left
//! out, the first and last too when it has four or more; its period is the clock over that. Its
//! level is its median loudness (the root mean square of its harmonics) over the loudest
//! note's, 1-255.
//!
//! Voices: in start order each note takes a free voice (one whose note has ended, or ends at
//! most a frame later and is cut there): the first whose note ends where it starts (a line
//! goes on), else the one whose note ended last (a line paused by a rest); with none free, the sounding note of least
//! fundamental amplitude is cut where the new one starts if the new one is stronger, else the
//! new one is dropped. Notes of one voice never overlap.
//!
//! # Times
//!
//! All times are T-states since `T0`, the T-state (in the stretch's first frame) of the
//! stretch's first edge; a frame is `ZX_FRAME_TSTATES` long.
//!
//! # Speed
//!
//! The windows are transformed on all cores: 9,000 frames (3 minutes) take about 2 s in a
//! release build (two to three seconds on the real tunes).

use crate::spectrum::{frame_levels, CLOCK_HZ};
use std::f64::consts::PI;
use zx_sys::{zx_edge, ZX_FRAME_TSTATES};

/// T-states between the signal's samples.
pub const DT: u32 = 64;
/// A main lobe's least magnitude half a resolution bin from its peak, as a share of the peak
/// (a steady tone's is at least 0.67 wherever it falls between bins) ...
const LOBE: f64 = 0.5;
/// ... and the most of the smaller of the magnitudes 1.5 bins either side (a steady tone's is
/// at most 0.3) ...
const LOBE_SIDE: f64 = 0.5;
/// ... and of the smaller of the largest magnitudes 2-3 bins either side (a tone's side lobes
/// are 0.03; a gap in the level, a rest, makes a field of ripples about that far apart).
const RING: f64 = 0.4;
/// Samples a frame.
const PER_FRAME: usize = (ZX_FRAME_TSTATES / DT) as usize;

/// One note of the transcription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    /// 0-2; one voice's notes do not overlap.
    pub voice: u8,
    /// T-states since `T0`.
    pub start_t: u64,
    /// T-states since `T0`, after the last one sounding.
    pub end_t: u64,
    /// The full period, T-states.
    pub period_t: u32,
    /// Loudness, 1-255: the root mean square of the note's harmonics over the loudest note's.
    pub level: u8,
}

impl Note {
    /// The note's frequency, Hz.
    pub fn hz(&self) -> f64 {
        CLOCK_HZ / f64::from(self.period_t.max(1))
    }
}

/// A pitch heard in one window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pitch {
    /// Hz.
    pub hz: f64,
    /// The fundamental's amplitude (a sine of amplitude `a` in the 0-1 level: `a`).
    pub amp: f64,
    /// Its salience (see the module docs).
    pub salience: f64,
    /// Its loudness: the root mean square of its harmonics found (each at most
    /// [`Params::cap`] times the fundamental).
    pub loudness: f64,
    /// The share of its window's peak energy the window's pitches hold.
    pub tonality: f64,
}

/// The transcriber's settings (see the module docs for each one's part).
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Frames in a window.
    pub window_frames: f64,
    /// Frames between windows.
    pub hop_frames: f64,
    /// The lowest and highest pitch a note may have, Hz.
    pub f_min: f64,
    pub f_max: f64,
    /// Harmonics are summed up to this frequency, Hz.
    pub f_top: f64,
    /// At most this many harmonics are summed.
    pub max_harmonics: usize,
    /// A harmonic's peak is within this share of `h f0` (the first twelve, to refine the
    /// pitch) ...
    pub tolerance: f64,
    /// ... then four times this share (up to the 40th) and this share (all) around the pitch
    /// refined by those found ...
    pub fine_tolerance: f64,
    /// ... or within this many Hz.
    pub tolerance_hz: f64,
    /// The weight of harmonic `h` is `h^-weight_power`.
    pub weight_power: f64,
    /// A harmonic counts at most this many times the fundamental.
    pub cap: f64,
    /// Peaks under this share of the window's strongest are not kept.
    pub peak_share: f64,
    /// Pitches a window at most.
    pub voices: usize,
    /// A pitch's least salience, as a share of the window's first pitch's.
    pub min_salience: f64,
    /// A pitch's least fundamental amplitude, as a share of the window's first pitch's.
    pub min_amp_share: f64,
    /// A pitch's fundamental is at least this many times the noise floor ...
    pub floor_ratio: f64,
    /// ... and at least this amplitude.
    pub min_amp: f64,
    /// A note's windows' pitches hold at least this share of their peaks' energy (the median
    /// over its windows).
    pub min_tonal: f64,
    /// A pitch continues a track within this many cents of its pitch.
    pub track_cents: f64,
    /// Windows a track may miss before it ends.
    pub gap_hops: usize,
    /// A track splits where its strength falls below this share of the peaks on both sides.
    pub dip: f64,
    /// The shortest note, frames.
    pub min_note_frames: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            window_frames: 8.0,
            hop_frames: 0.5,
            f_min: 30.0,
            f_max: 4200.0,
            f_top: 12000.0,
            max_harmonics: 400,
            tolerance: 0.012,
            fine_tolerance: 0.002,
            tolerance_hz: 3.0,
            weight_power: 0.75,
            cap: 2.0,
            peak_share: 0.02,
            voices: 3,
            min_salience: 0.05,
            min_amp_share: 0.02,
            floor_ratio: 3.0,
            min_amp: 2e-4,
            min_tonal: 0.35,
            track_cents: 60.0,
            gap_hops: 4,
            dip: 0.55,
            min_note_frames: 2.0,
        }
    }
}

/// Collects a stretch's frames and transcribes them.
#[derive(Clone, Debug)]
pub struct Transcriber {
    params: Params,
    t0: u32,
    frames: usize,
    signal: Decimator,
}

/// Transcribes a stretch: `frames` are each frame's edges and the level it starts at, from the
/// stretch's first frame; `t0` is the T-state of the first edge in that frame.
pub fn transcribe(frames: &[(Vec<zx_edge>, u8)], t0: u32) -> Vec<Note> {
    let mut t = Transcriber::new(t0);
    for (e, l) in frames {
        t.push_frame(e, *l);
    }
    t.notes()
}

impl Transcriber {
    /// A transcriber with the default settings; `t0` is the T-state of the stretch's first edge
    /// in its first frame.
    pub fn new(t0: u32) -> Self {
        Self::with_params(t0, Params::default())
    }

    /// A transcriber with the settings given.
    pub fn with_params(t0: u32, params: Params) -> Self {
        Transcriber { params, t0: t0.min(ZX_FRAME_TSTATES), frames: 0, signal: Decimator::new() }
    }

    /// Adds the stretch's next frame: its edges and the level it starts at
    /// ([`Machine::edges_start_level`](zx_sys::Machine::edges_start_level)).
    pub fn push_frame(&mut self, edges: &[zx_edge], start_level: u8) {
        self.signal.push(&frame_levels(edges, start_level, FINE));
        self.frames += 1;
    }

    /// Frames added.
    pub fn frames(&self) -> usize {
        self.frames
    }

    fn hop_samples(&self) -> f64 {
        (self.params.hop_frames * PER_FRAME as f64).max(1.0)
    }

    /// The number of windows over the frames added.
    pub fn hops(&self) -> usize {
        ((self.frames * PER_FRAME) as f64 / self.hop_samples()).ceil() as usize
    }

    /// The centre of window `j`, T-states since `T0` (negative before it).
    pub fn hop_t(&self, j: usize) -> i64 {
        (j as f64 * self.hop_samples() * f64::from(DT)).round() as i64 - i64::from(self.t0)
    }

    /// The pitches heard in each window, strongest first (before notes are made of them).
    pub fn pitches(&self) -> Vec<Vec<Pitch>> {
        let peaks = self.peaks();
        peaks.into_iter().map(|(mut p, floor)| pick(&mut p, floor, &self.params)).collect()
    }

    /// The notes, in start order.
    pub fn notes(&self) -> Vec<Note> {
        let pitches = self.pitches();
        let p = &self.params;
        let hop_t = self.hop_samples() * f64::from(DT);
        let min_hops = p.min_note_frames * f64::from(ZX_FRAME_TSTATES) / hop_t;
        let mut raw: Vec<RawNote> = Vec::new();
        for tr in track(&pitches, p) {
            for piece in split(&tr, p.dip) {
                let n = summarise(&piece);
                if n.end - n.start >= min_hops && n.tonality >= p.min_tonal {
                    raw.push(n);
                }
            }
        }
        let loudest = raw.iter().map(|n| n.loudness).fold(0.0, f64::max);
        let to_t = |x: f64| -> u64 { (x * hop_t - f64::from(self.t0)).round().max(0.0) as u64 };
        let mut notes: Vec<(Note, f64)> = raw
            .iter()
            .map(|n| {
                let level = if loudest > 0.0 { (255.0 * n.loudness / loudest).round().clamp(1.0, 255.0) as u8 } else { 1 };
                let period = (CLOCK_HZ / n.hz).round().clamp(1.0, f64::from(u32::MAX)) as u32;
                (Note { voice: 0, start_t: to_t(n.start), end_t: to_t(n.end), period_t: period, level }, n.amp)
            })
            .filter(|(n, _)| n.end_t > n.start_t)
            .collect();
        notes.sort_by(|a, b| a.0.start_t.cmp(&b.0.start_t).then(a.0.period_t.cmp(&b.0.period_t)));
        assign_voices(notes, u64::from(ZX_FRAME_TSTATES), p.voices.min(3))
    }

    /// Each window's peaks and noise floor.
    fn peaks(&self) -> Vec<(Vec<Peak>, f64)> {
        let n_hops = self.hops();
        if n_hops == 0 {
            return Vec::new();
        }
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
        let chunk = n_hops.div_ceil(threads).max(64);
        let sig = self.signal.samples();
        let sig = sig.as_slice();
        let mut out: Vec<(Vec<Peak>, f64)> = Vec::with_capacity(n_hops);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..n_hops).step_by(chunk).map(|a| s.spawn(move || self.peaks_range(sig, a, (a + chunk).min(n_hops)))).collect();
            for h in handles {
                out.extend(h.join().expect("spectrum thread"));
            }
        });
        out
    }

    fn peaks_range(&self, sig: &[f32], from: usize, to: usize) -> Vec<(Vec<Peak>, f64)> {
        let p = &self.params;
        let len = ((p.window_frames * PER_FRAME as f64).round() as usize).max(16);
        let n = (2 * len).next_power_of_two();
        let fft = Fft::new(n);
        let win = hann(len);
        let wsum: f64 = win.iter().sum();
        let rate = CLOCK_HZ / f64::from(DT);
        let bin_hz = rate / n as f64;
        let zp = n as f64 / len as f64;
        let lo = ((p.f_min * 0.9 / bin_hz).floor() as usize).max(2);
        let hi = ((p.f_top * 1.02 / bin_hz).ceil() as usize).min(n / 2 - 2);
        let mut re = vec![0f64; n];
        let mut im = vec![0f64; n];
        let mut mag_a = vec![0f64; n / 2];
        let mut mag_b = vec![0f64; n / 2];
        let mut scratch = Vec::with_capacity(hi - lo + 1);
        let mut out = Vec::with_capacity(to - from);
        let mut j = from;
        while j < to {
            let pair = j + 1 < to;
            self.fill(sig, j, &win, &mut re[..len]);
            if pair {
                self.fill(sig, j + 1, &win, &mut im[..len]);
            } else {
                im[..len].fill(0.0);
            }
            re[len..].fill(0.0);
            im[len..].fill(0.0);
            fft.run(&mut re, &mut im);
            let scale = 1.0 / wsum; // |X| of a sine of amplitude a is a x wsum / 2: x 2 / 2
            for k in 1..n / 2 {
                let (xr, xi, yr, yi) = (re[k], im[k], re[n - k], im[n - k]);
                mag_a[k] = ((xr + yr).powi(2) + (xi - yi).powi(2)).sqrt() * scale;
                mag_b[k] = ((xi + yi).powi(2) + (xr - yr).powi(2)).sqrt() * scale;
            }
            out.push(find_peaks(&mag_a, lo, hi, bin_hz, p.peak_share, zp, &mut scratch));
            if pair {
                out.push(find_peaks(&mag_b, lo, hi, bin_hz, p.peak_share, zp, &mut scratch));
            }
            j += if pair { 2 } else { 1 };
        }
        out
    }

    /// Window `j`'s samples, mean removed and windowed, into `buf`.
    fn fill(&self, sig: &[f32], j: usize, win: &[f64], buf: &mut [f64]) {
        let len = win.len();
        let centre = (j as f64 * self.hop_samples()).round() as i64;
        let start = centre - (len / 2) as i64;
        let n = sig.len() as i64;
        let first = f64::from(sig[0]);
        let last = f64::from(*sig.last().expect("samples"));
        let at = |i: i64| if i < 0 { first } else if i >= n { last } else { f64::from(sig[i as usize]) };
        let mut mean = 0.0;
        for (k, b) in buf.iter_mut().enumerate() {
            *b = at(start + k as i64);
            mean += *b;
        }
        mean /= len as f64;
        for (b, w) in buf.iter_mut().zip(win) {
            *b = (*b - mean) * w;
        }
    }
}

/// The level sampled every [`FINE`] T-states, low-pass filtered and taken every [`DT`]: a
/// narrow pulse's harmonics reach far above the sample rate, and sampled by the box filter
/// alone they fold back into the band at a third of their strength or more.
#[derive(Clone, Debug)]
struct Decimator {
    kernel: Vec<f32>,
    /// The fine samples not yet used up, from fine sample `base` on.
    fine: Vec<f32>,
    base: usize,
    /// Fine samples pushed.
    total: usize,
    out: Vec<f32>,
}

/// A harmonic counts as found for [`COMPLETE`] when it is at least this share of the
/// fundamental ...
const SUPPORT: f64 = 0.1;
/// ... and about 40% of a pitch's harmonics 2 to this one (as many as are in the band), one
/// at least, are found.
const COMPLETE: usize = 8;
/// A candidate's harmonics looked at for a pitch half or a third of it, per divisor.
const SUB_HARMONICS: usize = 8;
/// Within this share of a pitch taken (or a multiple of it) is near it: half a semitone.
const NEAR: f64 = 0.03;

/// T-states between the fine samples.
const FINE: u32 = 32;
/// Fine samples a sample.
const DECIM: usize = (DT / FINE) as usize;
/// The low-pass filter's taps (a Blackman-windowed sinc).
const TAPS: usize = 48;
/// Its cut-off, Hz: it passes [`Params::f_top`] and stops what would fold back below it.
const CUTOFF_HZ: f64 = 20_000.0;

impl Decimator {
    fn new() -> Self {
        let fs = CLOCK_HZ / f64::from(FINE);
        let c = (TAPS as f64 - 1.0) / 2.0;
        let mut kernel: Vec<f64> = (0..TAPS)
            .map(|k| {
                let x = k as f64 - c;
                let sinc = if x == 0.0 { 1.0 } else { (2.0 * PI * CUTOFF_HZ / fs * x).sin() / (PI * x) / (2.0 * CUTOFF_HZ / fs) };
                let w = 2.0 * PI * k as f64 / (TAPS as f64 - 1.0);
                sinc * (0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos())
            })
            .collect();
        let sum: f64 = kernel.iter().sum();
        kernel.iter_mut().for_each(|k| *k /= sum);
        Decimator { kernel: kernel.into_iter().map(|k| k as f32).collect(), fine: Vec::new(), base: 0, total: 0, out: Vec::new() }
    }

    /// The first fine sample output `i` uses: its taps are centred on the middle of its
    /// [`DT`] T-states.
    fn first_tap(i: usize) -> i64 {
        (i * DECIM + DECIM / 2) as i64 - (TAPS / 2) as i64
    }

    fn output(&self, i: usize, at: impl Fn(i64) -> f32) -> f32 {
        let a = Self::first_tap(i);
        self.kernel.iter().enumerate().map(|(k, &h)| h * at(a + k as i64)).sum()
    }

    fn push(&mut self, frame: &[f32]) {
        let first = self.fine.first().copied().or(frame.first().copied()).unwrap_or(0.0);
        let first = if self.total == 0 { frame.first().copied().unwrap_or(0.0) } else { first };
        self.fine.extend_from_slice(frame);
        self.total += frame.len();
        loop {
            let i = self.out.len();
            let a = Self::first_tap(i);
            if a + TAPS as i64 > self.total as i64 {
                break;
            }
            let (fine, base) = (&self.fine, self.base as i64);
            let head = if self.base == 0 { first } else { 0.0 };
            let v = self.output(i, |t| if t < base { head } else { fine[(t - base) as usize] });
            self.out.push(v);
        }
        // drop what the next output no longer needs
        let keep_from = Self::first_tap(self.out.len()).max(0) as usize;
        if keep_from > self.base {
            self.fine.drain(..keep_from - self.base);
            self.base = keep_from;
        }
    }

    /// All the samples: those still waiting for fine samples to come are made with the last
    /// level held.
    fn samples(&self) -> Vec<f32> {
        let mut out = self.out.clone();
        let n = self.total / DECIM;
        let last = self.fine.last().copied().unwrap_or(0.0);
        let base = self.base as i64;
        while out.len() < n {
            let i = out.len();
            out.push(self.output(i, |t| {
                let k = t - base;
                if k < 0 {
                    self.fine.first().copied().unwrap_or(last)
                } else {
                    self.fine.get(k as usize).copied().unwrap_or(last)
                }
            }));
        }
        out
    }
}

/// A spectral peak: frequency, amplitude, and what is left of it once pitches are taken.
#[derive(Clone, Copy, Debug)]
struct Peak {
    f: f64,
    a: f64,
    r: f64,
    /// Shaped as the window's main lobe: as wide as a steady tone's peak, no narrower (the
    /// ripples of a step in the level, where a note starts or stops) and no wider (the lobes of
    /// a short dip in the level, a rest between notes), and standing out of the magnitudes
    /// just past it on one side at least (a longer rest's field of ripples). Only such a peak
    /// may be a fundamental.
    lobe: bool,
    /// A main lobe standing out of the magnitudes past it on both sides.
    isolated: bool,
}

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / n as f64).cos()).collect()
}

/// The peaks of `mag` between bins `lo` and `hi`, at least `share` of the strongest, and the
/// median magnitude there.
/// `zp` is a resolution bin (the transform's length over the window's) in bins.
fn find_peaks(mag: &[f64], lo: usize, hi: usize, bin_hz: f64, share: f64, zp: f64, scratch: &mut Vec<f64>) -> (Vec<Peak>, f64) {
    let near = (0.5 * zp).round().max(1.0) as usize;
    let far = (1.5 * zp).round().max(2.0) as usize;
    let (ring_a, ring_b) = ((2.0 * zp).round() as usize, (3.0 * zp).round() as usize);
    scratch.clear();
    scratch.extend_from_slice(&mag[lo..=hi]);
    let mid = scratch.len() / 5;
    let floor = *scratch.select_nth_unstable_by(mid, f64::total_cmp).1;
    let top = mag[lo..=hi].iter().copied().fold(0.0, f64::max);
    let mut peaks = Vec::new();
    if top <= 0.0 {
        return (peaks, floor);
    }
    let thr = top * share;
    for k in lo.max(1)..=hi {
        let (a, b, c) = (mag[k - 1], mag[k], mag[k + 1]);
        if b < thr || b <= a || b < c {
            continue;
        }
        let (la, lb, lc) = (a.max(1e-12).ln(), b.ln(), c.max(1e-12).ln());
        let den = la - 2.0 * lb + lc;
        let d = if den < 0.0 { (0.5 * (la - lc) / den).clamp(-0.5, 0.5) } else { 0.0 };
        let amp = (lb - 0.25 * (la - lc) * d).exp();
        let (lobe, isolated) = if k >= ring_b && k + ring_b < mag.len() && mag[k - near].min(mag[k + near]) >= LOBE * b && mag[k - far].min(mag[k + far]) <= LOBE_SIDE * b {
            let below = mag[k - ring_b..=k - ring_a].iter().copied().fold(0.0, f64::max);
            let above = mag[k + ring_a..=k + ring_b].iter().copied().fold(0.0, f64::max);
            (below.min(above) <= RING * b, below.max(above) <= RING * b)
        } else {
            (false, false)
        };
        peaks.push(Peak { f: (k as f64 + d) * bin_hz, a: amp, r: amp, lobe, isolated });
    }
    (peaks, floor)
}

/// The peak nearest `hz` (peaks in frequency order).
fn nearest(peaks: &[Peak], hz: f64) -> Option<usize> {
    let i = peaks.partition_point(|p| p.f < hz);
    match (i.checked_sub(1), (i < peaks.len()).then_some(i)) {
        (Some(a), Some(b)) => Some(if hz - peaks[a].f <= peaks[b].f - hz { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// A candidate's harmonics: (h, peak index) for each found.
/// A candidate's harmonics up to the `most`th: (h, peak index) for each found within `share`
/// of `h f0` (or [`Params::tolerance_hz`]).
fn harmonics(peaks: &[Peak], f0: f64, share: f64, most: usize, p: &Params, out: &mut Vec<(usize, usize)>) {
    out.clear();
    let n = ((p.f_top / f0).floor() as usize).clamp(1, most);
    for h in 1..=n {
        let target = h as f64 * f0;
        if let Some(i) = nearest(peaks, target) {
            if (peaks[i].f - target).abs() <= (share * target).max(p.tolerance_hz) && out.last().is_none_or(|&(_, j)| j != i) {
                out.push((h, i));
            }
        }
    }
}

fn salience(peaks: &[Peak], found: &[(usize, usize)], a1: f64, p: &Params) -> f64 {
    found.iter().map(|&(h, i)| (h as f64).powf(-p.weight_power) * peaks[i].r.min(p.cap * a1)).sum()
}

/// Up to `p.voices` pitches from a window's peaks (see the module docs); the peaks' residuals
/// are used up.
fn pick(peaks: &mut [Peak], floor: f64, p: &Params) -> Vec<Pitch> {
    let mut got: Vec<Pitch> = Vec::new();
    let mut found = Vec::new();
    let mut best_found = Vec::new();
    let mut sub = Vec::new();
    let mut explained = 0.0;
    let top = peaks.iter().map(|q| q.a).fold(0.0, f64::max);
    let least = p.min_amp.max(p.floor_ratio * floor);
    for _ in 0..p.voices {
        let mut best: Option<(f64, f64, f64)> = None; // salience, f0, a1
        for c in 0..peaks.len() {
            let pc = peaks[c];
            if !pc.lobe || pc.f < p.f_min || pc.f > p.f_max || pc.r < least || pc.r < p.peak_share * top {
                continue;
            }
            // a side lobe of a pitch taken is no second voice, and a peak at a multiple of one
            // must stand alone (not one of a pair of near unison channels' split harmonics)
            if got.iter().any(|g| (pc.f / g.hz - 1.0).abs() < NEAR) || (!pc.isolated && multiple(&got, pc.f)) {
                continue;
            }
            // find the harmonics ever closer around a pitch refined by those found so far
            let mut f0 = pc.f;
            let mut lost = false;
            for (share, most) in [(p.tolerance, 12), (4.0 * p.fine_tolerance, 40), (p.fine_tolerance, p.max_harmonics)] {
                harmonics(peaks, f0, share, most, p, &mut found);
                if found.first().is_none_or(|&(h, i)| h != 1 || i != c) {
                    lost = true;
                    break;
                }
                let (mut num, mut den) = (0.0, 0.0);
                for &(h, i) in &found {
                    let w = peaks[i].r.min(p.cap * pc.r);
                    num += w * h as f64 * peaks[i].f;
                    den += w * (h * h) as f64;
                }
                if den > 0.0 {
                    f0 = num / den;
                }
            }
            if lost {
                continue;
            }
            // a pulse train has all its harmonics and a square the odd ones: of the first few
            // past the fundamental in the band, about 40% are found, one at least (noise and
            // the images of an engine's pass rate have few)
            let k = ((p.f_top / f0) as usize).min(COMPLETE);
            let need = ((0.42 * (k as f64 - 1.0)).round() as usize).max(1);
            if found.iter().filter(|&&(h, i)| (2..=k).contains(&h) && peaks[i].a >= SUPPORT * pc.a).count() < need {
                continue;
            }
            // a pulse train whose fundamental is buried (under the ripples of another voice's
            // rest) shows as its second or third harmonic: the pitch is that much lower when
            // the harmonics of the lower pitch that are not the candidate's are there too (at
            // least four of them in the band)
            for d in [2usize, 3] {
                let alt = f0 / d as f64;
                if alt < p.f_min {
                    continue;
                }
                let most = ((p.f_top / alt) as usize).min(SUB_HARMONICS * d);
                harmonics(peaks, alt, p.fine_tolerance, most, p, &mut sub);
                let own = sub.iter().filter(|&&(h, i)| h % d != 0 && peaks[i].r >= SUPPORT * pc.r).count();
                let expect = most - most / d;
                if expect >= 4 && own * 5 >= expect * 3 {
                    f0 = alt;
                    harmonics(peaks, f0, p.fine_tolerance, p.max_harmonics, p, &mut found);
                    break;
                }
            }
            if got.iter().any(|g| (f0 / g.hz - 1.0).abs() < NEAR) {
                continue;
            }
            let s = salience(peaks, &found, pc.r, p);
            if best.is_none_or(|b| s > b.0) {
                best = Some((s, f0, pc.r));
                std::mem::swap(&mut best_found, &mut found);
            }
        }
        let Some((s, f0, a1)) = best else { break };
        if let Some(first) = got.first() {
            let need = if multiple(&got, f0) { 2.0 } else { 1.0 };
            if s < need * p.min_salience * first.salience || a1 < need * p.min_amp_share * first.amp {
                break;
            }
        }
        let loudness = (best_found.iter().map(|&(_, i)| peaks[i].r.min(p.cap * a1).powi(2)).sum::<f64>() / 2.0).sqrt();
        // take its harmonics out: each by at most the larger of its neighbours' amplitudes
        let amps: Vec<(usize, usize, f64)> = best_found.iter().map(|&(h, i)| (h, i, peaks[i].r)).collect();
        for (k, &(h, i, r)) in amps.iter().enumerate() {
            let mut expect = r.min(p.cap * a1);
            if h > 1 {
                let nb: Vec<f64> = [k.checked_sub(1).map(|x| amps[x]), amps.get(k + 1).copied()].into_iter().flatten().filter(|&(hh, _, _)| hh + 1 == h || hh == h + 1).map(|x| x.2).collect();
                if let Some(m) = nb.iter().copied().reduce(f64::max) {
                    expect = expect.min(m);
                }
            }
            let left = (peaks[i].r - expect).max(0.0);
            explained += peaks[i].r * peaks[i].r - left * left;
            peaks[i].r = left;
        }
        got.push(Pitch { hz: f0, amp: a1, salience: s, loudness, tonality: 0.0 });
    }
    // the share of the peaks' energy the pitches hold: noise spreads it over many
    let total: f64 = peaks.iter().map(|q| q.a * q.a).sum();
    let tonality = explained / total.max(f64::MIN_POSITIVE);
    for g in &mut got {
        g.tonality = tonality;
    }
    got
}

/// `hz` is near a whole multiple (2 or more) of a pitch taken.
fn multiple(got: &[Pitch], hz: f64) -> bool {
    got.iter().any(|g| {
        let k = (hz / g.hz).round();
        k >= 2.0 && (hz / (k * g.hz) - 1.0).abs() < NEAR
    })
}

/// One window's pitch in a track.
#[derive(Clone, Copy, Debug)]
struct Point {
    /// The window.
    j: usize,
    hz: f64,
    amp: f64,
    loudness: f64,
    tonality: f64,
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

/// The middle value (the upper one of an even count); 0 for none.
fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The windows' pitches joined into tracks (see the module docs).
fn track(pitches: &[Vec<Pitch>], p: &Params) -> Vec<Vec<Point>> {
    let mut done: Vec<Vec<Point>> = Vec::new();
    let mut open: Vec<Vec<Point>> = Vec::new();
    let reference = |t: &[Point]| median(t.iter().rev().take(5).map(|x| x.hz).collect());
    for (j, ps) in pitches.iter().enumerate() {
        let point = |q: &Pitch| Point { j, hz: q.hz, amp: q.amp, loudness: q.loudness, tonality: q.tonality };
        let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
        for (ti, t) in open.iter().enumerate() {
            let r = reference(t);
            for (pi, q) in ps.iter().enumerate() {
                let d = cents(q.hz, r).abs();
                if d <= p.track_cents {
                    pairs.push((d, ti, pi));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut t_used = vec![false; open.len()];
        let mut p_used = vec![false; ps.len()];
        for (_, ti, pi) in pairs {
            if !t_used[ti] && !p_used[pi] {
                t_used[ti] = true;
                p_used[pi] = true;
                open[ti].push(point(&ps[pi]));
            }
        }
        for (pi, q) in ps.iter().enumerate() {
            if !p_used[pi] {
                open.push(vec![point(q)]);
            }
        }
        let (keep, end): (Vec<_>, Vec<_>) = open.into_iter().partition(|t| j - t.last().expect("track").j <= p.gap_hops);
        open = keep;
        done.extend(end);
    }
    done.extend(open);
    done
}

/// A track split where its strength dips below `dip` of the peaks either side.
fn split(t: &[Point], dip: f64) -> Vec<Vec<Point>> {
    let mut out = Vec::new();
    let mut from = 0;
    let n = t.len();
    let mut k = 1;
    while k + 1 < n {
        let a = t[k].amp;
        if a <= t[k - 1].amp && a <= t[k + 1].amp {
            let before = t[from..k].iter().map(|x| x.amp).fold(0.0, f64::max);
            let after = t[k + 1..n.min(k + 12)].iter().map(|x| x.amp).fold(0.0, f64::max);
            if a < dip * before.min(after) {
                out.push(t[from..k].to_vec());
                from = k + 1;
                k += 1;
            }
        }
        k += 1;
    }
    out.push(t[from..].to_vec());
    out.retain(|x| !x.is_empty());
    out
}

struct RawNote {
    /// Where it starts and ends, in windows (fractional).
    start: f64,
    end: f64,
    hz: f64,
    amp: f64,
    loudness: f64,
    tonality: f64,
}

/// A track piece's edges (where its strength crosses half its median), pitch (the weighted
/// mean in cents of the windows between, outliers out), strength, loudness and tonality (the
/// medians of its windows').
fn summarise(t: &[Point]) -> RawNote {
    let amp = median(t.iter().map(|x| x.amp).collect());
    let half = 0.5 * amp;
    let i = t.iter().position(|x| x.amp >= half).unwrap_or(0);
    let k = t.iter().rposition(|x| x.amp >= half).unwrap_or(t.len() - 1);
    // between a window under half and one over it
    let cross = |a: &Point, b: &Point| a.j as f64 + (half - a.amp) / (b.amp - a.amp) * (b.j as f64 - a.j as f64);
    let start = if i > 0 { cross(&t[i - 1], &t[i]) } else { t[0].j as f64 - 0.5 };
    let end = if k + 1 < t.len() { cross(&t[k + 1], &t[k]) } else { t[t.len() - 1].j as f64 + 0.5 };
    let body = &t[i..=k];
    let inner = if body.len() >= 4 { &body[1..body.len() - 1] } else { body };
    let med = median(inner.iter().map(|x| x.hz).collect());
    let (mut num, mut den) = (0.0, 0.0);
    for x in inner.iter().filter(|x| cents(x.hz, med).abs() <= 25.0) {
        num += x.amp * cents(x.hz, med);
        den += x.amp;
    }
    let hz = if den > 0.0 { med * 2f64.powf(num / den / 1200.0) } else { med };
    RawNote {
        start,
        end,
        hz,
        amp,
        loudness: median(body.iter().map(|x| x.loudness).collect()),
        tonality: median(body.iter().map(|x| x.tonality).collect()),
    }
}

/// Voices for notes in start order (see the module docs).
fn assign_voices(notes: Vec<(Note, f64)>, slack: u64, voices: usize) -> Vec<Note> {
    let voices = voices.max(1);
    let mut out: Vec<Option<(Note, f64)>> = Vec::with_capacity(notes.len());
    // per voice: the index in `out` of its last note
    let mut last: Vec<Option<usize>> = vec![None; voices];
    for (n, amp) in notes {
        let free: Vec<usize> = (0..voices).filter(|&v| last[v].and_then(|i| out[i].as_ref()).is_none_or(|(m, _)| m.end_t <= n.start_t + slack)).collect();
        let chosen = if !free.is_empty() {
            // the first voice whose note ends where this one starts (a line goes on), else the
            // one whose note ended last
            let prev = |v: usize| last[v].and_then(|i| out[i].as_ref()).map(|x| x.0);
            let goes_on = |v: usize| prev(v).is_some_and(|m| m.end_t + slack >= n.start_t);
            let dist = |v: usize| prev(v).map_or(f64::INFINITY, |m| cents(m.hz(), n.hz()).abs());
            // ended latest first (a line paused by a rest), voices never used last
            let ended = |v: usize| prev(v).map_or(-1.0, |m| m.end_t as f64);
            let rank = |v: usize| if goes_on(v) { (0, 0.0, 0.0) } else { (1, -ended(v), dist(v)) };
            let v = *free
                .iter()
                .min_by(|&&a, &&b| {
                    let (ra, rb) = (rank(a), rank(b));
                    ra.0.cmp(&rb.0).then(ra.1.total_cmp(&rb.1)).then(ra.2.total_cmp(&rb.2)).then(a.cmp(&b))
                })
                .expect("free voice");
            Some(v)
        } else {
            let weakest = (0..voices).min_by(|&a, &b| {
                let s = |v: usize| last[v].and_then(|i| out[i].as_ref()).map_or(0.0, |x| x.1);
                s(a).total_cmp(&s(b))
            });
            weakest.filter(|&v| last[v].and_then(|i| out[i].as_ref()).is_some_and(|x| x.1 < amp))
        };
        let Some(v) = chosen else { continue };
        if let Some(i) = last[v] {
            if let Some((m, _)) = out[i].as_mut() {
                if m.end_t > n.start_t {
                    m.end_t = n.start_t;
                }
                if m.end_t <= m.start_t {
                    out[i] = None;
                }
            }
        }
        last[v] = Some(out.len());
        out.push(Some((Note { voice: v as u8, ..n }, amp)));
    }
    out.into_iter().flatten().map(|x| x.0).collect()
}

/// A radix-2 complex FFT of one size.
struct Fft {
    n: usize,
    rev: Vec<u32>,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        let bits = n.trailing_zeros();
        let rev = (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect();
        let cos = (0..n / 2).map(|k| (2.0 * PI * k as f64 / n as f64).cos()).collect();
        let sin = (0..n / 2).map(|k| -(2.0 * PI * k as f64 / n as f64).sin()).collect();
        Fft { n, rev, cos, sin }
    }

    /// The forward transform in place.
    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fft_matches_a_plain_dft() {
        let n = 64;
        let fft = Fft::new(n);
        let x: Vec<f64> = (0..n).map(|i| ((i * 7919) % 13) as f64 - 6.0).collect();
        let y: Vec<f64> = (0..n).map(|i| ((i * 104_729) % 11) as f64 - 5.0).collect();
        let (mut re, mut im) = (x.clone(), y.clone());
        fft.run(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0, 0.0);
            for t in 0..n {
                let w = -2.0 * PI * (k * t) as f64 / n as f64;
                sr += x[t] * w.cos() - y[t] * w.sin();
                si += x[t] * w.sin() + y[t] * w.cos();
            }
            assert!((sr - re[k]).abs() < 1e-9 && (si - im[k]).abs() < 1e-9, "bin {k}");
        }
    }

    #[test]
    fn peaks_are_interpolated_between_bins() {
        let n = 4096;
        let len = 1600;
        let fft = Fft::new(n);
        let win = hann(len);
        let wsum: f64 = win.iter().sum();
        let rate = CLOCK_HZ / f64::from(DT);
        let f = 441.7;
        let mut re = vec![0.0; n];
        let mut im = vec![0.0; n];
        for i in 0..len {
            re[i] = 0.3 * (2.0 * PI * f * i as f64 / rate).sin() * win[i];
        }
        fft.run(&mut re, &mut im);
        let mag: Vec<f64> = (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() / wsum * 2.0).collect();
        let (peaks, _) = find_peaks(&mag, 2, n / 2 - 2, rate / n as f64, 0.05, n as f64 / len as f64, &mut Vec::new());
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].f - f).abs() < 0.05, "{:?}", peaks[0]);
        assert!((peaks[0].a - 0.3).abs() < 0.01, "{:?}", peaks[0]);
        assert!(peaks[0].lobe);
    }

    #[test]
    fn voices_never_overlap_and_the_weakest_gives_way() {
        let n = |s: u64, e: u64, p: u32| Note { voice: 0, start_t: s, end_t: e, period_t: p, level: 1 };
        let notes = vec![(n(0, 100, 1000), 1.0), (n(0, 100, 2000), 0.5), (n(0, 100, 3000), 0.8), (n(50, 150, 4000), 0.9), (n(60, 160, 5000), 0.1)];
        let out = assign_voices(notes, 0, 3);
        assert_eq!(out.len(), 4, "{out:?}");
        // the 0.5 note was cut at 50 for the 0.9 one; the 0.1 one found no voice
        assert!(out.iter().any(|x| x.period_t == 2000 && x.end_t == 50));
        assert!(!out.iter().any(|x| x.period_t == 5000));
        for a in &out {
            for b in &out {
                if a != b && a.voice == b.voice {
                    assert!(a.end_t <= b.start_t || b.end_t <= a.start_t, "{a:?} {b:?}");
                }
            }
        }
    }
}
