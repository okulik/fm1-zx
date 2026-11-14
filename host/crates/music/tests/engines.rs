// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Method B, the known music engines (`core/sound/engines.c`): its readers against the truth
//! readers (`truth.rs`) on Castaway's title tune and Escape from Colony 8's music, B against
//! method A where A is confident, the test engine of `burst_sna` moved in memory and held through
//! its gaps, frames with other code's edges, edges that disagree with the voices read, an engine
//! overwritten in place, and no B on the pack's effects, the border tone and noise.

use fmzx_pack::manifest::{encode_keymap, input_index, parse_manifest, BUTTON_COUNT};
use fmzx_pack::testprogs::{burst_sna, noise_sna, BURST_HZ, BURST_OUT};
use music::score::midi_of;
use music::truth::{Colony8Drums, Engine};
use music::{capture, decode, hz_of_period_q8, repo, test_sna, Frame, RATE};
use zx_sys::{zx_edge, Beeper, KeyMapper, KmInput, Machine, SndEngines, SndFrame, SndMixer, SndVoice, SND_DELAY_FRAMES, SND_MODE_FM_B};

/// The voices at a decision's end: its start state with its events applied.
fn end_voices(f: &SndFrame) -> [SndVoice; 3] {
    let mut t = f.tone;
    for ev in f.events() {
        if (ev.voice as usize) < t.len() {
            t[ev.voice as usize] = SndVoice { period_q8: ev.period_q8, level: ev.level };
        }
    }
    t
}

/// Per voice: its pitch (Hz) or None when silent.
fn hz(v: &[SndVoice; 3]) -> [Option<f64>; 3] {
    v.map(|x| (x.period_q8 != 0 && x.level != 0).then(|| hz_of_period_q8(x.period_q8)))
}

/// Sounding pitches of a voice state.
fn sounding(v: &[SndVoice; 3]) -> Vec<f64> {
    hz(v).into_iter().flatten().collect()
}

/// A drum sounds in the decision (at its start or from an event).
fn has_drum(f: &SndFrame) -> bool {
    f.drum != 0 || f.events().iter().any(|e| e.voice == 3 && e.level != 0)
}

/// A run of a machine with method B fed every frame.
struct Run {
    frames: Vec<Frame>,
    /// B's decision about each frame.
    dec: Vec<SndFrame>,
    /// The truth reader's (channel, Hz) at each frame's end (None: the engine is not running).
    chans: Vec<Option<Vec<(usize, f64)>>>,
    /// Frames a Colony 8 drum starts in.
    drum: Vec<bool>,
}

/// Runs `m` for `frames` frames, feeding `b`; `edit(frame, machine, edges)` may change the
/// machine before the frame runs (`edges` None) and the edges B gets after it.
fn run(mut m: Machine, frames: usize, mut b: SndEngines, engine: Option<Engine>, mut edit: impl FnMut(usize, &mut Machine, Option<&mut Vec<zx_edge>>)) -> Run {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut drums = Colony8Drums::new();
    let mut r = Run { frames: Vec::new(), dec: Vec::new(), chans: Vec::new(), drum: Vec::new() };
    let mut level = 0;
    for i in 0..frames {
        edit(i, &mut m, None);
        let mut f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        edit(i, &mut m, Some(&mut f.edges));
        r.dec.push(b.frame(&m, &f.edges, f.start_level));
        r.chans.push(engine.and_then(|e| e.read_channels(&m)));
        r.drum.push(engine == Some(Engine::Colony8) && drums.frame(&m));
        level = f.edges.last().map_or(f.start_level, |e| e.level);
        r.frames.push(f);
    }
    for _ in 0..SND_DELAY_FRAMES {
        r.dec.push(b.frame(&m, &[], level));
    }
    r.dec.drain(..SND_DELAY_FRAMES as usize);
    r
}

fn no_edit(_: usize, _: &mut Machine, _: Option<&mut Vec<zx_edge>>) {}

