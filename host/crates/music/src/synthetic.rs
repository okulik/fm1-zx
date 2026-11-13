// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Generated beeper edge streams with known notes, modelled on real music engines' loops, so
//! the decoders are tested without third-party code.
//!
//! - [`xor_loop`]: two 8-bit counters that each flip the speaker when they wrap, one `OUT` per
//!   loop iteration of 56.6 T on average (a free-running engine: notes last whole multiples of
//!   256 iterations and change anywhere in a frame). A wrap costs 21 T more (the reload), so it
//!   stretches the other counter's half-period too, and when both wrap in the same iteration the
//!   two flips cancel and the edge is lost. Detuned unison (N and N + 1) and chords such as 3:4.
//! - [`frame_loop`]: an interrupt-driven sequencer (one note per 50 Hz frame): two 16-bit
//!   down-counters at N and N - 1 (one voice doubled in detuned unison), 48.5 T per pass, each
//!   wrap flipping the speaker with its own `OUT`; the loop runs from the end of the interrupt
//!   routine to the next interrupt, so pitch changes land exactly on frame boundaries. Between
//!   arpeggios come whole frames of drums (a fixed buzz waveform, a pulse sweep, noise) and
//!   silent frames.
//! - [`slots_loop`]: a time-slot (TDM) engine: one loop of 153 T plus I/O contention with an `OUT`
//!   per channel, at 0, 34 and 83 T into it (slots of 34, 49 and 70 T), so each channel holds the
//!   speaker for its own slot. Each channel is a 16-bit phase accumulator with a duty threshold:
//!   it outputs 1 while the accumulator's high byte is below the threshold (0 = silent, 80h a
//!   square wave). Rests keep the loop running with every channel silent.

use zx_sys::{contention, zx_edge};

/// T-states per 50 Hz frame.
pub const FRAME_T: u64 = 69_888;
const CPU_HZ: f64 = 3_500_000.0;

/// A generated stream, frame by frame, with what the music engine played in each frame.
#[derive(Clone, Default)]
pub struct Stream {
    /// Each frame's beeper edges (T-state within the frame, `zx_edge.level`).
    pub frames: Vec<Vec<zx_edge>>,
    /// The level before the first frame.
    pub start_level: u8,
    /// The pitches (Hz) the engine plays at each frame's middle (empty: none).
    pub truth: Vec<Option<Vec<f64>>>,
    /// Frames that are a drum.
    pub drum: Vec<bool>,
    /// Frames with no sound at all (rests between notes).
    pub silent: Vec<bool>,
}

impl Stream {
    /// `b` appended to `self`, from the frame after `self`'s last.
    pub fn then(mut self, mut b: Stream) -> Stream {
        let end_level = self.frames.iter().flatten().last().map_or(self.start_level, |e| e.level);
        if (end_level ^ b.start_level) & 2 != 0 {
            // keep the EAR bit continuous: every edge of `b` must still change it
            for e in b.frames.iter_mut().flatten() {
                e.level ^= 2;
            }
        }
        self.frames.append(&mut b.frames);
        self.truth.append(&mut b.truth);
        self.drum.append(&mut b.drum);
        self.silent.append(&mut b.silent);
        self
    }
}

/// Collects absolute edge times and note spans, then cuts them into frames.
struct Builder {
    edges: Vec<(u64, u8)>,
    notes: Vec<(u64, u64, Vec<f64>)>,
    drums: Vec<(u64, u64)>,
    start_level: u8,
}

impl Builder {
    fn new(start_level: u8) -> Self {
        Builder { edges: Vec::new(), notes: Vec::new(), drums: Vec::new(), start_level }
    }

