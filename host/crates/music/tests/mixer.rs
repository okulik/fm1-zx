// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mixer (`core/sound/mixer.c`): Beeper mode is a plain copy, an FM mode delays all sound by
//! 3 frames, and switching between them neither clicks nor repeats sound.

use music::synthetic::{frame_loop, xor_loop, xor_tune, Drum, FrameNote, Stream, FRAME_T};
use music::{capture, castaway, render_stream, sample_rate, test_sna, Frame, RATE};
use zx_sys::{
    zx_edge, Beeper, SndDecoder, SndFm, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_ENTER_FRAMES, SND_MODE_BEEPER,
    SND_MODE_FM_A,
};

fn border(frames: usize) -> Vec<Frame> {
    capture(&mut test_sna("border.sna"), &mut Beeper::new(RATE.0, RATE.1), frames)
}

/// Castaway from its autostart; its title music plays from about 3 s (frame 150).
fn castaway_frames(frames: usize) -> Vec<Frame> {
    capture(&mut castaway(), &mut Beeper::new(RATE.0, RATE.1), frames)
}

/// The mixer's output for each frame, in the mode `mode_of(frame index)`.
fn mix(frames: &[Frame], mode_of: impl Fn(usize) -> u8) -> Vec<Vec<i16>> {
    let mut m = SndMixer::new(RATE.0, RATE.1);
    frames.iter().enumerate().map(|(i, f)| m.frame(mode_of(i), &f.edges, f.start_level, &f.beeper)).collect()
}

/// Sample index at which each frame starts, plus the total.
fn starts(frames: &[Frame]) -> Vec<usize> {
    let mut s = vec![0];
    for f in frames {
        s.push(s.last().unwrap() + f.beeper.len());
    }
    s
}

fn ms(x: f64) -> usize {
    (x / 1000.0 * sample_rate()).round() as usize
}

fn max_step(s: &[i16]) -> i32 {
    s.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0)
}

#[test]
fn beeper_mode_is_identical() {
    for (name, frames, from) in [("border", border(200), 0), ("castaway", castaway_frames(600), 400)] {
        let loud = frames[from..].iter().flat_map(|f| &f.beeper).filter(|&&x| x.abs() > 1000).count();
        assert!(loud > 1000, "{name}: frames {from}+ have sound ({loud} loud samples)");
        let out = mix(&frames, |_| SND_MODE_BEEPER);
        for (i, (o, f)) in out.iter().zip(&frames).enumerate() {
            assert!(o == &f.beeper, "{name}: frame {i} differs from the beeper");
        }
    }
}

#[test]
fn fm_mode_delays_three_frames() {
    let d = SND_DELAY_FRAMES as usize;
    for (name, frames) in [("border", border(200)), ("castaway", castaway_frames(600))] {
        let out = mix(&frames, |_| SND_MODE_FM_A);
        let beeper: Vec<i16> = frames.iter().flat_map(|f| f.beeper.iter().copied()).collect();
        let s = starts(&frames);
        // the delay: the first three frames' samples (frames have 880 or 881 samples)
        let delay = s[d];
        // where the music is decoded (Castaway's title, time slots) the FM voices play instead:
        // check the frames whose decision, and the two before it, play the beeper
        let dec = music::decode(frames.iter().map(|f| (f.edges.as_slice(), f.start_level)));
        let beeper_path = |t: usize| (t.saturating_sub(d + 2)..=t - d).all(|k| dec[k].confident == 0);
        let mut checked = 0;
        for (t, o) in out.iter().enumerate() {
            assert_eq!(o.len(), frames[t].beeper.len(), "{name}: frame {t} length");
            if t < d {
                assert!(o.iter().all(|&x| x == 0), "{name}: frame {t} is silence");
                continue;
            }
            if !beeper_path(t) {
                continue;
            }
            checked += 1;
            // frame t plays frame t - 3, starting within one sample of its first sample, and the
            // delayed stream is continuous: no sample repeated or dropped
            let from = s[t] - delay;
            assert!(from.abs_diff(s[t - d]) <= 1, "{name}: frame {t} starts {from}, frame {} at {}", t - d, s[t - d]);
            assert!(o[..] == beeper[from..from + o.len()], "{name}: frame {t} is not frame {}'s sound", t - d);
        }
        println!("{name}: {checked} of {} frames on the beeper path checked", out.len());
        assert!(checked >= 150, "{name}: {checked} frames checked");
    }
}