/// Frames where the truth reader reads notes: B confident, its engine `name`, and each channel's
/// voice at the frame's end that of the truth (within `tol` semitones; silent where the truth
/// has no note). Returns (frames scored, frames right).
fn score_truth(what: &str, r: &Run, b: &SndEngines, range: std::ops::Range<usize>, name: &str, tol: f64) -> (usize, usize) {
    let (mut n, mut ok) = (0, 0);
    let mut shown = 0;
    for i in range {
        let Some(t) = r.chans[i].as_ref().filter(|v| !v.is_empty()) else { continue };
        n += 1;
        let d = &r.dec[i];
        let v = hz(&end_voices(d));
        let right = d.confident != 0
            && b.name(d.engine) == name
            && (0..3).all(|c| match (t.iter().find(|x| x.0 == c), v[c]) {
                (Some(&(_, th)), Some(bh)) => (midi_of(th) - midi_of(bh)).abs() <= tol,
                (None, None) => true,
                _ => false,
            });
        if right {
            ok += 1;
        } else if shown < 12 {
            shown += 1;
            println!("  {what} frame {i}: truth {t:?}, B {v:?} confident {} engine {:?}", d.confident, b.name(d.engine));
        }
    }
    (n, ok)
}

#[test]
fn castaway_matches_truth() {
    let b = SndEngines::new();
    let r = run(music::castaway(), 1500, SndEngines::new(), Some(Engine::Castaway), no_edit);
    let (n, ok) = score_truth("castaway", &r, &b, 0..1500, "Castaway", 0.1);
    println!("castaway: B equals the truth in {ok} of {n} frames with notes");
    assert!(n > 800, "the tune plays ({n} frames)");
    assert!(ok * 100 >= n * 99, "B equals the truth in {ok} of {n} frames");
}

#[test]
fn colony8_matches_truth() {
    let (a, z) = (125, 1550); // the run tests/slots.rs uses: the title's music, no keys
    let b = SndEngines::new();
    let r = run(music::colony8(), z + 4, SndEngines::new(), Some(Engine::Colony8), no_edit);
    let (n, ok) = score_truth("colony8", &r, &b, a..z, "Colony 8", 0.1);
    println!("colony8: B equals the truth in {ok} of {n} frames with notes");
    assert!(n > 800, "the tune plays ({n} frames)");
    assert!(ok * 100 >= n * 99, "B equals the truth in {ok} of {n} frames");
    // a drum that drives the speaker (edges from its routines) is a confident drum in the frame
    // it starts in or the next
    let drum_pcs = [0x94a6u16, 0x94ba, 0x94c0];
    let audible = |k: usize| r.frames[k].edges.iter().any(|e| drum_pcs.contains(&e.pc));
    let starts: Vec<usize> = (a..z).filter(|&f| r.drum[f] && (audible(f) || audible(f + 1))).collect();
    let missed: Vec<usize> = starts.iter().copied().filter(|&f| !(f..=f + 1).any(|k| r.dec[k].confident != 0 && has_drum(&r.dec[k]))).collect();
    let false_drums: Vec<usize> = (a..z).filter(|&i| r.dec[i].confident != 0 && has_drum(&r.dec[i]) && !(i.saturating_sub(1)..=i).any(&audible)).collect();
    println!("colony8: {} audible drums, {} missed {missed:?}; drums where none sounds {false_drums:?}", starts.len(), missed.len());
    assert!(starts.len() >= 30, "audible drums ({})", starts.len());
    assert!(missed.is_empty(), "drums not reported: {missed:?}");
    assert!(false_drums.is_empty(), "drums where none sounds: {false_drums:?}");
}

/// Every voice state a decision passes through: its start and after each of its events.
fn states(f: &SndFrame) -> Vec<[SndVoice; 3]> {
    let mut t = f.tone;
    let mut v = vec![t];
    for ev in f.events() {
        if (ev.voice as usize) < t.len() {
            t[ev.voice as usize] = SndVoice { period_q8: ev.period_q8, level: ev.level };
            v.push(t);
        }
    }
    v
}

/// Is `x` within `tol` semitones of one of `ys`?
fn near(x: f64, ys: &[f64], tol: f64) -> bool {
    ys.iter().any(|&y| (midi_of(x) - midi_of(y)).abs() <= tol)
}

