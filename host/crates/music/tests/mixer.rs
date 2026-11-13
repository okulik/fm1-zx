// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mixer (`core/sound/mixer.c`): Beeper mode is a plain copy, an FM mode delays all sound by
//! 3 frames, and switching between them neither clicks nor repeats sound.

use music::{capture, castaway, sample_rate, test_sna, Frame, RATE};
use zx_sys::{Beeper, SndMixer, SND_DELAY_FRAMES, SND_MODE_BEEPER, SND_MODE_FM_A};

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
        for (t, o) in out.iter().enumerate() {
            assert_eq!(o.len(), frames[t].beeper.len(), "{name}: frame {t} length");
            if t < d {
                assert!(o.iter().all(|&x| x == 0), "{name}: frame {t} is silence");
                continue;
            }
            // frame t plays frame t - 3, starting within one sample of its first sample, and the
            // delayed stream is continuous: no sample repeated or dropped
            let from = s[t] - delay;
            assert!(from.abs_diff(s[t - d]) <= 1, "{name}: frame {t} starts {from}, frame {} at {}", t - d, s[t - d]);
            assert!(o[..] == beeper[from..from + o.len()], "{name}: frame {t} is not frame {}'s sound", t - d);
        }
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
