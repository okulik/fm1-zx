// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scoring decoded voices against the truth, frame by frame and note by note: a pitch is right
//! within half a semitone, at the same frame or one frame either side.
//!
//! - Frame level: a detected voice is correct if a true pitch within 0.5 semitone sounds in the
//!   frame or a neighbouring one (precision); a true pitch is found if a detected voice within
//!   0.5 semitone is in the frame or a neighbouring one (recall).
//! - Note level: true notes are runs of a pitch (within 0.1 semitone) over consecutive frames;
//!   detected notes are runs of a detected pitch (within 0.5 semitone of the run's first frames)
//!   over consecutive frames. A detected note is wrong if no true note within 0.5 semitone
//!   overlaps it (one frame of slack either side); a true note is split if the frames its
//!   detected notes cover form more than one run.

use std::fmt;

/// Half a semitone.
pub const TOL: f64 = 0.5;

/// The MIDI note number (fractional) of a frequency.
pub fn midi_of(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// A note as frame span `[a, b)` and MIDI pitch.
#[derive(Clone, Debug)]
pub struct Note {
    pub a: usize,
    pub b: usize,
    pub midi: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Score {
    /// Frames with a known truth.
    pub frames: usize,
    /// Detected voice-frames that are right / wrong.
    pub correct: usize,
    pub wrong_pitch: usize,
    /// True voice-frames found / missed.
    pub found: usize,
    pub missed: usize,
    pub true_notes: usize,
    pub notes_found: usize,
    pub detected_notes: usize,
    /// Detected notes matching no true note.
    pub wrong: Vec<Note>,
    /// True notes covered by more than one detected note.
    pub splits: usize,
}

impl Score {
    /// Frame precision, 0-1.
    pub fn precision(&self) -> f64 {
        self.correct as f64 / (self.correct + self.wrong_pitch).max(1) as f64
    }

    /// Frame recall, 0-1.
    pub fn recall(&self) -> f64 {
        self.found as f64 / (self.found + self.missed).max(1) as f64
    }
}

impl fmt::Display for Score {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frames {}: precision {:.1}% ({} right, {} wrong), recall {:.1}% ({} found, {} missed); \
             notes: {} true, {} found, {} detected, {} wrong, {} split",
            self.frames,
            100.0 * self.precision(),
            self.correct,
            self.wrong_pitch,
            100.0 * self.recall(),
            self.found,
            self.missed,
            self.true_notes,
            self.notes_found,
            self.detected_notes,
            self.wrong.len(),
            self.splits
        )?;
        for n in self.wrong.iter().take(8) {
            write!(f, "\n  wrong note: frames {}..{} MIDI {:.2}", n.a, n.b, n.midi)?;
        }
        Ok(())
    }
}

fn has(set: &[f64], m: f64) -> bool {
    set.iter().any(|x| (x - m).abs() < TOL)
}

/// Runs of a pitch over consecutive frames: a pitch continues a run whose reference (the mean
/// of its first 3 frames) is within `tol` semitones.
fn runs(sets: &[Vec<f64>], tol: f64) -> Vec<Note> {
    struct Open {
        a: usize,
        refm: f64,
        pitches: Vec<f64>,
        used: bool,
    }
    let mut open: Vec<Open> = Vec::new();
    let mut out = Vec::new();
    let close = |o: Open, b: usize, out: &mut Vec<Note>| {
        let mut p = o.pitches;
        p.sort_by(|x, y| x.partial_cmp(y).unwrap());
        out.push(Note { a: o.a, b, midi: p[p.len() / 2] });
    };
    for (i, set) in sets.iter().enumerate() {
        for o in open.iter_mut() {
            o.used = false;
        }
        for &m in set {
            let best = open
                .iter_mut()
                .filter(|o| !o.used && (o.refm - m).abs() < tol)
                .min_by(|x, y| (x.refm - m).abs().partial_cmp(&(y.refm - m).abs()).unwrap());
            match best {
                Some(o) => {
                    o.used = true;
                    o.pitches.push(m);
                    if o.pitches.len() <= 3 {
                        o.refm = o.pitches.iter().sum::<f64>() / o.pitches.len() as f64;
                    }
                }
                None => open.push(Open { a: i, refm: m, pitches: vec![m], used: true }),
            }
        }
        let (keep, done): (Vec<Open>, Vec<Open>) = open.into_iter().partition(|o| o.used);
        open = keep;
        for o in done {
            close(o, i, &mut out);
        }
    }
    for o in open {
        close(o, sets.len(), &mut out);
    }
    out
}

/// Scores `det` (Hz of the voices played in each frame) against `truth` (Hz the engine played in
/// each frame; `None` = unknown, not scored).
pub fn score(truth: &[Option<Vec<f64>>], det: &[Vec<f64>]) -> Score {
    let n = truth.len().min(det.len());
    let tm: Vec<Option<Vec<f64>>> = truth[..n].iter().map(|t| t.as_ref().map(|v| v.iter().map(|&h| midi_of(h)).collect())).collect();
    let dm: Vec<Vec<f64>> = det[..n].iter().map(|v| v.iter().map(|&h| midi_of(h)).collect()).collect();
    let mut s = Score::default();
    let near = |i: usize, f: &dyn Fn(usize) -> Vec<f64>| -> Vec<f64> {
        (i.saturating_sub(1)..=(i + 1).min(n - 1)).flat_map(f).collect()
    };
    for i in 0..n {
        let Some(t) = &tm[i] else { continue };
        s.frames += 1;
        let tn = near(i, &|j| tm[j].clone().unwrap_or_default());
        let dn = near(i, &|j| dm[j].clone());
        for &d in &dm[i] {
            if has(&tn, d) {
                s.correct += 1;
            } else {
                s.wrong_pitch += 1;
            }
        }
        for &x in t {
            if has(&dn, x) {
                s.found += 1;
            } else {
                s.missed += 1;
            }
        }
    }
    let true_notes = runs(&tm.iter().map(|t| t.clone().unwrap_or_default()).collect::<Vec<_>>(), 0.1);
    let det_notes = runs(&dm, TOL);
    let overlaps = |x: &Note, y: &Note| x.a < y.b + 1 && y.a < x.b + 1;
    s.true_notes = true_notes.len();
    s.detected_notes = det_notes.len();
    for t in &true_notes {
        let matching: Vec<&Note> = det_notes.iter().filter(|d| (d.midi - t.midi).abs() < TOL && overlaps(d, t)).collect();
        if !matching.is_empty() {
            s.notes_found += 1;
        }
        // split: inside the true note, the frames its detected notes cover form more than one run
        let covered: Vec<bool> = (t.a..t.b).map(|i| matching.iter().any(|d| d.a <= i && i < d.b)).collect();
        let pieces = covered.iter().enumerate().filter(|&(k, &c)| c && (k == 0 || !covered[k - 1])).count();
        if pieces > 1 {
            s.splits += 1;
        }
    }
    for d in &det_notes {
        if (d.a..d.b).all(|i| tm[i].is_none()) {
            continue;
        }
        if !true_notes.iter().any(|t| (t.midi - d.midi).abs() < TOL && overlaps(d, t)) {
            s.wrong.push(d.clone());
        }
    }
    s
}