/// A note change in B's decisions within `k` frames of frame i (events or a changed start state).
fn near_change(dec: &[SndFrame], i: usize, k: usize) -> bool {
    (i.saturating_sub(k)..=(i + k).min(dec.len() - 1)).any(|j| dec[j].events().iter().any(|e| e.voice < 3) || (j > 0 && sounding(&dec[j].tone) != sounding(&end_voices(&dec[j - 1]))))
}

/// B's voice on channel c moves within frames i-2..=i+2 (it changes pitch, starts or stops in a
/// state B passes through there).
fn moving(dec: &[SndFrame], i: usize, c: usize) -> bool {
    let mut p = (i.saturating_sub(2)..=(i + 2).min(dec.len() - 1)).flat_map(|j| states(&dec[j])).map(|s| hz(&s)[c]);
    let first = p.next().flatten();
    p.any(|x| match (x, first) {
        (Some(x), Some(y)) => (midi_of(x) - midi_of(y)).abs() > 0.5,
        (None, None) => false,
        _ => true,
    })
}

#[test]
fn agrees_with_a() {
    // On the frames where A is confident, B agrees on every voice: B claims the frame and in
    // one of the voice states it passes through in the frame (its notes change inside frames,
    // A's at their edges) every voice of A's is within 0.5 semitone of one of B's and every
    // sounding voice of B's within 0.5 semitone of one of A's. A frame A calls silent agrees
    // with B silent in one of its states, or with B not claiming it when the frame has no edges
    // (the beeper's silence). One exception, A's rule for time-slot channels (slots.c,
    // snd_slots_voice: a channel has a voice only where a run of agreeing periods spans the
    // moment): A reports nothing for a channel whose pitch moves (a slide, a note starting or
    // ending), so a voice of B's that A lacks is excused when B's voice on that channel moves
    // within 2 frames (A's look-ahead) of the frame. A contradicts B when both claim a frame and
    // A hears a pitch B plays in none of its states, away from B's note changes.
    for (name, m, range) in [("castaway", music::castaway(), 150..1500), ("colony8", music::colony8(), 125..1550)] {
        let r = run(m, range.end + 4, SndEngines::new(), None, no_edit);
        let a = decode(r.frames.iter().map(|f| (f.edges.as_slice(), f.start_level)));
        let (mut n, mut agree, mut excused, mut silent) = (0, 0, 0, 0);
        let mut contra = Vec::new();
        for i in range.clone() {
            if a[i].confident == 0 {
                continue;
            }
            n += 1;
            let av = music::voices_hz(&a[i]);
            let b = &r.dec[i];
            let bst = states(b);
            let heard = |x: f64| bst.iter().any(|s| near(x, &sounding(s), 0.5));
            // per state: (agrees, agrees only with the exception)
            let fits = |s: &[SndVoice; 3]| -> (bool, bool) {
                let bv = sounding(s);
                if !av.iter().all(|&x| near(x, &bv, 0.5)) {
                    return (false, false);
                }
                let extra: Vec<usize> = (0..3).filter(|&c| hz(s)[c].is_some_and(|x| !near(x, &av, 0.5))).collect();
                (extra.iter().all(|&c| moving(&r.dec, i, c)), !extra.is_empty())
            };
            let ok = if b.confident == 0 {
                av.is_empty() && r.frames[i].edges.is_empty()
            } else {
                bst.iter().any(|s| fits(s).0)
            };
            if ok {
                agree += 1;
                if b.confident == 0 {
                    silent += 1;
                } else if !bst.iter().any(|s| fits(s) == (true, false)) {
                    excused += 1;
                }
            }
            if b.confident != 0 && av.iter().any(|&x| !heard(x)) && !near_change(&r.dec, i, 2) {
                contra.push((i, av.clone(), bst.iter().map(sounding).collect::<Vec<_>>()));
            }
        }
        println!(
            "{name}: A confident in {n} frames, B agrees in {agree} ({excused} only with a voice A lacks on a moving channel, \
             {silent} silent frames B leaves to the beeper); contradictions {}: {:?}",
            contra.len(),
            &contra[..contra.len().min(8)]
        );
        assert!(n > 500, "{name}: A confident in {n} frames");
        assert!(agree * 100 >= n * 95, "{name}: B agrees with A in {agree} of {n} frames");
        assert!(contra.is_empty(), "{name}: A contradicts B in {} frames", contra.len());
    }
}

