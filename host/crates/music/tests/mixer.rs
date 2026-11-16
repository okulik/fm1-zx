// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mixer (`core/sound/mixer.c`): Beeper mode is a plain copy, an FM mode delays all sound by
//! 3 frames, and switching between them neither clicks nor repeats sound.

use music::synthetic::{frame_loop, xor_loop, xor_tune, Drum, FrameNote, Stream, FRAME_T};
use music::{capture, castaway, render_stream, sample_rate, test_sna, Frame, RATE};
use zx_sys::{
    zx_edge, Beeper, Machine, SndDecoder, SndFm, SndFrame, SndMixer, SndRoles, SND_DELAY_FRAMES, SND_ENTER_FRAMES, SND_LEAVE_FRAMES,
    SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B, SndEngines,
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
    frames.iter().enumerate().map(|(i, f)| m.frame(mode_of(i), None, &f.edges, f.start_level, &f.beeper)).collect()
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
    // sound that is never music (the decoder never confident), so an FM mode plays exactly the
    // beeper, 3 frames late, on every frame
    let scroll = capture(&mut test_sna("scroll.sna"), &mut Beeper::new(RATE.0, RATE.1), 300);
    for (name, frames) in [("border", border(300)), ("scroll", scroll)] {
        let dec = music::decode(frames.iter().map(|f| (f.edges.as_slice(), f.start_level)));
        assert!(dec.iter().all(|f| f.confident == 0), "{name}: decoded as music");
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
        let loud = beeper.iter().filter(|&&x| x.abs() > 1000).count();
        println!("{name}: {} frames compared, {loud} loud samples", out.len() - d);
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
/// decoder and synth of our own: the decisions' voices when confident; in FM, through a lone
/// unconfident decision, the last confident one's voices; silence otherwise; the roles the live
/// rule's), the delayed beeper, and the decisions (frame t's decision is about frame t - 3).
fn fm_signals(frames: &[Frame]) -> (Vec<i16>, Vec<i16>, Vec<i16>, Vec<SndFrame>) {
    let d = SND_DELAY_FRAMES as usize;
    let out = mix(frames, |_| SND_MODE_FM_A).concat();
    let (mut dec, mut fm, mut roles) = (SndDecoder::new(), SndFm::new(RATE.0, RATE.1), SndRoles::new());
    let mut fm_out = Vec::new();
    let mut decisions = Vec::new();
    // the mixer's hysteresis: FM after SND_ENTER_FRAMES confident decisions, the beeper after
    // SND_LEAVE_FRAMES unconfident ones
    let (mut on, mut run, mut held) = (false, 0u32, SndFrame::default());
    for (t, f) in frames.iter().enumerate() {
        let fr = dec.frame(&f.edges, f.start_level);
        decisions.push(fr);
        if t < d {
            fm_out.extend(std::iter::repeat_n(0, f.beeper.len()));
        } else {
            if (fr.confident != 0) == on {
                run = 0;
            } else {
                run += 1;
                if run >= if on { SND_LEAVE_FRAMES } else { SND_ENTER_FRAMES } {
                    on = !on;
                    run = 0;
                }
            }
            let mut src = if fr.confident != 0 {
                held = SndFrame { tone: fr.tone, ..SndFrame::default() };
                fr
            } else if on {
                held
            } else {
                SndFrame::default()
            };
            roles.frame(&mut src);
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
            zx_edge { tstate: t, level, pc: 0 }
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

/// The xor tune with stray short pulses (a third of its edges) added to `frames`, so the decoder is
/// unsure of each of them.
fn tune_with_strays(frames: &[usize]) -> Stream {
    let mut s = xor_loop(&xor_tune(0));
    let mut level = s.start_level;
    let mut seed = 0x1234_5678u32;
    for (i, f) in s.frames.iter_mut().enumerate() {
        let start = level;
        if frames.contains(&i) && !f.is_empty() {
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
    s
}

#[test]
fn lone_unsure_frame_stays_fm() {
    // music the decoder is sure of except for single frames: FM holds through each of them with
    // the voices it had (a drop to the beeper for 2-3 frames is a click in the music), and only
    // a second unsure frame in a row leaves FM
    let lone = [40usize, 60, 80];
    let frames = render_stream(&tune_with_strays(&lone));
    let out = mix(&frames, |_| SND_MODE_FM_A).concat();
    let mut dec = SndDecoder::new();
    let dec: Vec<SndFrame> = frames.iter().map(|f| dec.frame(&f.edges, f.start_level)).collect();
    let d = SND_DELAY_FRAMES as usize;
    let conf: String = (30..110).map(|i| if dec[i + d].confident != 0 { 'C' } else { '.' }).collect();
    println!("decisions about frames 30-109: {conf}");
    for &i in &lone {
        assert!(dec[i + d].confident == 0, "frame {i}: the decoder is unsure");
        assert!(dec[i - 1 + d].confident != 0 && dec[i + 1 + d].confident != 0, "frame {i}: sure around it");
    }
    // FM from frame 30 to 110 sounds all the time: the delayed beeper never comes in (where it
    // differs from FM by much), and no 5 ms of the output is silent
    let st = starts(&frames);
    let bd = delayed_beeper(&frames);
    let (a, b) = (st[30 + d], st[110 + d]);
    let beeperish = (a..b).filter(|&p| out[p] == bd[p] && bd[p].abs() > 1000).count();
    assert!(beeperish < ms(1.0), "{beeperish} samples of the delayed beeper in the music");
    let w = ms(5.0);
    for p in (a..b - w).step_by(w / 2) {
        assert!(out[p..p + w].iter().any(|&x| x.abs() > 500), "silence at samples {p}..{}", p + w);
    }
}

fn fnv(s: &[i16]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for x in s {
        for b in x.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Runs `frames` frames of `m` through a mixer in the mode `mode_of(frame index)`, the machine
/// given to the mixer as the firmware does; returns the frames and the mixer's output.
fn mix_live(m: &mut Machine, frames: usize, mode_of: impl Fn(usize) -> u8) -> (Vec<Frame>, Vec<Vec<i16>>) {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    // effects off: these tests pin FM against the delayed beeper (the follower, tests/fx.rs, would
    // re-voice FM: engines' beeper frames)
    mixer.set_effects(false);
    let (mut fr, mut out) = (Vec::new(), Vec::new());
    for i in 0..frames {
        m.run_frame();
        let start_level = m.edges_start_level();
        beeper.set_level(start_level);
        let edges = m.edges().to_vec();
        let samples = beeper.render(&edges);
        out.push(mixer.frame(mode_of(i), Some(m), &edges, start_level, &samples));
        fr.push(Frame { edges, start_level, beeper: samples });
    }
    (fr, out)
}

#[test]
fn fm_a_unchanged() {
    // FM: sound (method A) plays sample for sample as it did when the live rule gave its voices
    // their roles (tests/roles.rs; Colony 8's first 12 s are a lone voice, a lead, as before); its
    // decisions are pinned apart, unchanged since before (tests/instruments.rs,
    // decisions_unchanged)
    for (name, m, want) in [
        ("castaway", castaway(), 0xc579d5b7c8aad978u64),
        ("colony8", music::colony8(), 0xd69dd22c970004b0),
    ] {
        let frames = capture(&mut { m }, &mut Beeper::new(RATE.0, RATE.1), 600);
        let dec = music::decode(frames.iter().map(|f| (f.edges.as_slice(), f.start_level)));
        let conf = dec.iter().filter(|f| f.confident != 0).count();
        assert!(conf > 300, "{name}: music decoded ({conf} confident frames)");
        let out = mix(&frames, |_| SND_MODE_FM_A).concat();
        println!("(\"{name}\", {:#018x}),", fnv(&out));
        assert_eq!(fnv(&out), want, "{name}");
    }
}

/// The beeper stream 3 frames late, as an FM mode plays it when nothing is confident.
fn delayed_beeper(frames: &[Frame]) -> Vec<i16> {
    let b: Vec<i16> = frames.iter().flat_map(|f| f.beeper.iter().copied()).collect();
    let delay = starts(frames)[SND_DELAY_FRAMES as usize];
    (0..b.len()).map(|p| if p < delay { 0 } else { b[p - delay] }).collect()
}

#[test]
fn fm_b_unclaimed_is_delayed_beeper() {
    // music from code no engine entry knows (tune.sna's loop), which method A plays in FM: FM:
    // engines claims none of it, the delayed beeper throughout
    let mut m = test_sna("tune.sna");
    let dec = music::decode(capture(&mut test_sna("tune.sna"), &mut Beeper::new(RATE.0, RATE.1), 300).iter().map(|f| (f.edges.as_slice(), f.start_level)));
    assert!(dec.iter().filter(|f| f.confident != 0).count() > 200, "method A plays the tune in FM");
    let (frames, out) = mix_live(&mut m, 300, |_| SND_MODE_FM_B);
    let bd = delayed_beeper(&frames);
    let out = out.concat();
    let loud = bd.iter().filter(|&&x| x.abs() > 1000).count();
    assert!(loud > 10_000, "the tune sounds ({loud} loud samples)");
    if let Some(p) = (0..out.len()).find(|&p| out[p] != bd[p]) {
        panic!("sample {p} is {}, not the delayed beeper's {}", out[p], bd[p]);
    }
}

#[test]
fn engines_claim_only_known_engines() {
    // method B on its own, both tables: no frame of code no entry knows claimed, no engine named
    // for it; the tables name their entries; and a mixer given no machine (nothing is B's) plays
    // FM: engines as the delayed beeper, even on Castaway's title music
    let mut m = test_sna("tune.sna");
    let (mut b, mut t) = (SndEngines::new(), SndEngines::new_test());
    for _ in 0..300 {
        m.run_frame();
        for e in [&mut b, &mut t] {
            let fr = e.frame(&m, m.edges(), m.edges_start_level());
            assert!(fr.confident == 0 && fr.engine == 0 && fr.events().is_empty(), "{fr:?}");
        }
    }
    assert_eq!(b.name(0), "");
    let names: Vec<String> = (1..=7).map(|i| b.name(i)).collect();
    assert_eq!(names, ["Castaway", "Colony 8", "ROM BEEP", "Cave Runner title", "Cave Runner in game", "Unison", ""]);
    assert_eq!(t.name(7), "Test burst");
    let frames = castaway_frames(600);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_test_engines();
    mixer.set_effects(false);   // B's claims alone (the follower would re-voice the beeper: tests/fx.rs)
    let out: Vec<i16> =
        frames.iter().flat_map(|f| mixer.frame(SND_MODE_FM_B, None, &f.edges, f.start_level, &f.beeper)).collect();
    assert!(out == delayed_beeper(&frames), "the delayed beeper");
}

#[test]
fn switch_between_fm_choices() {
    // FM: sound, then FM: engines, then FM: sound again, in Castaway's title music: the delayed
    // stream carries on sample for sample, only FM and the delayed beeper crossfade
    let (b_from, a_from) = (300, 400);
    let mode = |i: usize| if (b_from..a_from).contains(&i) { SND_MODE_FM_B } else { SND_MODE_FM_A };
    let (frames, out) = mix_live(&mut castaway(), 600, mode);
    let out = out.concat();
    let bd = delayed_beeper(&frames);
    let st = starts(&frames);
    let d = SND_DELAY_FRAMES as usize;

    // the FM the mixer plays: the chosen method's voices where it is confident (A's in FM:
    // sound, B's in FM: engines), silence elsewhere, with the live rule's roles (afresh at each
    // switch: A's slots are not B's channels), through one synth that never restarts
    let (mut dec, mut eng, mut fm, mut roles) = (SndDecoder::new(), SndEngines::new(), SndFm::new(RATE.0, RATE.1), SndRoles::new());
    let mut m = castaway();
    let mut fm_out = Vec::new();
    for (t, f) in frames.iter().enumerate() {
        m.run_frame();
        let fa = dec.frame(&f.edges, f.start_level);
        let fb = eng.frame(&m, &f.edges, f.start_level);
        if t < d {
            fm_out.extend(std::iter::repeat_n(0, f.beeper.len()));
        } else {
            let fr = if mode(t) == SND_MODE_FM_A { fa } else { fb };
            let mut src = if fr.confident != 0 { fr } else { SndFrame::default() };
            if mode(t) != mode(t - 1) {
                roles = SndRoles::new();
            }
            roles.frame(&mut src);
            fm_out.extend(fm.render(&src, f.beeper.len()));
        }
    }

    // every sample a mix of the FM and the delayed beeper of the same moment: no silence put in,
    // nothing replayed
    for p in st[d]..out.len() {
        let (lo, hi) = (fm_out[p].min(bd[p]) as i32 - 1, fm_out[p].max(bd[p]) as i32 + 1);
        assert!((lo..=hi).contains(&(out[p] as i32)), "sample {p}: {} is not between FM {} and beeper {}", out[p], fm_out[p], bd[p]);
    }
    // no 20 ms of silence where both the FM (rests in the music are silent FM) and the delayed
    // beeper sound
    let w = ms(20.0);
    for p in (st[d]..out.len() - w).step_by(w / 4) {
        if bd[p..p + w].iter().any(|&x| x != 0) && fm_out[p..p + w].iter().any(|&x| x != 0) {
            assert!(out[p..p + w].iter().any(|&x| x != 0), "silence at samples {p}..{}", p + w);
        }
    }
    // FM in both FM: sound stretches and in FM: engines (B claims the title music), and the
    // switches are crossfades, not cuts
    let fm_in = |a: usize, b: usize| (st[a]..st[b]).filter(|&p| out[p] == fm_out[p] && fm_out[p] != bd[p]).count();
    let (before, during, after) = (fm_in(d, b_from), fm_in(b_from + d, a_from), fm_in(a_from, frames.len()));
    println!("FM samples: {before} before FM: engines, {during} in it, {after} after");
    assert!(before > 10_000 && during > 10_000 && after > 10_000, "FM plays FM ({before}, {during}, {after})");
    let sw = switches(&out, &fm_out, &bd, st[d]);
    println!("switches: {sw:?}");
    for &(a, b, to) in &sw {
        assert!(b - a - 1 >= ms(2.0), "switch to {to:?} at sample {b}: a fade of {} samples", b - a - 1);
    }
}
