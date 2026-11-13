// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The short-pulse decoder (`core/sound/pulses.c`) and the style selector (`decoder.c`): a
//! generated pin-pulse tune (three voices, a short pulse per period, noise bursts) at the spike's
//! level, and the style reported in confident frames right on every synthetic style, the test
//! tune and the pack's music.

use music::score::score;
use music::synthetic::{frame_loop, frame_tune, pin_pulse, pin_tune, slots_loop, slots_tune, xor_loop, xor_tune, PIN_ROW_T};
use music::{capture, decode, decode_stream, played_hz, test_sna, RATE};
use zx_sys::{Beeper, SndFrame, SND_STYLE_FLIPS, SND_STYLE_PULSES, SND_STYLE_SLOTS};

#[test]
fn synthetic_pinpulse() {
    let tune = pin_tune();
    let s = pin_pulse(&tune, PIN_ROW_T, true);
    let dec = decode_stream(&s);
    let sc = score(&s.truth, &played_hz(&dec));
    let conf = dec.iter().filter(|f| f.confident != 0).count();
    let pulses = dec.iter().filter(|f| f.confident != 0 && f.style == SND_STYLE_PULSES).count();
    println!("pin-pulse tune ({} rows): {sc}", tune.len());
    println!("pin-pulse tune: {conf} confident frames, {pulses} of them PULSES");
    assert_eq!(tune.len(), 30);
    assert!(sc.precision() >= 0.94, "precision {:.1}%", 100.0 * sc.precision());
    assert!(sc.recall() >= 0.45, "recall {:.1}%", 100.0 * sc.recall());
    assert!(sc.wrong.len() <= 1, "{} wrong notes", sc.wrong.len());
    assert_eq!(pulses, conf, "a confident frame not PULSES");
}

/// The share of confident frames in `range` whose style is `want`: (right, confident).
fn style_share(dec: &[SndFrame], range: std::ops::Range<usize>, want: u8) -> (usize, usize) {
    let conf: Vec<&SndFrame> = dec[range].iter().filter(|f| f.confident != 0).collect();
    (conf.iter().filter(|f| f.style == want).count(), conf.len())
}

#[test]
fn selector_picks_right() {
    let game = |mut m: zx_sys::Machine, n: usize| {
        let fr = capture(&mut m, &mut Beeper::new(RATE.0, RATE.1), n);
        decode(fr.iter().map(|f| (f.edges.as_slice(), f.start_level)))
    };
    let cases: Vec<(&str, Vec<SndFrame>, std::ops::Range<usize>, u8)> = vec![
        ("xor tune", decode_stream(&xor_loop(&xor_tune(0))), 0..usize::MAX, SND_STYLE_FLIPS),
        ("xor tune with rests", decode_stream(&xor_loop(&xor_tune(4))), 0..usize::MAX, SND_STYLE_FLIPS),
        ("frame tune", decode_stream(&frame_loop(&frame_tune(40))), 0..usize::MAX, SND_STYLE_FLIPS),
        ("slots tune", decode_stream(&slots_loop(&slots_tune())), 0..usize::MAX, SND_STYLE_SLOTS),
        ("pin-pulse tune", decode_stream(&pin_pulse(&pin_tune(), PIN_ROW_T, true)), 0..usize::MAX, SND_STYLE_PULSES),
        ("pin-pulse tune, no noise", decode_stream(&pin_pulse(&pin_tune(), PIN_ROW_T, false)), 0..usize::MAX, SND_STYLE_PULSES),
        ("tune.sna", game(test_sna("tune.sna"), 450), 0..usize::MAX, SND_STYLE_FLIPS),
        ("castaway title", game(music::castaway(), 2150), 150..2150, SND_STYLE_SLOTS),
        ("colony8 music", game(music::colony8(), 1554), 125..1550, SND_STYLE_SLOTS),
    ];
    for (name, dec, range, want) in cases {
        let range = range.start..range.end.min(dec.len());
        let (right, conf) = style_share(&dec, range, want);
        println!("{name}: style right in {right} of {conf} confident frames");
        assert!(conf > 50, "{name}: the music plays in FM ({conf} confident frames)");
        assert!(right * 100 >= conf * 98, "{name}: style right in {right} of {conf} confident frames");
    }
}


#[test]
fn pinpulse_adjacent_semitones() {
    // two or three voices a semitone apart, from MIDI 48 to 95 (131 Hz to 2.5 kHz): none taken for
    // the other or for a pitch between them
    let mut rows = Vec::new();
    for m in [48u32, 55, 60, 67, 72, 79, 84, 88, 91, 94] {
        rows.push([m, m + 1, 0]);
        rows.push([m + 1, m + 5, m + 12]);
        rows.push([m, m + 1, m + 2]);
    }
    let s = pin_pulse(&rows, PIN_ROW_T, false);
    let sc = score(&s.truth, &played_hz(&decode_stream(&s)));
    println!("pin-pulse semitones: {sc}");
    assert!(sc.precision() >= 0.94, "precision {:.1}%", 100.0 * sc.precision());
    assert!(sc.wrong.is_empty(), "{} wrong notes", sc.wrong.len());
    assert!(sc.notes_found * 3 >= sc.true_notes, "{} of {} notes found", sc.notes_found, sc.true_notes);
}