/// A machine running `burst_sna(base)`.
fn burst(base: u16) -> Machine {
    let mut m = Machine::new();
    m.load_sna(&burst_sna(base)).expect("loads");
    m
}

/// The notes B starts (their pitches), in order, from confident decisions.
fn notes_started(dec: &[SndFrame]) -> Vec<f64> {
    dec.iter().filter(|d| d.confident != 0).flat_map(|d| d.events().iter().filter(|e| e.voice == 0 && e.period_q8 != 0).map(|e| hz_of_period_q8(e.period_q8)).collect::<Vec<_>>()).collect()
}

#[test]
fn moved_engine() {
    let mut seqs = Vec::new();
    for base in [0x8000u16, 0xb123] {
        let b = SndEngines::new_test();
        let r = run(burst(base), 100, SndEngines::new_test(), None, no_edit);
        let names: Vec<String> = r.dec.iter().filter(|d| d.confident != 0).map(|d| b.name(d.engine)).collect();
        assert!(names.iter().all(|n| n == "Test burst"), "engine names {names:?}");
        let notes = notes_started(&r.dec);
        println!("burst at {base:04X}: notes {notes:.2?}");
        assert!(notes.len() >= 3 * BURST_HZ.len(), "{base:04X}: {} notes", notes.len());
        for (k, &f) in notes.iter().enumerate() {
            let want = BURST_HZ[k % BURST_HZ.len()];
            assert!((f / want - 1.0).abs() < 0.005, "{base:04X}: note {k} at {f:.2} Hz, the melody's is {want:.2} Hz");
        }
        seqs.push(notes);
    }
    assert_eq!(seqs[0], seqs[1], "the same notes wherever the engine sits");
}

#[test]
fn burst_holds_through_gaps() {
    let frames = 130; // four times the melody
    let mut m = burst(0x8000);
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_test_engines();
    let mut b = SndEngines::new_test();
    let (mut out, mut starts_in, mut on) = (Vec::new(), Vec::new(), Vec::new());
    let mut inp = Vec::new();
    let mut dec = Vec::new();
    for _ in 0..frames {
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        starts_in.push(inp.len());
        inp.extend_from_slice(&f.beeper);
        out.extend(mixer.frame(SND_MODE_FM_B, Some(&m), &f.edges, f.start_level, &f.beeper));
        on.push(mixer.fm_on());
        dec.push((b.frame(&m, &f.edges, f.start_level), f));
    }
    // fm_on() after frame t's call is the decision about frame t - 3
    let first = on.iter().position(|&x| x).expect("FM starts");
    let off: Vec<usize> = (first..frames).filter(|&i| !on[i]).collect();
    println!("FM from call {first}; off in calls {off:?}");
    assert!(first <= 5, "FM from the first burst ({first})");
    assert!(off.is_empty(), "FM off in calls {off:?}");
    // each burst's FM note starts in the output within 1 ms of its first beeper edge (the
    // output is the input 3 frames late, to the sample)
    let delay = starts_in[SND_DELAY_FRAMES as usize];
    let ms = RATE.0 as f64 / RATE.1 as f64 / 1000.0;
    let mut bursts = 0;
    for i in 1..frames - 4 {
        let (f, prev) = (&dec[i].1, &dec[i - 1].1);
        if f.edges.is_empty() || !prev.edges.is_empty() {
            continue;
        }
        let n = f.beeper.len();
        let at = starts_in[i] + f.edges[0].tstate as usize * n / 69_888 + delay;
        let onset = (at - 400..at + 400).find(|&k| out[k] != 0).expect("the FM note sounds");
        let err = (onset as f64 - at as f64) / ms;
        println!("burst at frame {i}: FM note from sample {onset}, its first edge at {at}: {err:+.2} ms");
        assert!(out[at - 400..onset].iter().all(|&x| x == 0), "silence before burst {i}");
        assert!(err.abs() <= 1.0, "burst at frame {i}: the FM note starts {err:+.2} ms from its first edge");
        bursts += 1;
    }
    assert!(bursts >= 20, "{bursts} bursts");
}