    fn finish(self, end: u64) -> Stream {
        let n = end.div_ceil(FRAME_T) as usize;
        let mut frames = vec![Vec::new(); n];
        for &(t, level) in &self.edges {
            if t < end {
                frames[(t / FRAME_T) as usize].push(zx_edge { tstate: (t % FRAME_T) as u32, level });
            }
        }
        let span = |i: usize| (i as u64 * FRAME_T, (i as u64 + 1) * FRAME_T);
        let truth = (0..n)
            .map(|i| {
                let mid = span(i).0 + FRAME_T / 2;
                Some(self.notes.iter().filter(|x| x.0 <= mid && mid < x.1).flat_map(|x| x.2.iter().copied()).collect())
            })
            .collect();
        let drum = (0..n)
            .map(|i| {
                let mid = span(i).0 + FRAME_T / 2;
                self.drums.iter().any(|d| d.0 <= mid && mid < d.1)
            })
            .collect();
        let silent = (0..n)
            .map(|i| {
                let (a, b) = span(i);
                frames[i].is_empty()
                    && !self.notes.iter().any(|x| x.0 < b && x.1 > a)
                    && !self.drums.iter().any(|d| d.0 < b && d.1 > a)
            })
            .collect();
        Stream { frames, start_level: self.start_level, truth, drum, silent }
    }
}

/// One note of [`xor_loop`]: `len` x 256 loop iterations with the counters reloading at `n1`
/// and `n2` (1-255), then `rest_frames` frames of silence.
#[derive(Clone, Copy, Debug)]
pub struct XorNote {
    pub len: u32,
    pub n1: u32,
    pub n2: u32,
    pub rest_frames: u32,
}

/// The two-counter XOR loop (see the module notes). Truth: each counter's measured mean
/// half-period over the note.
pub fn xor_loop(notes: &[XorNote]) -> Stream {
    // T-states in tenths: 56.6 T per iteration on average (56 T plus the OUT's contention)
    const ITER: u64 = 566;
    const RELOAD: u64 = 210;
    const DJNZ_OUT: u64 = 110;
    const GAP: u64 = 7_000; // between notes: the key check and the next note's set-up
    let mut b = Builder::new(0);
    let mut t10: u64 = 2 * FRAME_T * 10 / 3;
    let mut a: u8 = 0; // the level the loop holds (EAR and MIC flip together)
    let mut out: u8 = 0; // the level last sent to the port
    for note in notes {
        let start = t10 / 10;
        let (mut d, mut e) = (note.n1, note.n2);
        let mut wraps: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
        for i in 0..note.len * 256 {
            if a != out {
                b.edges.push((t10 / 10, a));
                out = a;
            }
            let at = t10 / 10;
            t10 += ITER;
            d -= 1;
            if d == 0 {
                d = note.n1;
                a ^= 3;
                wraps[0].push(at);
                t10 += RELOAD;
            }
            e -= 1;
            if e == 0 {
                e = note.n2;
                a ^= 3;
                wraps[1].push(at);
                t10 += RELOAD;
            }
            if (i + 1) % 256 == 0 {
                t10 += DJNZ_OUT;
            }
        }
        let end = t10 / 10;
        let hz: Vec<f64> = wraps
            .iter()
            .map(|w| CPU_HZ / (2.0 * (w[w.len() - 1] - w[0]) as f64 / (w.len() - 1) as f64))
            .collect();
        b.notes.push((start, end, hz));
        t10 += GAP + note.rest_frames as u64 * FRAME_T * 10;
    }
    b.finish(t10 / 10 + FRAME_T)
}

/// The counter reload of [`xor_loop`] for a frequency.
pub fn xor_n(hz: f64) -> u32 {
    (CPU_HZ / (2.0 * hz * 56.6)).round() as u32
}

/// A frequency from a MIDI note number.
pub fn hz_of_midi(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// A 20-note tune for [`xor_loop`]: detuned unisons and 3:4 chords over a melody, notes of
/// about 5, 10 and 21 frames, each followed by `rest_frames` frames of silence.
pub fn xor_tune(rest_frames: u32) -> Vec<XorNote> {
    let melody = [72, 76, 79, 74, 77, 71, 72, 67, 69, 76, 74, 72, 71, 69, 67, 64, 65, 69, 72, 71];
    let lens = [50, 50, 25, 100, 50, 25, 50, 50, 100, 25];
    melody
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            let n = xor_n(hz_of_midi(m as f64));
            let (n1, n2) = if i % 3 == 2 {
                let k = (n + 1) / 3; // a 3:4 chord: the melody note and the fourth below
                (3 * k, 4 * k)
            } else {
                (n, n + 1)
            };
            XorNote { len: lens[i % lens.len()], n1, n2, rest_frames }
        })
        .collect()
}