#[test]
fn mode_switch_no_click() {
    let frames = border(300);
    let (on, off) = (100, 200);
    let out: Vec<i16> =
        mix(&frames, |i| if (on..off).contains(&i) { SND_MODE_FM_A } else { SND_MODE_BEEPER }).concat();
    let b: Vec<i16> = frames.iter().flat_map(|f| f.beeper.iter().copied()).collect();
    let s = starts(&frames);
    let delay = s[SND_DELAY_FRAMES as usize];
    let (s1, s2) = (s[on], s[off]);
    let fade = ms(4.5);
    let delayed = |p: usize| b[p - delay];

    // which sound plays where: live up to the switch to FM; after its fade-out, silence until the
    // delayed path reaches the switch frame; then the delayed sound from the switch frame on;
    // after the switch back, the live sound. So nothing is played twice (beyond the fades' own
    // overlap) and no 60 ms block of the past is replayed at the switch.
    assert!(out[..s1] == b[..s1], "live before the switch to FM");
    assert!(out[s1 + fade..s1 + delay].iter().all(|&x| x == 0), "silence while the delay fills");
    if let Some(p) = (s1 + delay + fade..s2).find(|&p| out[p] != delayed(p)) {
        panic!("sample {p} is {}, not the delayed sound's {}", out[p], delayed(p));
    }
    let replay: Vec<i16> = (s1 + fade..s1 + delay).map(delayed).collect();
    assert!(replay.iter().any(|&x| x != 0), "the replay this guards against would not be silent");
    assert!(out[s2 + fade..] == b[s2 + fade..], "live after the switch back");

    // each change is a fade of at least 2 ms, not a cut (a cut in a square wave can step less
    // than the wave's own edges, so the step check below alone would not see it)
    let fade_out = (s1..s1 + fade).rev().find(|&p| out[p] != 0).map_or(0, |p| p + 1 - s1);
    assert!(fade_out >= ms(2.0), "the live sound fades out over {fade_out} samples");
    // samples from `at` until the output equals `want` for good
    let settles = |at: usize, want: &dyn Fn(usize) -> i16| (at..at + fade).rev().find(|&p| out[p] != want(p)).map_or(0, |p| p + 1 - at);
    let fade_in = settles(s1 + delay, &delayed);
    assert!(fade_in >= ms(2.0), "the delayed sound fades in over {fade_in} samples");
    let cross = settles(s2, &|p| b[p]);
    assert!(cross >= ms(2.0), "the delayed sound crossfades to the live one over {cross} samples");
    println!("fades: out {fade_out} samples, in {fade_in}, across {cross}");

    // no click: around each change the largest step is no larger than the signals' own
    let w = ms(5.0);
    for (what, at, sources) in [
        ("live -> silence", s1, vec![s1]),
        ("silence -> delayed", s1 + delay, vec![s1]),
        ("delayed -> live", s2, vec![s2, s2 - delay]),
    ] {
        // the signals there: the live and the delayed sound, as beeper samples around `src`
        let own = sources.iter().map(|&src| max_step(&b[src - w..src + w])).max().unwrap();
        let got = max_step(&out[at - w..at + w]);
        println!("{what}: largest step {got}, the signals' own {own}");
        assert!(own > 1000, "{what}: the tone sounds there");
        assert!(got <= own, "{what}: step {got} above the signals' own {own}");
    }
}