/// Writes a routine at F000h that drives the speaker (two `OUT`s) and returns, and calls it at
/// the machine's next instruction.
fn call_other_code(m: &mut Machine) {
    let code = [0xf5, 0x3e, 0x10, 0xd3, 0xfe, 0xaf, 0xd3, 0xfe, 0xf1, 0xc9]; // PUSH AF; LD A,10h; OUT; XOR A; OUT; POP AF; RET
    for (k, &c) in code.iter().enumerate() {
        m.poke(0xf000 + k as u16, c);
    }
    let mut r = m.regs();
    let sp = r.sp.wrapping_sub(2);
    m.poke(sp, r.pc as u8);
    m.poke(sp.wrapping_add(1), (r.pc >> 8) as u8);
    r.sp = sp;
    r.pc = 0xf000;
    m.set_regs(&r);
}

#[test]
fn mixed_frame_beeper() {
    let dry = run(burst(0x8000), 100, SndEngines::new_test(), None, no_edit);
    let has = |i: usize| !dry.frames[i].edges.is_empty();
    // the last silent frame before a burst, and a frame of a later burst
    let gap = (40..90).find(|&i| !has(i) && has(i + 1)).unwrap();
    let busy = (gap + 6..95).find(|&i| has(i)).unwrap();
    assert!(dry.dec[gap].confident != 0 && dry.dec[busy].confident != 0, "both claimed without the other code");
    let r = run(burst(0x8000), 100, SndEngines::new_test(), None, |i, m, edges| {
        if (i == gap || i == busy) && edges.is_none() {
            call_other_code(m);
        }
    });
    for i in [gap, busy] {
        let pcs: Vec<u16> = r.frames[i].edges.iter().map(|e| e.pc).collect();
        println!("frame {i}: edges from {pcs:04X?}; confident {}, next {}", r.dec[i].confident, r.dec[i + 1].confident);
        assert!(pcs.iter().any(|&p| (0xf000..0xf010).contains(&p)), "frame {i} has the other code's edges");
        assert_eq!(r.dec[i].confident, 0, "frame {i} with the other code's edges is claimed");
        assert_ne!(r.dec[i + 1].confident, 0, "frame {} after it is not claimed", i + 1);
    }
}

#[test]
fn many_out_sites_not_b() {
    // a burst frame whose edges also come from 12 other OUT addresses (more than the frame's
    // listed sites and the addresses B looks up past them): not B's, and the frame after it is
    const BASE: u16 = 0x8000;
    let dry = run(burst(BASE), 100, SndEngines::new_test(), None, no_edit);
    let busy = (40..90).find(|&i| dry.frames[i].edges.len() >= 6 && !dry.frames[i + 1].edges.is_empty()).unwrap();
    assert!(dry.dec[busy].confident != 0, "claimed without the other code");
    let r = run(burst(BASE), 100, SndEngines::new_test(), None, |i, _, edges| {
        if let Some(edges) = edges.filter(|_| i == busy) {
            let t0 = edges.last().unwrap().tstate + 200;
            let mut level = edges.last().unwrap().level;
            for k in 0..12u16 {
                level ^= 2;
                edges.push(zx_edge { tstate: t0 + 100 * k as u32, level, pc: 0xf000 + 4 * k });
            }
        }
    });
    println!("frame {busy}: confident {}, the next {}", r.dec[busy].confident, r.dec[busy + 1].confident);
    assert_eq!(r.dec[busy].confident, 0, "a frame with 12 other OUTs is claimed");
    assert_ne!(r.dec[busy + 1].confident, 0, "the frame after it is not claimed");
}

