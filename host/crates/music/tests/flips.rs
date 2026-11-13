// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The speaker-flip decoder (`core/sound/flips.c`, `decoder.c`): the test tune's notes exactly,
//! generated XOR-mixed music in both note timings, rests left silent, drums not read as chords,
//! and a steady test tone never taken for music.

use fmzx_pack::testprogs::{BORDER_TONE_HZ, TUNE_HZ, TUNE_NOTE_FRAMES, TUNE_START_FRAME};
use music::score::{score, Score};
use music::synthetic::{frame_loop, frame_tune, xor_loop, xor_tune, Stream};
use music::{capture, decode, decode_stream, played_hz, test_sna, voices_hz, RATE};
use zx_sys::{Beeper, SndFrame, SND_STYLE_FLIPS};

/// The decisions about the first `frames` frames of `games/test/<name>`.
fn decide_sna(name: &str, frames: usize) -> Vec<SndFrame> {
    let fr = capture(&mut test_sna(name), &mut Beeper::new(RATE.0, RATE.1), frames);
    decode(fr.iter().map(|f| (f.edges.as_slice(), f.start_level)))
}

fn scored(s: &Stream, dec: &[SndFrame], range: std::ops::Range<usize>) -> Score {
    score(&s.truth[range.clone()], &played_hz(&dec[range]))
}

/// Frame-level gates and the wrong-note limit for one part of a stream.
fn gate(what: &str, sc: &Score, precision: f64, recall: f64, wrong: usize) {
    println!("{what}: {sc}");
    assert!(sc.precision() >= precision, "{what}: precision {:.1}%", 100.0 * sc.precision());
    assert!(sc.recall() >= recall, "{what}: recall {:.1}%", 100.0 * sc.recall());
    assert!(sc.wrong.len() <= wrong, "{what}: {} wrong notes", sc.wrong.len());
}

/// Drum frames carry no tonal voice in a confident decision; returns how many report a drum.
fn drums_not_chords(what: &str, s: &Stream, dec: &[SndFrame], from: usize) -> usize {
    let mut reported = 0;
    let mut total = 0;
    for (i, fr) in dec.iter().enumerate().skip(from) {
        if !s.drum[i] {
            continue;
        }
        total += 1;
        if fr.confident != 0 {
            assert!(voices_hz(fr).is_empty(), "{what}: drum frame {i} played as {:?}", voices_hz(fr));
        }
        if fr.drum != 0 {
            assert!(voices_hz(fr).is_empty(), "{what}: drum frame {i} reports a drum and {:?}", voices_hz(fr));
            reported += 1;
        }
    }
    println!("{what}: {reported} of {total} drum frames report a drum");
    reported
}

#[test]
fn tune_exact() {
    let dec = decide_sna("tune.sna", 450);
    // frame f (1-based) is dec[f - 1]; note k starts at frame TUNE_START_FRAME + 25k. The first
    // note is not checked: no pitch change has been heard yet, and a lone tone is not music until
    // one has (steady_tone_not_music).
    let len = TUNE_NOTE_FRAMES as usize;
    let mut checked = 0;
    for k in 1.. {
        let first = TUNE_START_FRAME as usize + k * len;
        if first + len > dec.len() {
            break;
        }
        let want = TUNE_HZ[k % TUNE_HZ.len()];
        for f in first + 2..first + len - 2 {
            let fr = &dec[f - 1];
            let v = voices_hz(fr);
            assert!(fr.confident != 0, "note {k}, frame {f}: not confident ({fr:?})");
            assert_eq!(fr.style, SND_STYLE_FLIPS, "note {k}, frame {f}: style");
            assert_eq!(v.len(), 1, "note {k}, frame {f}: voices {v:?}");
            let err = (v[0] - want).abs() / want;
            assert!(err <= 0.005, "note {k}, frame {f}: {:.2} Hz, want {want} ({:.3}%)", v[0], 100.0 * err);
            checked += 1;
        }
        let v = voices_hz(&dec[first + len / 2]);
        println!("note {k}: {:.2} Hz (recorded {want})", v[0]);
    }
    let first_note = (TUNE_START_FRAME as usize - 1..TUNE_START_FRAME as usize - 1 + len).filter(|&i| dec[i].confident != 0).count();
    println!("{checked} frames checked; the first note has {first_note} confident frames of {len}");
    assert!(checked >= 15 * (len - 4));
}

