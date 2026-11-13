// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The FM synth (`core/sound/fm.c`): pitch and silence.

use music::{period_q8, sample_rate, RATE};
use zx_sys::{Beeper, SndFm, SndFrame, SndVoice};

/// Renders `frames` frames of `fr`, each as long as the beeper's frame at the FM-1 rate.
fn render(fm: &mut SndFm, fr: &SndFrame, frames: usize, sizes: &mut Beeper) -> Vec<i16> {
    let mut out = Vec::new();
    for _ in 0..frames {
        let n = sizes.render(&[]).len();
        out.extend(fm.render(fr, n));
    }
    out
}

fn tone(hz: f64) -> SndFrame {
    let mut fr = SndFrame { confident: 1, ..Default::default() };
    fr.tone[0] = SndVoice { period_q8: period_q8(hz), level: 255 };
    fr
}

/// Frequency from the rising zero crossings (linearly interpolated) that follow a dip below
/// half the negative peak: a Schmitt trigger, so FM's small ripples near zero do not count.
fn crossing_hz(s: &[i16]) -> f64 {
    let peak = s.iter().map(|&x| (x as i32).abs()).max().unwrap();
    assert!(peak > 1000, "the voice sounds (peak {peak})");
    let th = peak / 2;
    let mut armed = false;
    let mut times = Vec::new();
    for i in 1..s.len() {
        let (a, b) = (s[i - 1] as f64, s[i] as f64);
        if (s[i] as i32) < -th {
            armed = true;
        }
        if armed && a <= 0.0 && b > 0.0 {
            times.push((i - 1) as f64 + a / (a - b));
            armed = false;
        }
    }
    assert!(times.len() > 10, "{} crossings", times.len());
    (times.len() - 1) as f64 * sample_rate() / (times[times.len() - 1] - times[0])
}

#[test]
fn fm_pitch() {
    for hz in [440.0, 110.0, 1760.0] {
        let mut fm = SndFm::new(RATE.0, RATE.1);
        let s = render(&mut fm, &tone(hz), 50, &mut Beeper::new(RATE.0, RATE.1));
        let secs = s.len() as f64 / sample_rate();
        assert!((secs - 1.0).abs() < 0.01, "1 s rendered ({secs})");
        // the second half: the index has decayed to its settled value
        let got = crossing_hz(&s[s.len() / 2..]);
        let err = (got / hz - 1.0).abs();
        println!("{hz} Hz: measured {got:.3} Hz ({:.4}%)", err * 100.0);
        assert!(err < 0.005, "{hz} Hz voice measured {got:.3} Hz");
    }
}

#[test]
fn fm_silence() {
    let release = (0.060 * sample_rate()).ceil() as usize;
    let mut chord = SndFrame { confident: 1, drum: 200, ..Default::default() };
    for (i, hz) in [110.0, 440.0, 660.0].into_iter().enumerate() {
        chord.tone[i] = SndVoice { period_q8: period_q8(hz), level: 200 };
    }
    let off = SndFrame::default();

    // nothing ever played: silence at once
    let mut fm = SndFm::new(RATE.0, RATE.1);
    let s = render(&mut fm, &off, 5, &mut Beeper::new(RATE.0, RATE.1));
    assert!(s.iter().all(|&x| x == 0), "an idle synth is silent");

    // a chord with a drum, then everything off: a release, then exact zeros within 60 ms
    for (name, held) in [("chord + drum", chord), ("drum only", SndFrame { drum: 255, ..Default::default() })] {
        let mut fm = SndFm::new(RATE.0, RATE.1);
        let mut sizes = Beeper::new(RATE.0, RATE.1);
        let on = render(&mut fm, &held, 10, &mut sizes);
        assert!(on.iter().any(|&x| x.abs() > 1000), "{name}: it sounds");
        let s = render(&mut fm, &off, 10, &mut sizes);
        assert!(s[..release / 4].iter().any(|&x| x != 0), "{name}: released, not cut off");
        if let Some(i) = s[release..].iter().position(|&x| x != 0) {
            panic!("{name}: sample {} after the voices stopped is {}", release + i, s[release + i]);
        }
    }
}