#[test]
fn long_run_carry() {
    // one note held for 10.5 minutes (burst_sna's melody made one note, its bursts 65,534 edges
    // long, no gap): the check's state is carried frame to frame all that time, the channels the
    // engine does not use never refreshed; B claims the note throughout (times carried from
    // frames before stay bounded: no overflow, which host and device could resolve differently)
    const BASE: u16 = 0x8000;
    let frames = 31_500;
    let mut m = burst(BASE);
    for k in 0..BURST_HZ.len() as u16 {
        let at = BASE + 0x2c + 5 * k;
        for (j, v) in [253u8, 0xfe, 0xff, 1, 0].into_iter().enumerate() {
            m.poke(at + j as u16, v); // n 253, 65,534 edges, a gap of one turn
        }
    }
    let r = run(m, frames, SndEngines::new_test(), None, no_edit);
    let off: Vec<usize> = (10..frames).filter(|&i| r.dec[i].confident == 0 || hz(&end_voices(&r.dec[i]))[0].is_none_or(|f| (f / BURST_HZ[0] - 1.0).abs() > 0.005)).collect();
    println!("{frames} frames of one note: not claimed at it in {} frames {:?}", off.len(), &off[..off.len().min(10)]);
    assert!(off.is_empty(), "the held note not claimed in {} frames", off.len());
}

#[test]
fn edges_disagree_beeper() {
    let dry = run(music::castaway(), 1500, SndEngines::new(), None, no_edit);
    let picked: Vec<usize> = (200..1500).filter(|&i| dry.dec[i].confident != 0 && !sounding(&end_voices(&dry.dec[i])).is_empty() && dry.frames[i].edges.len() >= 20).step_by(5).collect();
    assert!(picked.len() >= 100, "{} frames", picked.len());
    // 3 semitones up in even frames, down in odd ones
    let off = |i: usize| sounding(&end_voices(&dry.dec[i]))[0] * 2f64.powf(if i.is_multiple_of(2) { 3.0 } else { -3.0 } / 12.0);
    let square = |i: usize, edges: &[zx_edge]| -> Vec<zx_edge> {
        let f = off(i);
        let half = 3_500_000.0 / f / 2.0;
        let mut out = Vec::new();
        let mut t = 100.0;
        let mut level = dry.frames[i].start_level ^ 2;
        while t < 69_800.0 {
            out.push(zx_edge { tstate: t as u32, level, pc: edges[out.len() % edges.len()].pc });
            level ^= 2;
            t += half;
        }
        out
    };
    let r = run(music::castaway(), 1500, SndEngines::new(), None, |i, _, edges| {
        if let Some(edges) = edges.filter(|_| picked.contains(&i)) {
            *edges = square(i, edges);
        }
    });
    let claimed: Vec<usize> = picked.iter().copied().filter(|&i| r.dec[i].confident != 0).collect();
    for &i in claimed.iter().take(10) {
        println!("  frame {i}: voices {:.1?}, square at {:.1} Hz", sounding(&end_voices(&dry.dec[i])), off(i));
    }
    println!("{} frames with a square 3 semitones off the voice read; claimed {claimed:?}", picked.len());
    assert!(claimed.is_empty(), "frames claimed whose edges disagree with the voices: {claimed:?}");
}