/// One frame of [`frame_loop`].
#[derive(Clone, Copy, Debug)]
pub enum FrameNote {
    /// The doubled voice at reload `n` (and `n - 1`).
    Melody(u32),
    Drum(Drum),
    Silent,
}

/// The drum routines of [`frame_loop`]: each fills the rest of its frame.
#[derive(Clone, Copy, Debug)]
pub enum Drum {
    /// A fixed waveform: one cycle each of 8 half-periods, repeated.
    Buzz,
    /// Pulses whose spacing grows every step.
    Sweep,
    /// Pseudo-random spacing.
    Noise,
}

/// Frequency of the [`frame_loop`] counter reload `n`.
pub fn frame_hz(n: u32) -> f64 {
    CPU_HZ / (2.0 * n as f64 * 48.5)
}

/// The counter reload of [`frame_loop`] for a frequency.
pub fn frame_n(hz: f64) -> u32 {
    (CPU_HZ / (2.0 * hz * 48.5)).round() as u32
}

/// The frame-synchronous engine (see the module notes). Truth: both counters' pitches.
pub fn frame_loop(frames: &[FrameNote]) -> Stream {
    const BUZZ: [u64; 8] = [2096, 2720, 3030, 3190, 3265, 3304, 1665, 846];
    let mut b = Builder::new(0);
    let mut level: u8 = 0;
    let (mut c1, mut c2) = (100u32, 99u32);
    let mut seed = 0x2545_f491u32;
    for (i, f) in frames.iter().enumerate() {
        let f0 = i as u64 * FRAME_T;
        let f1 = f0 + FRAME_T;
        let mut flip = |t: u64, b: &mut Builder| {
            level ^= 2;
            b.edges.push((t, level));
        };
        match *f {
            FrameNote::Melody(n) => {
                // the interrupt routine (sequencer) runs first; then the loop resumes, its
                // counters where the last melody frame left them
                let mut t2: u64 = (f0 + 1_000 + (i as u64 * 37) % 400) * 2;
                let mut pass = 0u64;
                while t2 / 2 + 60 < f1 {
                    let t = t2 / 2;
                    c1 -= 1;
                    if c1 == 0 {
                        c1 = n;
                        flip(t + 20, &mut b);
                    }
                    c2 -= 1;
                    if c2 == 0 {
                        c2 = n - 1;
                        flip(t + 40, &mut b);
                    }
                    t2 += if pass.is_multiple_of(2) { 96 } else { 98 }; // 48.5 T per pass
                    pass += 1;
                }
                b.notes.push((f0, f1, vec![frame_hz(n), frame_hz(n - 1)]));
            }
            FrameNote::Drum(kind) => {
                let mut t = f0 + 3_350;
                let mut k = 0u64;
                while t < f1 - 100 {
                    flip(t, &mut b);
                    t += match kind {
                        Drum::Buzz => BUZZ[(k as usize / 2) % 8],
                        Drum::Sweep => 55 + 26 * (k / 2) + if k % 2 == 1 { 19 } else { 0 },
                        Drum::Noise => {
                            seed ^= seed << 13;
                            seed ^= seed >> 17;
                            seed ^= seed << 5;
                            200 + (seed % 3200) as u64
                        }
                    };
                    k += 1;
                }
                b.drums.push((f0, f1));
            }
            FrameNote::Silent => {}
        }
    }
    b.finish(frames.len() as u64 * FRAME_T)
}

/// A frame-synchronous tune for [`frame_loop`]: `beats` beats of a drum frame, a 4-note
/// arpeggio (one note per frame) and 3 silent frames; the drums cycle buzz, sweep, buzz, noise.
pub fn frame_tune(beats: usize) -> Vec<FrameNote> {
    // arpeggios, highest note first (MIDI): A minor, F major, C major, G major
    let chords: [[u32; 4]; 4] = [[69, 64, 60, 57], [69, 65, 60, 53], [67, 64, 60, 55], [67, 62, 59, 55]];
    let drums = [Drum::Buzz, Drum::Sweep, Drum::Buzz, Drum::Noise];
    let mut out = Vec::new();
    for k in 0..beats {
        out.push(FrameNote::Drum(drums[k % 4]));
        for &m in &chords[(k / 2) % 4] {
            out.push(FrameNote::Melody(frame_n(hz_of_midi(m as f64))));
        }
        out.extend([FrameNote::Silent; 3]);
    }
    out
}

