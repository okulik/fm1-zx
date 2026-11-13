// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The time-slot decoder (`core/sound/slots.c`) and the style selector (`decoder.c`): the pack's
//! music (Castaway's title tune, Escape from Colony 8's music) scored against the notes read from
//! the games' RAM (`truth.rs`), Colony 8's drums (which stop the music loop) with the voices back
//! after each, a generated three-voice time-slot stream, and the speaker-flip streams still
//! decoded as speaker flips.

use fmzx_pack::testprogs::TUNE_START_FRAME;
use music::score::{midi_of, score, Note, Score};
use music::synthetic::{
    frame_loop, frame_tune, hz_of_midi, slots_engine, slots_loop, slots_step, slots_tune, xor_loop, xor_tune, SlotsNote, Stream,
};
use music::truth::{Colony8Drums, Engine};
use music::{capture, decode, decode_stream, played_hz, test_sna, voices_hz, RATE};
use zx_sys::{Beeper, Machine, SndFrame, SND_STYLE_FLIPS, SND_STYLE_SLOTS};

/// A game run from boot: every frame's edges, the decoder's decisions and the RAM truth.
struct Run {
    dec: Vec<SndFrame>,
    /// Hz per frame (`None`: the engine was not running at the frame's end).
    truth: Vec<Option<Vec<f64>>>,
    /// (channel, Hz) per frame.
    chans: Vec<Option<Vec<(usize, f64)>>>,
    /// Speaker edge times (T-states within the frame) per frame.
    edges: Vec<Vec<u32>>,
    /// Frames a drum routine starts in (Colony 8).
    drum: Vec<bool>,
}

fn run(mut m: Machine, engine: Engine, frames: usize) -> Run {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut drums = Colony8Drums::new();
    let mut fr = Vec::new();
    let (mut truth, mut chans, mut edges, mut drum) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..frames {
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        let c = engine.read_channels(&m);
        truth.push(c.as_ref().map(|v| v.iter().map(|x| x.1).collect()));
        chans.push(c);
        drum.push(engine == Engine::Colony8 && drums.frame(&m));
        let mut level = (f.start_level >> 1) & 1;
        edges.push(
            f.edges
                .iter()
                .filter_map(|e| {
                    let l = (e.level >> 1) & 1;
                    (l != level).then(|| {
                        level = l;
                        e.tstate
                    })
                })
                .collect(),
        );
        fr.push(f);
    }
    let dec = decode(fr.iter().map(|f| (f.edges.as_slice(), f.start_level)));
    Run { dec, truth, chans, edges, drum }
}

/// Frames inside a pitch slide: a channel's truth pitch changes by a semitone or more at two of
/// the three frame steps around the frame (the engine's semitone-every-two-frames glides and fast
/// sweeps), and the frames either side of those.
fn slides(chans: &[Option<Vec<(usize, f64)>>]) -> Vec<bool> {
    let n = chans.len();
    let pitch = |i: usize, c: usize| chans[i].as_ref().and_then(|v| v.iter().find(|x| x.0 == c)).map(|x| midi_of(x.1));
    let change = |k: usize, c: usize| k > 0 && k < n && matches!((pitch(k - 1, c), pitch(k, c)), (Some(a), Some(b)) if (a - b).abs() >= 1.0);
    let mut core = vec![false; n];
    for (i, s) in core.iter_mut().enumerate() {
        *s = (0..3).any(|c| (i.saturating_sub(1)..=i + 1).filter(|&k| change(k, c)).count() >= 2);
    }
    (0..n).map(|i| (i.saturating_sub(1)..=(i + 1).min(n - 1)).any(|k| core[k])).collect()
}

/// Wrong notes that do not touch a slide.
fn wrong_outside(sc: &Score, slide: &[bool]) -> Vec<Note> {
    sc.wrong.iter().filter(|w| !(w.a.saturating_sub(1)..(w.b + 1).min(slide.len())).any(|i| slide[i])).cloned().collect()
}