#[test]
fn edges_disagree_after_stop_beeper() {
    // Colony 8: a note ends with the OUT after the loop (9521h) and the next often begins in the
    // same frame. In frames with such a stop followed by the next note's edges, the edges after
    // the stop are replaced by a regular square 3 semitones off the voice B reads at the frame's
    // end (the next note's), from the same OUTs: those frames are not claimed
    const STOP: u16 = 0x9521;
    let tonal = [0x9505u16, 0x950d, 0x9517];
    let (from, to) = (125, 1550);
    let dry = run(music::colony8(), to + 4, SndEngines::new(), None, no_edit);
    let after_stop = |i: usize| -> Option<usize> {
        let e = &dry.frames[i].edges;
        let k = e.iter().position(|x| x.pc == STOP)?;
        (e[k + 1..].iter().filter(|x| tonal.contains(&x.pc)).count() >= 20).then_some(k)
    };
    let picked: Vec<usize> = (from..to).filter(|&i| dry.dec[i].confident != 0 && !sounding(&end_voices(&dry.dec[i])).is_empty() && after_stop(i).is_some()).collect();
    assert!(picked.len() >= 30, "{} frames with a stop and the next note", picked.len());
    let off = |i: usize| sounding(&end_voices(&dry.dec[i]))[0] * 2f64.powf(if i.is_multiple_of(2) { 3.0 } else { -3.0 } / 12.0);
    let replace = |i: usize, edges: &mut Vec<zx_edge>| {
        let k = after_stop(i).unwrap();
        let pcs: Vec<u16> = edges[k + 1..].iter().map(|x| x.pc).filter(|p| tonal.contains(p)).collect();
        let half = 3_500_000.0 / off(i) / 2.0;
        let mut level = edges[k].level ^ 2;
        let mut t = edges[k].tstate as f64 + 500.0;
        edges.truncate(k + 1);
        let mut n = 0;
        while t < 69_800.0 {
            edges.push(zx_edge { tstate: t as u32, level, pc: pcs[n % pcs.len()] });
            level ^= 2;
            n += 1;
            t += half;
        }
    };
    let r = run(music::colony8(), to + 4, SndEngines::new(), None, |i, _, edges| {
        if let Some(edges) = edges.filter(|_| picked.contains(&i)) {
            replace(i, edges);
        }
    });
    let claimed: Vec<usize> = picked.iter().copied().filter(|&i| r.dec[i].confident != 0).collect();
    for &i in claimed.iter().take(10) {
        println!("  frame {i}: voices {:.1?}, square at {:.1} Hz", sounding(&end_voices(&dry.dec[i])), off(i));
    }
    println!("{} Colony 8 frames with a square 3 semitones off the next note after the stop; claimed {claimed:?}", picked.len());
    assert!(claimed.is_empty(), "frames claimed whose edges after the stop disagree with the voices: {claimed:?}");
}

#[test]
fn overwritten_engine_forgotten() {
    let at = 60;
    let base = 0x8000u16;
    let out = base + BURST_OUT;
    let r = run(burst(base), 160, SndEngines::new_test(), None, |i, m, edges| {
        if i == at && edges.is_none() {
            // the engine's bytes become NOPs and a tone loop around the same OUT
            for a in base..base + 0x2c {
                m.poke(a, 0);
            }
            for (k, &c) in [0x06, 0x40, 0x10, 0xfe, 0xee, 0x10, 0xd3, 0xfe, 0x18, 0xf6].iter().enumerate() {
                m.poke(out - 6 + k as u16, c); // LD B,40h; DJNZ $; XOR 10h; OUT (FEh),A; JR back
            }
            let mut regs = m.regs();
            regs.pc = out - 6;
            m.set_regs(&regs);
        }
    });
    let before = (0..at).filter(|&i| r.dec[i].confident != 0).count();
    let after: Vec<usize> = (at..r.dec.len()).filter(|&i| r.dec[i].confident != 0).collect();
    let pcs: Vec<u16> = r.frames[at + 5].edges.iter().map(|e| e.pc).take(3).collect();
    println!("claimed in {before} frames before the overwrite, after it in {after:?}; edges then from {pcs:04X?}");
    assert!(before > 30, "the engine is claimed before ({before})");
    assert!(pcs.iter().all(|&p| p == out), "the speaker is driven from the same OUT");
    assert!(after.is_empty(), "claimed after the overwrite: {after:?}");
}

/// One step of a `demo` script: inputs (KmInput bits) held from `at` for `hold` ms.
struct Step {
    button: u32,
    note: u32,
    at: u64,
    hold: u64,
}

/// Parses `demo = "<input>@<ms>[+<hold>],..."` (hold 100 ms when not given).
fn parse_demo(s: &str) -> Vec<Step> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (key, when) = p.split_once('@').expect("key@ms");
            let (at, hold) = when.split_once('+').map_or((when, 100), |(a, h)| (a, h.parse().expect("hold")));
            let i = input_index(key).expect("input");
            let (button, note) = if i < BUTTON_COUNT { (1 << i, 0) } else { (0, 1 << (i - BUTTON_COUNT)) };
            Step { button, note, at: at.parse().expect("ms"), hold }
        })
        .collect()
}