/// One note of [`slots_loop`]: each channel's accumulator step and duty threshold (0 = silent),
/// for `frames` frames.
#[derive(Clone, Copy, Debug)]
pub struct SlotsNote {
    pub step: [u16; 3],
    pub thr: [u8; 3],
    pub frames: u32,
}

/// The `OUT` offsets of [`slots_loop`]'s channels in its loop, and the loop's length (T).
pub const SLOTS_AT: [u64; 3] = [0, 34, 83];
pub const SLOTS_LOOP: u64 = 153;

/// The time-slot engine (see the module notes). Truth: each sounding channel's pitch, from its
/// step and the note's mean loop length (contention included).
pub fn slots_loop(notes: &[SlotsNote]) -> Stream {
    slots_engine(notes, SLOTS_LOOP, SLOTS_AT)
}

/// [`slots_loop`] with another loop length and `OUT` offsets (T).
pub fn slots_engine(notes: &[SlotsNote], loop_t: u64, at_t: [u64; 3]) -> Stream {
    const GAP: u64 = 2_200; // between notes: the sequencer reads the next row
    let mut b = Builder::new(0);
    let mut t: u64 = FRAME_T / 3;
    let mut level: u8 = 0;
    let mut phase = [0u16; 3];
    for note in notes {
        let start = t;
        let end = start + note.frames as u64 * FRAME_T;
        let mut iters = 0u64;
        while t < end {
            let base = t;
            for c in 0..3 {
                phase[c] = phase[c].wrapping_add(note.step[c]);
                let mut at = base + at_t[c];
                // the OUT's I/O contention delays it and everything after it
                let delay = contention((at % FRAME_T) as u32) as u64;
                at += delay;
                t += delay;
                let out = if ((phase[c] >> 8) as u8) < note.thr[c] { 2 } else { 0 };
                if out != level {
                    level = out;
                    b.edges.push((at, level));
                }
            }
            t += loop_t;
            iters += 1;
        }
        let mean = (t - start) as f64 / iters as f64;
        let hz: Vec<f64> =
            (0..3).filter(|&c| note.thr[c] != 0 && note.step[c] != 0).map(|c| note.step[c] as f64 * CPU_HZ / (mean * 65536.0)).collect();
        b.notes.push((start, t, hz));
        t += GAP;
    }
    b.finish(t + 4 * FRAME_T)
}

/// The accumulator step of [`slots_loop`] for a frequency (at the loop's mean length with
/// contention, about 156 T).
pub fn slots_step(hz: f64) -> u16 {
    (hz * 156.0 * 65536.0 / CPU_HZ).round() as u16
}

/// A 30-note tune for [`slots_loop`]: three voices (bass, middle, melody) with different duty
/// thresholds, notes of 4 to 16 frames, some with one or two voices silent, and two rests.
pub fn slots_tune() -> Vec<SlotsNote> {
    let bass = [36, 36, 43, 43, 41, 41, 43, 0, 45, 45, 40, 40, 41, 41, 43, 43, 36, 38, 40, 41, 43, 0, 45, 47, 48, 43, 41, 40, 38, 36];
    let mid = [60, 64, 64, 62, 60, 65, 62, 0, 57, 60, 64, 0, 60, 65, 59, 62, 0, 62, 64, 65, 67, 0, 64, 62, 60, 59, 57, 0, 55, 60];
    let mel = [72, 74, 76, 79, 77, 76, 74, 0, 69, 72, 76, 74, 72, 71, 67, 71, 72, 74, 0, 77, 79, 0, 76, 74, 72, 71, 69, 67, 71, 72];
    let lens = [8, 4, 6, 12, 5, 8, 16, 4, 6, 10];
    let thr = [0x80u8, 0x40, 0xc0];
    (0..30)
        .map(|i| {
            let ms = [bass[i], mid[i], mel[i]];
            let mut step = [0u16; 3];
            let mut th = [0u8; 3];
            for c in 0..3 {
                if ms[c] != 0 {
                    step[c] = slots_step(hz_of_midi(ms[c] as f64));
                    th[c] = thr[(c + i) % 3];
                }
            }
            SlotsNote { step, thr: th, frames: lens[i % lens.len()] }
        })
        .collect()
}