#[test]
fn castaway_title() {
    let (a, b) = (150, 2150); // 3 s to 43 s
    let r = run(music::castaway(), Engine::Castaway, b);
    let sc = score(&r.truth[a..b], &played_hz(&r.dec[a..b]));
    let slide = slides(&r.chans[a..b]);
    let wrong = wrong_outside(&sc, &slide);
    let music: Vec<usize> = (a..b).filter(|&i| r.truth[i].as_ref().is_some_and(|v| !v.is_empty())).collect();
    let slots = music.iter().filter(|&&i| r.dec[i].style == SND_STYLE_SLOTS).count();
    // the engine has no drum routine (its kicks are fast pitch sweeps of a channel): the
    // sequencer's pauses between notes must not pass for drums
    let drums: Vec<usize> = (a..b).filter(|&i| r.dec[i].drum != 0 && r.dec[i].confident != 0).collect();
    println!("castaway: {sc}");
    println!(
        "castaway: {} wrong notes outside slides ({} slide frames); style SLOTS in {slots} of {} music frames; {} drums",
        wrong.len(),
        slide.iter().filter(|&&s| s).count(),
        music.len(),
        drums.len()
    );
    for w in &wrong {
        println!("  wrong outside slides: frames {}..{} MIDI {:.2}", w.a + a, w.b + a, w.midi);
    }
    assert!(sc.precision() >= 0.90, "precision {:.1}%", 100.0 * sc.precision());
    assert!(wrong.is_empty(), "{} wrong notes outside slides", wrong.len());
    assert!(slots * 10 >= music.len() * 9, "style SLOTS in {slots} of {} music frames", music.len());
    assert!(drums.is_empty(), "drums reported at frames {drums:?}");
}

#[test]
fn colony8_music() {
    let (a, b) = (125, 1550); // 2.5 s to 31 s, the title's music, no keys
    let r = run(music::colony8(), Engine::Colony8, b + 4);
    let sc = score(&r.truth[a..b], &played_hz(&r.dec[a..b]));
    println!("colony8: {sc}");
    assert!(sc.precision() >= 0.90, "precision {:.1}%", 100.0 * sc.precision());
    assert!(sc.wrong.len() <= 1, "{} wrong notes", sc.wrong.len());

    // drums: a drum routine stops the loop for about 23,000 T and drives the speaker itself;
    // the audible ones leave edges isolated from the loop's (more than 600 T from any other)
    let isolated = |f: usize| -> bool {
        let mut v: Vec<i64> = Vec::new();
        for k in 0..3 {
            if f + k >= 1 && f + k - 1 < r.edges.len() {
                v.extend(r.edges[f + k - 1].iter().map(|&t| t as i64 + (k as i64 - 1) * 69_888));
            }
        }
        let iso = (0..v.len())
            .filter(|&i| (i == 0 || v[i] - v[i - 1] > 600) && (i + 1 == v.len() || v[i + 1] - v[i] > 600) && (0..69_888).contains(&v[i]))
            .count();
        iso >= 2
    };
    // from the first confident decision on (the music recognised: a first drum before the first
    // pitch change cannot be told from other sound)
    let first = (a..b).find(|&i| r.dec[i].confident != 0).expect("the music plays in FM");
    let starts: Vec<usize> = (first..b).filter(|&f| r.drum[f]).collect();
    let mut audible = 0;
    let mut reported = 0;
    let mut resumed = 0;
    let mut need_voices = 0;
    for &f in &starts {
        let frames: Vec<usize> = (f.saturating_sub(1)..=f + 1).filter(|&k| isolated(k)).collect();
        if frames.is_empty() {
            continue; // a silent drum, or one without edges of its own
        }
        audible += 1;
        // the routine starts in frame f and runs for about a third of a frame
        let hit = (f..=f + 1).chain(frames.iter().copied()).any(|k| r.dec[k].drum != 0 && r.dec[k].confident != 0);
        if hit {
            reported += 1;
        } else {
            println!("  drum at frame {f} (edges in {frames:?}) not reported: {:?}", &r.dec[f - 1..=f + 1]);
        }
        // the voices come back: where the music goes on after the drum, a confident decision
        // with voices within 3 frames after it
        let last = *frames.last().unwrap();
        if (last + 1..=last + 3).any(|k| r.truth[k].as_ref().is_some_and(|v| !v.is_empty())) {
            need_voices += 1;
            if (last + 1..=last + 3).any(|k| r.dec[k].confident != 0 && !voices_hz(&r.dec[k]).is_empty()) {
                resumed += 1;
            } else {
                let conf: String = r.dec[last + 1..=last + 3].iter().map(|d| if d.confident != 0 { 'C' } else { '.' }).collect();
                println!("  drum at frame {f}: no voices in frames {}..={} ({conf})", last + 1, last + 3);
            }
        }
    }
    // and no drum where none runs
    let false_drums: Vec<usize> =
        (a..b).filter(|&i| r.dec[i].drum != 0 && r.dec[i].confident != 0 && !(i.saturating_sub(2)..=i + 1).any(|k| r.drum[k])).collect();
    println!(
        "colony8: {} drum starts from frame {first}, {audible} audible, {reported} reported; voices back within 3 frames after {resumed} of {need_voices}; \
         {} drums reported where none runs {false_drums:?}",
        starts.len(),
        false_drums.len()
    );
    assert!(audible >= 30, "audible drums in the excerpt ({audible})");
    assert_eq!(reported, audible, "every audible drum reported");
    assert_eq!(resumed, need_voices, "the voices resume after every drum");
    assert!(false_drums.is_empty(), "drums reported where none runs: {false_drums:?}");
}