/// Game `id` of `games/games.toml` under its demo (keys `delay_ms` later, the steps `extra`
/// added) for `frames` frames, with B (the firmware's table) fed: B's decisions, whether the
/// game's music engine runs at each frame's end (`truth.rs`) and the frames with speaker edges.
fn demo_b(id: &str, frames: usize, delay_ms: u64, extra: &str) -> (Vec<SndFrame>, Vec<bool>, usize) {
    let dir = repo().join("games");
    let manifest = parse_manifest(&std::fs::read_to_string(dir.join("games.toml")).expect("games.toml")).expect("parses");
    let game = manifest.game.iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}"));
    let data = std::fs::read(dir.join(&game.file)).expect("game file");
    let mut m = Machine::new();
    if game.file.ends_with(".tap") {
        m.tape_insert(&data).expect("inserts");
        m.tape_autostart();
    } else {
        m.load_sna(&data).expect("loads");
    }
    let mut km = KeyMapper::new();
    km.set_map(&encode_keymap(&game.keys).expect("key map"));
    let mut steps = parse_demo(game.demo.as_deref().expect("a demo"));
    steps.extend(parse_demo(extra));
    let engine = Engine::of_file(&game.file);
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut b = SndEngines::new();
    let (mut dec, mut eng, mut sound) = (Vec::new(), Vec::new(), 0);
    let mut level = 0;
    for k in 0..frames {
        let ms = (k as u64 * 20).wrapping_sub(delay_ms);
        let mut inp = KmInput::default();
        for s in steps.iter().filter(|s| s.at <= ms && ms < s.at + s.hold) {
            inp.buttons |= s.button;
            inp.notes |= s.note;
        }
        let (rows, kemp) = km.apply(&inp);
        m.set_keys(rows);
        m.set_kempston(kemp);
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        dec.push(b.frame(&m, &f.edges, f.start_level));
        eng.push(engine.is_some_and(|e| e.read(&m).is_some()));
        sound += usize::from(!f.edges.is_empty());
        level = f.edges.last().map_or(f.start_level, |e| e.level);
    }
    for _ in 0..SND_DELAY_FRAMES {
        dec.push(b.frame(&m, &[], level));
    }
    dec.drain(..SND_DELAY_FRAMES as usize);
    (dec, eng, sound)
}

/// Confident decisions of `dec` outside the music `eng` marks (its frames, the hold after them
/// and the frame before).
fn outside_music(dec: &[SndFrame], eng: &[bool]) -> Vec<usize> {
    let music = |i: usize| (i.saturating_sub(7)..=(i + 1).min(eng.len() - 1)).any(|j| eng[j]);
    (0..dec.len()).filter(|&i| dec[i].confident != 0 && !music(i)).collect()
}

#[test]
fn effects_not_b() {
    let shots: Vec<String> = (0..48).map(|k| format!("note.C5@{}+100", 7_000 + 600 * k)).collect();
    let shots = shots.join(", ");
    let runs = [
        ("dino", 1800, 0, ""),
        ("oneway", 1800, 0, "button.OCT+@9000, button.OCT+@9500, button.OCT+@11000"),
        ("sopwith", 1800, 0, shots.as_str()),
        ("castaway", 1800, 0, ""),
        ("colony8", 2050, 5_000, ""),
    ];
    let mut bad = Vec::new();
    for (id, frames, delay, extra) in runs {
        let (dec, eng, sound) = demo_b(id, frames, delay, extra);
        let claimed = dec.iter().filter(|d| d.confident != 0).count();
        let out = outside_music(&dec, &eng);
        println!("{id}: {sound} frames with edges, {} with the engine running, B claims {claimed}, outside the music {out:?}", eng.iter().filter(|&&e| e).count());
        if !out.is_empty() {
            bad.push(format!("{id}: {out:?}"));
        }
    }
    for (name, mut m, frames) in [("border.sna", test_sna("border.sna"), 3000), ("noise.sna", Machine::new(), 1000)] {
        if name == "noise.sna" {
            m.load_sna(&noise_sna()).expect("loads");
        }
        let r = run(m, frames, SndEngines::new(), None, no_edit);
        let claimed: Vec<usize> = (0..frames).filter(|&i| r.dec[i].confident != 0).collect();
        println!("{name}: B claims {claimed:?}");
        if !claimed.is_empty() {
            bad.push(format!("{name}: {claimed:?}"));
        }
    }
    assert!(bad.is_empty(), "B claims non-music sound:\n{}", bad.join("\n"));
}