/// In an FM mode from the start: the output, the FM signal the mixer mixes in (rebuilt with a
/// decoder and synth of our own: the decisions' voices when confident, silence otherwise), the
/// delayed beeper, and the decisions (frame t's decision is about frame t - 3).
fn fm_signals(frames: &[Frame]) -> (Vec<i16>, Vec<i16>, Vec<i16>, Vec<SndFrame>) {
    let d = SND_DELAY_FRAMES as usize;
    let out = mix(frames, |_| SND_MODE_FM_A).concat();
    let (mut dec, mut fm) = (SndDecoder::new(), SndFm::new(RATE.0, RATE.1));
    let mut fm_out = Vec::new();
    let mut decisions = Vec::new();
    for (t, f) in frames.iter().enumerate() {
        let fr = dec.frame(&f.edges, f.start_level);
        decisions.push(fr);
        if t < d {
            fm_out.extend(std::iter::repeat_n(0, f.beeper.len()));
        } else {
            let src = if fr.confident != 0 { fr } else { SndFrame::default() };
            fm_out.extend(fm.render(&src, f.beeper.len()));
        }
    }
    let b: Vec<i16> = frames.iter().flat_map(|f| f.beeper.iter().copied()).collect();
    let delay = starts(frames)[d];
    let bd: Vec<i16> = (0..b.len()).map(|p| if p < delay { 0 } else { b[p - delay] }).collect();
    (out, fm_out, bd, decisions)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Plays {
    Fm,
    Beeper,
}

/// Every switch between the FM and the delayed beeper: (last sample of the old sound, first of
/// the new, the new sound). Samples where both are equal say nothing and are skipped.
fn switches(out: &[i16], fm: &[i16], bd: &[i16], from: usize) -> Vec<(usize, usize, Plays)> {
    let mut v = Vec::new();
    let mut last: Option<(usize, Plays)> = None;
    for p in from..out.len() {
        if fm[p] == bd[p] {
            continue;
        }
        let now = if out[p] == fm[p] {
            Plays::Fm
        } else if out[p] == bd[p] {
            Plays::Beeper
        } else {
            continue; // fading
        };
        if let Some((q, was)) = last {
            if was != now {
                v.push((q, p, now));
            }
        }
        last = Some((p, now));
    }
    v
}

/// The edges of `times` (T-states in the frame), alternating from `start_level`.
fn edges_from(times: &[u32], start_level: u8) -> Vec<zx_edge> {
    let mut level = start_level;
    times
        .iter()
        .map(|&t| {
            level ^= 2;
            zx_edge { tstate: t, level }
        })
        .collect()
}

#[test]
fn fm_beeper_crossfade_in_fm_mode() {
    // music, then a noise effect, a pause, and music again
    let tune = xor_tune(0);
    let noise = vec![FrameNote::Drum(Drum::Noise); 25];
    let s: Stream = xor_loop(&tune[..8])
        .then(frame_loop(&noise))
        .then(frame_loop(&[FrameNote::Silent; 10]))
        .then(xor_loop(&tune[8..16]));
    let frames = render_stream(&s);
    let (out, fm, bd, _) = fm_signals(&frames);
    let delay = starts(&frames)[SND_DELAY_FRAMES as usize];

    // nothing repeated or dropped: every sample is a mix of the FM and the delayed beeper of
    // the same moment
    for p in delay..out.len() {
        let (lo, hi) = (fm[p].min(bd[p]) as i32 - 1, fm[p].max(bd[p]) as i32 + 1);
        assert!((lo..=hi).contains(&(out[p] as i32)), "sample {p}: {} is not between FM {} and beeper {}", out[p], fm[p], bd[p]);
    }
    let sw = switches(&out, &fm, &bd, delay);
    println!("switches (from sample, to sample, to): {sw:?}");
    assert!(sw.iter().any(|x| x.2 == Plays::Beeper), "the effect plays on the beeper");
    assert!(sw.iter().filter(|x| x.2 == Plays::Fm).count() >= 2, "both tunes play in FM");
    let w = ms(5.0);
    for &(a, b, to) in &sw {
        let fade = b - a - 1;
        assert!(fade >= ms(2.0), "switch to {to:?} at sample {b}: a fade of {fade} samples");
        let c = (a + b) / 2;
        let own = max_step(&fm[c - w..c + w]).max(max_step(&bd[c - w..c + w]));
        let got = max_step(&out[c - w..c + w]);
        println!("to {to:?} at {c}: fade {fade} samples, largest step {got}, the signals' own {own}");
        assert!(got <= own, "switch to {to:?} at sample {c}: step {got} above the signals' own {own}");
    }
}

#[test]
fn hysteresis_no_flutter() {
    // music whose every other frame from frame 40 to 120 also carries stray short pulses (a third
    // of its edges), so the decoder is sure of one frame and unsure of the next
    let mut s = xor_loop(&xor_tune(0));
    let mut level = s.start_level;
    let mut seed = 0x1234_5678u32;
    for (i, f) in s.frames.iter_mut().enumerate() {
        let start = level;
        if (40..120).contains(&i) && i % 2 == 1 && !f.is_empty() {
            let mut times: Vec<u32> = f.iter().map(|e| e.tstate).collect();
            for _ in 0..times.len() / 6 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let t = seed % (FRAME_T as u32 - 100);
                times.extend([t, t + 30]);
            }
            times.sort();
            *f = edges_from(&times, start);
        }
        level = f.last().map_or(start, |e| e.level);
    }
    let frames = render_stream(&s);
    let (out, fm, bd, dec) = fm_signals(&frames);
    let d = SND_DELAY_FRAMES as usize;
    // the decisions about frames 40-119 (made at frames 43-122) alternate
    let conf: String = (40..120).map(|i| if dec[i + d].confident != 0 { 'C' } else { '.' }).collect();
    println!("decisions about frames 40-119: {conf}");
    let alternating = conf.as_bytes().windows(2).filter(|p| p[0] != p[1]).count();
    assert!(alternating >= 70, "the decisions alternate ({alternating} changes)");
    // the output does not follow them: at most one switch while they alternate (with the
    // sound of frame t playing during frame t + 3)
    let st = starts(&frames);
    let (a, b) = (st[40 + d + SND_ENTER_FRAMES as usize], st[120 + d]);
    let sw: Vec<_> = switches(&out, &fm, &bd, st[d]).into_iter().filter(|x| x.1 >= a && x.1 < b).collect();
    println!("switches while the decisions alternate: {sw:?}");
    assert!(sw.len() <= 1, "{} switches", sw.len());
}