#[test]
fn synthetic_slots_three_voices() {
    let s = slots_loop(&slots_tune());
    let dec = decode_stream(&s);
    let sc = score(&s.truth, &played_hz(&dec));
    println!("slots tune: {sc}");
    assert!(sc.true_notes >= 60, "the three voices' notes are in the truth ({})", sc.true_notes);
    assert!(sc.precision() >= 0.95, "precision {:.1}%", 100.0 * sc.precision());
    assert!(sc.wrong.is_empty(), "{} wrong notes", sc.wrong.len());
    let slots = dec.iter().filter(|f| f.style == SND_STYLE_SLOTS).count();
    println!("slots tune: style SLOTS in {slots} of {} frames", dec.len());
    assert!(slots * 10 >= dec.len() * 8);
}

#[test]
fn plain_tone_after_slots_music() {
    // time-slot music, then a plain tone (the whole speaker toggling, no loop carrier) at a new
    // pitch every frame: while the loop is still held (25 frames) the time-slot decoder must not
    // play it, though every channel moves with it in clean runs
    let mut s = slots_loop(&slots_tune());
    let n = s.frames.len();
    let mut level = s.frames.iter().flatten().last().map_or(s.start_level, |e| e.level);
    for k in 0..25u32 {
        let hp = 1_500 + 230 * (k % 7);
        s.frames.push(
            (1..69_800 / hp)
                .map(|i| {
                    level ^= 2;
                    zx_sys::zx_edge { tstate: i * hp, level }
                })
                .collect(),
        );
    }
    let dec = decode_stream(&s);
    let music = dec[..n].iter().filter(|f| f.confident != 0).count();
    let slots = dec[n..].iter().filter(|f| f.style == SND_STYLE_SLOTS).count();
    let played: Vec<usize> = (n..dec.len()).filter(|&i| dec[i].confident != 0 && !voices_hz(&dec[i]).is_empty()).map(|i| i - n).collect();
    println!("music: {music} of {n} frames confident; tone: {slots} of 25 frames time slots, played in {played:?}");
    assert!(music > n / 2, "the music before it plays in FM");
    assert_eq!(slots, 25, "the loop is held through the tone");
    assert!(played.is_empty(), "the tone played as notes in frames {played:?}");
}

/// `a` then `b`, with `gap` silent frames between.
fn join(a: Stream, gap: usize, b: Stream) -> (Stream, usize) {
    let mut a = a;
    a.frames.extend((0..gap).map(|_| Vec::new()));
    a.truth.extend((0..gap).map(|_| Some(Vec::new())));
    a.drum.extend((0..gap).map(|_| false));
    a.silent.extend((0..gap).map(|_| true));
    let n = a.frames.len();
    (a.then(b), n)
}

#[test]
fn effect_after_slots_music_stays_beeper() {
    // time-slot music, then within a second an effect with no pitch change of its own: another
    // time-slot loop (110 T, a steady two-channel chord) or a plain square tone (speaker flips);
    // the music's pitch changes must not carry over to it
    let tdm_effect = slots_engine(
        &[SlotsNote { step: [slots_step(600.0), slots_step(900.0), 0], thr: [0x80, 0x80, 0], frames: 50 }],
        110,
        [0, 30, 70],
    );
    let tone = {
        let mut level = 0u8;
        let frames: Vec<Vec<zx_sys::zx_edge>> = (0..50)
            .map(|_| {
                (1..69_888u32 / 2_917)
                    .map(|i| {
                        level ^= 2;
                        zx_sys::zx_edge { tstate: i * 2_917, level }
                    })
                    .collect()
            })
            .collect();
        let n = frames.len();
        Stream { frames, start_level: 0, truth: vec![Some(vec![]); n], drum: vec![false; n], silent: vec![false; n] }
    };
    // and speaker-flip music, a short time-slot tune, then the plain tone: the time slots end the
    // speaker-flip music's context too (one thin channel: under 256 edges a frame, so the
    // time-slot frames are not too busy for speaker flips)
    let thin: Vec<SlotsNote> = [60.0, 64.0, 67.0, 64.0]
        .iter()
        .map(|&m| SlotsNote { step: [0, slots_step(hz_of_midi(m)), 0], thr: [0, 0x40, 0], frames: 6 })
        .collect();
    let mixed = frame_loop(&frame_tune(8)).then(slots_loop(&thin));
    for (name, before, effect) in [
        ("time-slot effect", slots_loop(&slots_tune()), tdm_effect),
        ("plain tone", slots_loop(&slots_tune()), tone.clone()),
        ("plain tone after both styles", mixed, tone),
    ] {
        for gap in [0, 20] {
            let (s, n) = join(before.clone(), gap, effect.clone());
            let dec = decode_stream(&s);
            let music = dec[..n - gap].iter().filter(|f| f.confident != 0).count();
            let conf: Vec<usize> = (n..dec.len()).filter(|&i| dec[i].confident != 0).map(|i| i - n).collect();
            println!("{name} after {gap} silent frames: music {music} of {} confident; effect confident in {conf:?}", n - gap);
            assert!(music > (n - gap) / 2, "{name}: the music before it plays in FM");
            assert!(conf.is_empty(), "{name} after {gap} silent frames: FM in effect frames {conf:?}");
        }
    }
}