#[test]
fn synthetic_xor_two_voices() {
    let s = xor_loop(&xor_tune(0));
    let dec = decode_stream(&s);
    let sc = scored(&s, &dec, 0..dec.len());
    gate("xor tune", &sc, 0.95, 0.90, 0);
    assert!(sc.true_notes >= 40, "both counters' notes are in the truth ({})", sc.true_notes);
}

#[test]
fn rest_not_filled() {
    let s = xor_loop(&xor_tune(4));
    let dec = decode_stream(&s);
    let sc = scored(&s, &dec, 0..dec.len());
    gate("xor tune with rests", &sc, 0.95, 0.85, 0);
    // every frame wholly inside a rest reports no voice (the decision about frame t is made with
    // frames t + 1..t + 3 in view, so a rest is seen as soon as it starts)
    let mut rests = 0;
    let mut i = 0;
    while i < dec.len() {
        if !s.silent[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < dec.len() && s.silent[i] {
            assert!(voices_hz(&dec[i]).is_empty(), "rest frame {i}: voices {:?}", voices_hz(&dec[i]));
            i += 1;
        }
        if i >= dec.len() || a < 20 {
            continue; // the stream's end, or before the music was recognised
        }
        rests += 1;
        // through the rest and its edges: the decision drops to the beeper at most once
        let w = &dec[a.saturating_sub(2)..(i + 2).min(dec.len())];
        let drops = w.windows(2).filter(|p| p[0].confident != 0 && p[1].confident == 0).count();
        let conf: String = w.iter().map(|f| if f.confident != 0 { 'C' } else { '.' }).collect();
        assert!(drops <= 1, "rest at frames {a}..{i}: decisions {conf}");
    }
    println!("{rests} rests checked");
    assert!(rests >= 15);
}

#[test]
fn synthetic_frame_notes() {
    let s = frame_loop(&frame_tune(40));
    let dec = decode_stream(&s);
    let sc = scored(&s, &dec, 0..dec.len());
    gate("frame tune", &sc, 0.95, 0.85, 0);
    drums_not_chords("frame tune", &s, &dec, 0);
}

#[test]
fn timing_adapts() {
    let a = xor_loop(&xor_tune(0));
    let b = frame_loop(&frame_tune(40));
    let (n, m) = (a.frames.len(), a.frames.len() + b.frames.len());
    let s = a.then(b).then(xor_loop(&xor_tune(0)));
    let dec = decode_stream(&s);
    let long = scored(&s, &dec, 0..n);
    gate("long notes", &long, 0.95, 0.90, 1);
    assert!(long.splits <= 1, "long notes: {} split", long.splits);
    let frame = scored(&s, &dec, n..m);
    gate("frame notes", &frame, 0.95, 0.85, 0);
    drums_not_chords("frame notes", &s, &dec, n);
    // and back to long notes
    let back = scored(&s, &dec, m..dec.len());
    gate("long notes again", &back, 0.95, 0.85, 1);
    assert!(back.splits <= 1, "long notes again: {} split", back.splits);
}

#[test]
fn steady_tone_not_music() {
    let dec = decide_sna("border.sna", 500);
    let heard = dec.iter().filter(|f| voices_hz(f).iter().any(|&h| (h - BORDER_TONE_HZ).abs() < BORDER_TONE_HZ * 0.01)).count();
    let confident = dec.iter().filter(|f| f.confident != 0).count();
    println!("border tone: decoded in {heard} of {} frames, confident in {confident}", dec.len());
    assert!(heard >= dec.len() * 9 / 10, "the decoder hears the tone ({heard} frames)");
    assert_eq!(confident, 0, "a steady tone is not music");
}
