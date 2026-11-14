// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The FM synth (`core/sound/fm.c`): pitch and silence, and note events inside a frame.

use music::{period_q8, sample_rate, RATE};
use zx_sys::{Beeper, SndEvent, SndFm, SndFrame, SndVoice, ZX_FRAME_TSTATES};

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

/// The rising zero crossings (linearly interpolated, in samples) that follow a dip below half
/// the negative peak: a Schmitt trigger, so FM's small ripples near zero do not count.
fn crossings(s: &[i16]) -> Vec<f64> {
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
    times
}

/// Frequency from the crossings of `s` (at least `min` of them).
fn hz_of(s: &[i16], min: usize) -> f64 {
    let times = crossings(s);
    assert!(times.len() >= min, "{} crossings", times.len());
    (times.len() - 1) as f64 * sample_rate() / (times[times.len() - 1] - times[0])
}

fn crossing_hz(s: &[i16]) -> f64 {
    hz_of(s, 11)
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

/// The sample of a frame of `n` samples that an event at T-state `t` starts on.
fn at(t: u32, n: usize) -> usize {
    t as usize * n / ZX_FRAME_TSTATES as usize
}

fn with_event(mut fr: SndFrame, ev: SndEvent) -> SndFrame {
    fr.ev[fr.n_ev as usize] = ev;
    fr.n_ev += 1;
    fr
}

fn note(t: u32, hz: f64) -> SndEvent {
    SndEvent { t, period_q8: if hz > 0.0 { period_q8(hz) } else { 0 }, voice: 0, level: if hz > 0.0 { 255 } else { 0 } }
}

/// Renders the frames `frs` one after another; returns the samples and each frame's start.
fn render_seq(frs: &[SndFrame]) -> (Vec<i16>, Vec<usize>) {
    let mut fm = SndFm::new(RATE.0, RATE.1);
    let mut sizes = Beeper::new(RATE.0, RATE.1);
    let (mut out, mut starts) = (Vec::new(), Vec::new());
    for fr in frs {
        starts.push(out.len());
        let n = sizes.render(&[]).len();
        out.extend(fm.render(fr, n));
    }
    (out, starts)
}

#[test]
fn event_starts_mid_frame() {
    let first = with_event(SndFrame { confident: 1, ..Default::default() }, note(ZX_FRAME_TSTATES / 2, 440.0));
    let mut frs = vec![first];
    frs.extend(std::iter::repeat_n(tone(440.0), 4));
    let (s, st) = render_seq(&frs);
    let n = st[1];
    assert_eq!(at(ZX_FRAME_TSTATES / 2, n), n / 2);
    assert!(s[..n / 2 - 2].iter().all(|&x| x == 0), "silent before the event");
    // after it the voice sounds: its first sample within an 8 kHz tick (the envelope's step) of
    // the event, then no stretch of silence
    let tick = (sample_rate() / 8000.0).ceil() as usize;
    let first = s.iter().position(|&x| x != 0).unwrap();
    println!("event at sample {}, first sound at {first}", n / 2);
    assert!(first >= n / 2 - 2 && first <= n / 2 + 2 + tick, "first sound at {first}, the event at {}", n / 2);
    let mut zeros = 0;
    for (i, &x) in s.iter().enumerate().skip(n / 2 + 2 + tick) {
        zeros = if x == 0 { zeros + 1 } else { 0 };
        assert!(zeros < 4, "silence at sample {i}");
    }
    let got = crossing_hz(&s[n / 2..]);
    println!("after the event: {got:.3} Hz");
    assert!((got / 440.0 - 1.0).abs() < 0.005, "{got:.3} Hz");
}

#[test]
fn event_stops_mid_frame() {
    let t = 31_500; // 9 ms
    let mut frs = vec![tone(440.0); 5];
    frs.push(with_event(tone(440.0), note(t, 0.0)));
    frs.extend(std::iter::repeat_n(SndFrame { confident: 1, ..Default::default() }, 4));
    let (s, st) = render_seq(&frs);
    let held = render_seq(&vec![tone(440.0); 10]).0;
    let p = st[5] + at(t, st[6] - st[5]);
    // the same sound up to the event, as if no event came
    assert!(s[..p] == held[..p], "unchanged before the event");
    // from it, a release: each period's peak below the one before, down to exact zeros within
    // the release time (50 ms) and an 8 kHz tick
    let period = (sample_rate() / 440.0).round() as usize;
    let release = (0.050 * sample_rate()).ceil() as usize + (sample_rate() / 8000.0).ceil() as usize;
    let peak = |a: usize| s[a..a + period].iter().map(|&x| (x as i32).abs()).max().unwrap();
    let before = peak(p - period);
    let peaks: Vec<i32> = (0..).map(|k| p + k * period).take_while(|&a| a + period <= p + release).map(peak).collect();
    println!("peak before {before}, after: {peaks:?}");
    assert!(before > 1000, "the voice sounds before the event");
    for (k, w) in peaks.windows(2).enumerate() {
        assert!(w[1] <= w[0], "period {} after the event: peak {} after {}", k + 1, w[1], w[0]);
    }
    assert!(peaks[0] <= before && peaks[peaks.len() / 2] < before * 2 / 3, "falling: {peaks:?}");
    let held_peak = held[p + release / 2..p + release / 2 + period].iter().map(|&x| (x as i32).abs()).max().unwrap();
    assert!(held_peak * 3 > before * 2, "without the event the voice holds ({held_peak})");
    if let Some(i) = s[p + release..].iter().position(|&x| x != 0) {
        panic!("sample {} after the release is {}", p + release + i, s[p + release + i]);
    }
}

#[test]
fn two_notes_in_one_frame() {
    let mut frs = vec![tone(440.0); 3];
    frs.push(with_event(tone(440.0), note(ZX_FRAME_TSTATES / 2, 660.0)));
    frs.push(tone(660.0));
    let (s, st) = render_seq(&frs);
    let (a, n) = (st[3], st[4] - st[3]);
    let h = a + at(ZX_FRAME_TSTATES / 2, n);
    let first = hz_of(&s[a..h], 4);
    let second = hz_of(&s[h..a + n], 4);
    println!("halves: {first:.2} Hz, {second:.2} Hz");
    assert!((first / 440.0 - 1.0).abs() < 0.01, "first half {first:.2} Hz");
    assert!((second / 660.0 - 1.0).abs() < 0.01, "second half {second:.2} Hz");
}

#[test]
fn drum_event_mid_frame() {
    let drum = |t: u32, level: u8| SndEvent { t, period_q8: 0, voice: 3, level };
    let on = with_event(SndFrame { confident: 1, ..Default::default() }, drum(ZX_FRAME_TSTATES / 2, 200));
    let held = SndFrame { confident: 1, drum: 200, ..Default::default() };
    let off = with_event(held, drum(ZX_FRAME_TSTATES / 4, 0));
    let rest = SndFrame { confident: 1, ..Default::default() };
    let (s, st) = render_seq(&[on, held, off, rest, rest, rest, rest]);
    let n = st[1];
    // the noise starts at the event's sample (within an 8 kHz tick, the envelope's step)
    let tick = (sample_rate() / 8000.0).ceil() as usize;
    assert!(s[..n / 2 - 2].iter().all(|&x| x == 0), "silent before the drum event");
    let first = s.iter().position(|&x| x != 0).unwrap();
    println!("drum event at sample {}, first sound at {first}", n / 2);
    assert!(first >= n / 2 - 2 && first <= n / 2 + 2 + tick, "first sound at {first}, the event at {}", n / 2);
    assert!(s[n / 2 + tick..n].iter().filter(|&&x| x != 0).count() > (n / 2 - tick) * 9 / 10, "the noise sounds");
    // a level-0 drum event releases it: the same sound up to the event, exact zeros within the
    // release time (50 ms) and a tick after it, while a drum left on still sounds then
    let p = st[2] + at(ZX_FRAME_TSTATES / 4, st[3] - st[2]);
    let release = (0.050 * sample_rate()).ceil() as usize + tick;
    let kept = render_seq(&[on, held, held, held, held, held, held]).0;
    assert!(s[..p] == kept[..p], "unchanged before the off event");
    assert!(s[p..p + 20].iter().any(|&x| x != 0), "released, not cut off");
    if let Some(i) = s[p + release..].iter().position(|&x| x != 0) {
        panic!("sample {} after the drum's release is {}", p + release + i, s[p + release + i]);
    }
    assert!(kept[p + release..p + release + 100].iter().any(|&x| x != 0), "without the event the drum still sounds");
}