#[test]
fn synthetic_slots_adjacent_semitones() {
    // pairs of notes a semitone apart from 1 kHz up to the decoder's 2.5 kHz top, on the
    // time-slot engine (16-bit accumulators: a period of 9-22 loops, so single periods jitter by
    // a whole loop): each note within half a semitone, none taken for the other or between them
    let mut notes = vec![
        SlotsNote { step: [0, slots_step(hz_of_midi(60.0)), 0], thr: [0, 0x80, 0], frames: 6 },
        SlotsNote { step: [0, slots_step(hz_of_midi(64.0)), 0], thr: [0, 0x80, 0], frames: 6 },
    ];
    let lead = notes.len();
    for m in [84.0, 86.0, 88.0, 90.0, 92.0, 94.0, 96.0, 97.5] {
        for (k, mm) in [m, m + 1.0, m, m + 1.0].into_iter().enumerate() {
            let c = k % 2 * 2; // the 0.22 and 0.46 slots in turn
            let mut step = [0u16; 3];
            let mut thr = [0u8; 3];
            step[c] = slots_step(hz_of_midi(mm));
            thr[c] = 0x80;
            notes.push(SlotsNote { step, thr, frames: 5 });
        }
    }
    let s = slots_loop(&notes);
    let dec = decode_stream(&s);
    // from the first pair on (the lead-in gives the music context)
    let a = (lead * 6) as usize + 1;
    let sc = score(&s.truth[a..], &played_hz(&dec[a..]));
    println!("adjacent semitones: {sc}");
    assert!(sc.wrong.is_empty(), "{} wrong notes", sc.wrong.len());
    assert_eq!(sc.notes_found, sc.true_notes, "every note decoded");
}

#[test]
fn flips_still_pass() {
    // the speaker-flip streams have no loop carrier: the selector never picks SLOTS for them, so
    // Task 2's decisions (tests/flips.rs) stand
    let sna = |name: &str, n: usize| {
        let fr = capture(&mut test_sna(name), &mut Beeper::new(RATE.0, RATE.1), n);
        decode(fr.iter().map(|f| (f.edges.as_slice(), f.start_level)))
    };
    let streams = [
        ("xor tune", decode_stream(&xor_loop(&xor_tune(0)))),
        ("xor tune with rests", decode_stream(&xor_loop(&xor_tune(4)))),
        ("frame tune", decode_stream(&frame_loop(&frame_tune(40)))),
        ("tune.sna", sna("tune.sna", 450)),
        ("border.sna", sna("border.sna", 500)),
    ];
    for (name, dec) in &streams {
        let slots = dec.iter().filter(|f| f.style == SND_STYLE_SLOTS).count();
        let flips = dec.iter().filter(|f| f.confident != 0 && f.style == SND_STYLE_FLIPS).count();
        let conf = dec.iter().filter(|f| f.confident != 0).count();
        println!("{name}: {conf} confident frames, all FLIPS: {}; SLOTS in {slots}", flips == conf);
        assert_eq!(slots, 0, "{name}: SLOTS chosen");
        assert_eq!(flips, conf, "{name}: a confident frame not FLIPS");
    }
    let tune = &streams[3].1;
    assert!(tune[TUNE_START_FRAME as usize + 40].confident != 0, "tune.sna still plays in FM");
}
