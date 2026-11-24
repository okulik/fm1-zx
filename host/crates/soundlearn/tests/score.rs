// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recorded scores (kind-2 records): the encoder against the decoder's checks
//! (`core/sound/learned.c`), and method B's player on synthetic tunes recorded on the host: it
//! claims the tune's frames and plays the score's events at their T-states whatever the tune's
//! phase in the frame, stops within the delay when the tune changes (the beeper returns), and
//! leaves a frame a table entry claims to the table; of several scores at one site, each tune
//! plays its own; a tune that repeats a phrase forever is followed round the score's loop pass
//! after pass, until it changes. A score's drum events (voice 3) reach method B's drum voice at
//! their T-states; the list's scores read back ([`decode_scores`]), are replaced
//! ([`replace_scores`]) and named ([`score_key`]).

mod fixtures;

use fixtures::{Asm, Labels};
use soundlearn::entry::{ear_edges, score_code, score_fingerprint, score_level};
use soundlearn::{
    decode_scores, encode_list, encode_list_scores, encode_list_with, replace_scores, score_key, BuiltinRoles, LearnedEntry, Op, ScoreEvent, ScoreRecord, SCORE_DRUM,
};
use zx_sys::{
    zx_edge, Machine, SndEngines, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_MODE_FM_B, SND_ROLE_AUTO, SND_ROLE_BASS, SND_ROLE_LEAD, SND_SCORES_MAX,
    SND_SCORE_BUCKETS_MAX, SND_SITE_TONAL, ZX_FRAME_TSTATES,
};

const F: u64 = ZX_FRAME_TSTATES as u64;
const F32: u32 = ZX_FRAME_TSTATES;

// ---- the decoder ---------------------------------------------------------

fn on(t: u32, voice: u8, period: u32, level: u8) -> ScoreEvent {
    ScoreEvent { t, voice, on: Some((period, level)) }
}

fn off(t: u32, voice: u8) -> ScoreEvent {
    ScoreEvent { t, voice, on: None }
}

/// A drum hit (`level` 1-255).
fn hit(t: u32, level: u8) -> ScoreEvent {
    ScoreEvent { t, voice: SCORE_DRUM, on: Some((0, level)) }
}

/// A valid score with no loop: two voices, three buckets, four events.
fn score() -> ScoreRecord {
    ScoreRecord {
        site: 0x8123,
        quiet: 2,
        code: 0xDEAD_BEEF,
        voices: 2,
        roles: [SND_ROLE_BASS, SND_ROLE_LEAD, 0],
        intro_t: 3 * F32,
        loop_t: 0,
        intro_fp: vec![40, 41, 39],
        loop_fp: vec![],
        intro_events: vec![on(0, 0, 2716, 200), on(640, 1, 64, 1), off(64 * 210, 0), off(64 * 2210, 1)],
        loop_events: vec![],
    }
}

/// A valid score with a loop: an intro of two and a half buckets, a loop of two and a bit.
fn looped() -> ScoreRecord {
    ScoreRecord {
        intro_t: 5 * F32 / 2,
        loop_t: 2 * F32 + 100,
        intro_fp: vec![40, 52],
        loop_fp: vec![30, 31],
        intro_events: vec![on(0, 0, 2716, 200), on(F32, 1, 5000, 90)],
        loop_events: vec![on(0, 0, 3000, 200), off(100, 1), on(2 * F32 + 50, 1, 6000, 90)],
        ..score()
    }
}

fn list(scores: &[ScoreRecord]) -> Vec<u8> {
    encode_list_scores(&[], &[], scores)
}

/// Does the device's decoder take the list? [`decode_scores`] (which checks the list's structure
/// and its scores, not its learned entries) agrees.
fn decodes(l: &[u8]) -> bool {
    let ok = SndEngines::with_learned(l).is_ok();
    assert_eq!(decode_scores(l).is_ok(), ok, "decode_scores against the decoder: {:?}", decode_scores(l).err());
    ok
}

/// The list of `s` with bytes from `at` (in the record) replaced.
fn patched_of(s: &ScoreRecord, at: usize, bytes: &[u8]) -> Vec<u8> {
    let mut l = list(std::slice::from_ref(s));
    l[4 + at..4 + at + bytes.len()].copy_from_slice(bytes);
    l
}

/// [`patched_of`] [`score`].
fn patched(at: usize, bytes: &[u8]) -> Vec<u8> {
    patched_of(&score(), at, bytes)
}

/// The fixed part's fields of a raw record.
#[derive(Clone, Copy)]
struct Head {
    voices: u8,
    intro_t: u32,
    loop_t: u32,
    buckets: (u16, u16),
    events: (u16, u16),
    pitches: u8,
}

/// One voice, a one-bucket intro, no loop, one event, one pitch.
const HEAD: Head = Head { voices: 1, intro_t: F32, loop_t: 0, buckets: (1, 0), events: (1, 0), pitches: 1 };

/// The list of one raw record: `h`, its pitches each 100 T, then `tail` (the fingerprint and
/// events' bytes), its size fitted.
fn raw(h: Head, tail: &[u8]) -> Vec<u8> {
    let mut r = vec![0, 0, 2, h.voices, 0x23, 0x81, 0, 0, 0, 0, 0, 0];
    r.extend(h.intro_t.to_le_bytes());
    r.extend(h.loop_t.to_le_bytes());
    for w in [h.buckets.0, h.buckets.1, h.events.0, h.events.1] {
        r.extend(w.to_le_bytes());
    }
    r.extend([h.pitches, 0, 0, 0]);
    for _ in 0..h.pitches {
        r.extend([100, 0, 0]);
    }
    r.extend(tail);
    r.resize(r.len().next_multiple_of(4), 0);
    let size = r.len() as u16;
    r[..2].copy_from_slice(&size.to_le_bytes());
    let mut l = vec![1, 0, 0, 0];
    l.extend(r);
    l
}

/// A one-bucket fingerprint of 7 edges (escape, 7, 0) and its spare nibble.
const FP1: [u8; 2] = [0x78, 0x00];

/// A score's record parsed back as the format describes it (docs/pack-format.md): the
/// fingerprints, and each section's events as (64-T unit from its start, voice, level 0-63, the
/// pitch's period; a drum event's 0).
#[allow(clippy::type_complexity)]
fn parse(l: &[u8]) -> ([Vec<u8>; 2], [Vec<(u32, u8, u8, u32)>; 2]) {
    let r = &l[4..];
    let w = |at: usize| usize::from(u16::from_le_bytes([r[at], r[at + 1]]));
    let np = usize::from(r[28]);
    let pitch = |i: usize| u32::from_le_bytes([r[32 + 3 * i], r[33 + 3 * i], r[34 + 3 * i], 0]);
    let fp0 = 32 + 3 * np;
    let nib = |i: usize| (r[fp0 + i / 2] >> (4 * (i % 2))) & 15;
    let mut i = 0;
    let mut fps = [Vec::new(), Vec::new()];
    for (s, fp) in fps.iter_mut().enumerate() {
        let mut prev = 0i32;
        for k in 0..w(20 + 2 * s) {
            let d = nib(i);
            i += 1;
            prev = if d == 8 {
                i += 2;
                i32::from(nib(i - 2) | nib(i - 1) << 4)
            } else {
                assert!(k > 0, "a section's first bucket is an escape");
                prev + i32::from(d) - if d & 8 != 0 { 16 } else { 0 }
            };
            fp.push(u8::try_from(prev).expect("a count 0-255"));
        }
    }
    let mut at = fp0 + i.div_ceil(2);
    let mut evs = [Vec::new(), Vec::new()];
    for (s, ev) in evs.iter_mut().enumerate() {
        let mut u = 0;
        for _ in 0..w(24 + 2 * s) {
            let (mut dt, mut sh) = (0u32, 0);
            loop {
                let c = r[at];
                at += 1;
                dt |= u32::from(c & 0x7F) << sh;
                sh += 7;
                if c & 0x80 == 0 {
                    break;
                }
            }
            u += dt;
            let b = r[at];
            at += 1;
            let p = if b >> 2 != 0 && b & 3 != SCORE_DRUM {
                at += 1;
                pitch(usize::from(r[at - 1]))
            } else {
                0
            };
            ev.push((u, b & 3, b >> 2, p));
        }
    }
    (fps, evs)
}

#[test]
fn encoder_writes_the_record() {
    let l = list(&[score()]);
    assert_eq!(&l[..4], [1, 0, 0, 0]);
    let r = &l[4..];
    let size = usize::from(u16::from_le_bytes([r[0], r[1]]));
    assert_eq!((size, r[2], r[3]), (l.len() - 4, 2, 2));
    assert_eq!(size % 4, 0);
    #[rustfmt::skip]
    let head = [0x23, 0x81, 2, 0, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x33, 0x03, 0x00, 0, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 2, SND_ROLE_BASS, SND_ROLE_LEAD, 0];
    assert_eq!(&r[4..32], head, "3 x 69,888 = 33300h T, no loop; 3 + 0 buckets, 4 + 0 events, 2 pitches");
    // the pitches ascending: 64, 2716 (A9Ch)
    assert_eq!(&r[32..38], [64, 0, 0, 0x9C, 0x0A, 0]);
    // the fingerprint: escape 40 (8, 8, 2), +1, -2 (Eh), the spare nibble 0
    assert_eq!(&r[38..41], [0x88, 0x12, 0x0E]);
    // events: dt 0, voice 0 on, level 200 as 50 (C8h), pitch 1; dt 10, voice 1 on, level 1 as 1,
    // pitch 0; dt 200 (two bytes), voice 0 off; dt 2000, voice 1 off
    let ev = [0, 0xC8, 1, 10, 0x05, 0, 0xC8, 0x01, 0x00, 0xD0, 0x0F, 0x01];
    assert_eq!(&r[41..41 + ev.len()], ev);
    assert!(r[41 + ev.len()..].iter().all(|&b| b == 0), "zero padding");
    assert_eq!(size, 56);

    // the times are rounded from the section's start, so the rounding does not add up; an event at
    // the section's end goes to its last unit
    let s = ScoreRecord { intro_events: vec![off(40, 0), off(80, 0), off(120, 0), off(160, 0), off(3 * F32 - 10, 0)], voices: 1, ..score() };
    let (_, ev) = parse(&list(&[s]));
    let units: Vec<u32> = ev[0].iter().map(|e| e.0).collect();
    assert_eq!(units, [1, 1, 2, 3, 3 * F32 / 64 - 1], "40, 80, 120, 160 T: 1, 1, 2, 3 units from T0");

    // the loop's fields and its sections, read back
    let l = list(&[looped()]);
    let r = &l[4..];
    assert_eq!(&r[12..28], [0x80, 0xAA, 0x02, 0, 0x64, 0x22, 0x02, 0, 2, 0, 2, 0, 2, 0, 3, 0]);
    let (fp, ev) = parse(&l);
    assert_eq!(fp, [vec![40, 52], vec![30, 31]]);
    let l50 = |level: u8| (u32::from(level) + 2) / 4;
    assert_eq!(ev[0], [(0, 0, l50(200) as u8, 2716), (F32 / 64, 1, l50(90) as u8, 5000)]);
    assert_eq!(ev[1], [(0, 0, 50, 3000), (2, 1, 0, 0), ((2 * F32 + 50 + 32) / 64, 1, l50(90) as u8, 6000)]);
    assert!(decodes(&l));

    // levels: (level + 2) / 4 of 1-63, played as that x 4 + 3
    assert_eq!([1, 2, 5, 6, 200, 253, 255].map(score_level), [7, 7, 7, 11, 203, 255, 255]);

    // without scores, the lists are as encode_list_with writes them
    let e = LearnedEntry {
        digest: 1,
        len: 4,
        wild: vec![],
        reloc: vec![],
        sites: vec![(0, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        observed: false,
        drum: 0,
        stale: None,
        level: vec![200],
        instrument: [0; 3],
        progs: vec![vec![Op::K(300)]],
        timing: None,
    };
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    assert_eq!(encode_list_scores(std::slice::from_ref(&e), &[roles], &[]), encode_list_with(std::slice::from_ref(&e), &[roles]));
    // and the scores come last, counted
    let l = encode_list_scores(std::slice::from_ref(&e), &[roles], &[score()]);
    let head = encode_list_with(&[e], &[roles]);
    assert_eq!(l[0], 3);
    assert_eq!(&l[1..head.len()], &head[1..]);
    assert_eq!(&l[head.len()..], &list(&[score()])[4..]);
    let b = SndEngines::with_learned(&l).expect("decodes");
    assert_eq!(b.scores(), 1);
}

#[test]
fn fingerprints_round_trip() {
    // escapes at each section's start, the largest changes a nibble holds (+7, -7), changes past
    // them both ways, the counts' extremes
    let intro = vec![0, 7, 0, 255, 248, 255, 0, 8, 1, 9, 2, 200, 193, 200, 200, 0];
    let lp = vec![5, 5, 12, 255, 0, 0, 7];
    let s = ScoreRecord { intro_t: intro.len() as u32 * F32 + 7, loop_t: lp.len() as u32 * F32 + 3, intro_fp: intro.clone(), loop_fp: lp.clone(), ..looped() };
    let l = list(std::slice::from_ref(&s));
    assert!(decodes(&l));
    assert_eq!(parse(&l).0, [intro.clone(), lp.clone()]);
    // a nibble a bucket but the escapes (three each): 16 + 7 buckets, 2 + 2 + 5 + 1 escapes...
    let nibbles = |fp: &[u8]| fp.len() + 2 * (1 + fp.windows(2).filter(|w| w[0].abs_diff(w[1]) > 7).count());
    let fp_bytes = (nibbles(&intro) + nibbles(&lp)).div_ceil(2);
    let r = &l[4..];
    let events_at = 32 + 3 * usize::from(r[28]) + fp_bytes;
    assert_eq!(r[events_at..events_at + 2], [0, 0xC8], "the events start after the fingerprint's last byte");
    // random fingerprints, odd and even lengths
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..200 {
        let (na, nb) = (1 + (next() % 40) as usize, (next() % 9) as usize);
        let mut fp = |n: usize| {
            let mut c = (next() % 256) as i32;
            (0..n)
                .map(|_| {
                    c = match next() % 4 {
                        0 => (next() % 256) as i32,
                        _ => (c + (next() % 15) as i32 - 7).clamp(0, 255),
                    };
                    c as u8
                })
                .collect::<Vec<u8>>()
        };
        let (a, b) = (fp(na), fp(nb));
        let loop_t = if b.is_empty() { 0 } else { b.len() as u32 * F32 + 9 };
        let loop_events = if b.is_empty() { vec![] } else { looped().loop_events };
        let s = ScoreRecord { intro_t: a.len() as u32 * F32 + 11, loop_t, intro_fp: a.clone(), loop_fp: b.clone(), loop_events, ..looped() };
        let l = list(&[s]);
        assert!(decodes(&l), "{a:?} {b:?}");
        assert_eq!(parse(&l).0, [a, b]);
    }
}

#[test]
fn encoder_builds_the_pitch_table() {
    // 300 periods, each a step of about 1.2 cents: merged to 255 by cents, every note within a
    // few cents of its own
    let periods: Vec<u32> = (0..300).map(|i| (100_000.0 * 2f64.powf(f64::from(i) * 0.001)) as u32).collect();
    let events: Vec<ScoreEvent> = periods.iter().enumerate().map(|(i, &p)| on(i as u32 * 64, 0, p, 100)).collect();
    let s = ScoreRecord { voices: 1, intro_t: 10 * F32, intro_fp: vec![9; 10], intro_events: events, ..score() };
    let l = list(&[s]);
    assert!(decodes(&l));
    assert_eq!(l[4 + 28], 255);
    let (_, ev) = parse(&l);
    for (e, &p) in ev[0].iter().zip(&periods) {
        let cents = 1200.0 * (f64::from(e.3) / f64::from(p)).log2().abs();
        assert!(cents < 5.0, "{p} played as {}", e.3);
    }
    // a score with no note on still has a pitch; one note's period exact
    let s = ScoreRecord { intro_events: vec![off(0, 0)], ..score() };
    let l = list(&[s]);
    assert_eq!((l[4 + 28], decodes(&l)), (1, true));
    assert_eq!(parse(&list(&[score()])).1[0][0].3, 2716);
}

#[test]
fn decoder_takes_good_scores() {
    let b = SndEngines::with_learned(&list(&[score()])).expect("decodes");
    let n = SndEngines::new().entries().len();
    assert_eq!((b.entries().len(), b.scores()), (n, 1), "a score is no table entry");
    assert_eq!(b.name(n as u8 + 1), "score 1");
    assert_eq!(b.name(n as u8 + 2), "");
    let eight = SndEngines::with_learned(&list(&vec![score(); SND_SCORES_MAX])).expect("eight scores");
    assert_eq!((eight.scores(), eight.name(n as u8 + 8).as_str()), (8, "score 8"));
    assert_eq!(eight.name(n as u8 + 9), "");
    let ok = |s: ScoreRecord, what: &str| assert!(decodes(&list(std::slice::from_ref(&s))), "{what}: {s:?}");
    // with a loop; a loop from T0 (no intro); an intro under a bucket (none whole) before a loop
    ok(looped(), "a loop");
    ok(ScoreRecord { intro_t: 0, intro_fp: vec![], intro_events: vec![], ..looped() }, "no intro");
    ok(ScoreRecord { intro_t: 100, intro_fp: vec![], intro_events: vec![on(0, 0, 100, 9)], ..looped() }, "a short intro");
    // no events; one bucket; under a bucket with no loop; the most buckets, in one section or two
    ok(ScoreRecord { intro_events: vec![], ..score() }, "no events");
    ok(ScoreRecord { intro_t: F32, intro_fp: vec![9], intro_events: vec![], ..score() }, "one bucket");
    ok(ScoreRecord { intro_t: F32 - 1, intro_fp: vec![], intro_events: vec![on(F32 - 64, 0, 64, 1)], ..score() }, "under a bucket");
    let max = SND_SCORE_BUCKETS_MAX as u32;
    ok(ScoreRecord { intro_t: max * F32 + F32 - 1, intro_fp: vec![3; max as usize], ..score() }, "the most buckets");
    ok(ScoreRecord { intro_t: 9 * F32, intro_fp: vec![3; 9], loop_t: (max - 9) * F32, loop_fp: vec![4; max as usize - 9], ..looped() }, "the most in two");
    ok(ScoreRecord { voices: 3, intro_events: vec![on(0, 2, 0xFF_FFFF, 255)], ..score() }, "three voices");
    // the last unit of each section
    ok(ScoreRecord { intro_events: vec![off(3 * F32 - 64, 0)], ..score() }, "the intro's last unit");
    let lp = looped();
    ok(ScoreRecord { loop_events: vec![off(lp.loop_t - 33, 1)], ..lp }, "the loop's last unit");
    assert!(decodes(&raw(HEAD, &[0x78, 0x00, 0xC3, 0x08, 0x00])), "unit 1091 of one bucket");
    // a varint of four bytes; a padded record; roles above 3 (read as none)
    assert!(decodes(&raw(Head { events: (2, 0), ..HEAD }, &[0x78, 0x00, 0x80, 0x80, 0x80, 0x00, 0x00, 0x01, 0x00])));
    let mut padded = list(&[score()]);
    padded.extend([0; 8]);
    let size = u16::from_le_bytes([padded[4], padded[5]]) + 8;
    padded[4..6].copy_from_slice(&size.to_le_bytes());
    assert!(decodes(&padded));
    assert!(decodes(&patched(29, &[9, 200, 4])));
    // 8 entries of kinds 0 and 1 and 8 scores: 16 records
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    assert!(decodes(&encode_list_scores(&[], &[roles; 8], &vec![looped(); 8])));
    assert!(!decodes(&encode_list_scores(&[], &[roles; 9], &[score()])), "9 of kinds 0 and 1");
}

#[test]
fn decoder_refuses_bad_scores() {
    let refused = |l: Vec<u8>, what: &str| assert!(!decodes(&l), "{what}: {l:02X?}");
    let bad = |s: ScoreRecord, what: &str| refused(list(&[s]), what);
    refused(list(&vec![score(); SND_SCORES_MAX + 1]), "nine scores");
    refused(patched(2, &[3]), "kind 3");
    refused(patched(3, &[0]), "no voice");
    refused(patched(3, &[4]), "four voices");
    refused(patched(0, &[28, 0]), "a size under the fixed part");
    let l = list(&[score()]);
    let size = u16::from_le_bytes([l[4], l[5]]);
    refused(patched(0, &(size - 2).to_le_bytes()), "a size not a multiple of 4");
    refused(patched(0, &(size + 4).to_le_bytes()), "a size past the list");
    // the sections and their buckets
    bad(ScoreRecord { intro_t: 0, intro_fp: vec![], intro_events: vec![], ..score() }, "no intro and no loop");
    refused(patched(20, &[2, 0]), "intro buckets under its length's");
    refused(patched(20, &[4, 0]), "intro buckets over its length's");
    refused(patched_of(&looped(), 22, &[1, 0]), "loop buckets under its length's");
    refused(patched_of(&looped(), 22, &[3, 0]), "loop buckets over its length's");
    refused(patched(16, &[1, 0, 0, 0]), "a loop of 1 T, no bucket");
    bad(ScoreRecord { loop_t: F32 - 1, loop_fp: vec![], loop_events: vec![], ..looped() }, "a loop with no whole bucket");
    let max = SND_SCORE_BUCKETS_MAX as u32;
    bad(ScoreRecord { intro_t: (max + 1) * F32, intro_fp: vec![3; max as usize + 1], intro_events: vec![], ..score() }, "over the most buckets");
    bad(
        ScoreRecord { intro_t: 9 * F32, intro_fp: vec![3; 9], loop_t: (max - 8) * F32, loop_fp: vec![4; max as usize - 8], ..looped() },
        "over the most in two",
    );
    refused(patched(12, &[0xFF, 0xFF, 0xFF, 0xFF]), "an intro of 2^32 - 1 T");
    // the pitches
    refused(patched(28, &[0]), "no pitch");
    refused(patched(28, &[255]), "pitches past the record");
    refused(patched(32, &[63]), "a pitch under 64");
    refused(raw(Head { pitches: 0, ..HEAD }, &[0x78, 0x00, 0x00, 0x00]), "no pitch, raw");
    // the fingerprint
    refused(patched(38, &[0x87]), "a section's first bucket not an escape");
    refused(patched_of(&looped(), 44 + 3, &[0x07]), "the loop's first bucket not an escape");
    refused(patched(40, &[0x1E]), "the spare nibble not 0");
    refused(raw(Head { buckets: (2, 0), intro_t: 2 * F32, ..HEAD }, &[0x08, 0xF0, 0x00, 0x00]), "a count change below 0");
    refused(raw(Head { buckets: (2, 0), intro_t: 2 * F32, ..HEAD }, &[0xF8, 0x7F, 0x00, 0x00]), "a count change above 255");
    refused(raw(Head { events: (0, 0), ..HEAD }, &[0x78]), "an escape cut off");
    refused(raw(Head { buckets: (200, 0), intro_t: 200 * F32, events: (0, 0), ..HEAD }, &[0x78, 0x00, 0, 0, 0, 0]), "a fingerprint past the record");
    // the events
    refused(patched(24, &[9, 0]), "an event past the record");
    refused(raw(Head { events: (3, 0), ..HEAD }, &FP1), "events past the record");
    let lp = looped();
    refused(patched_of(&ScoreRecord { loop_t: 0, loop_fp: vec![], loop_events: vec![], ..lp.clone() }, 26, &[1, 0]), "a loop's event with no loop");
    refused(raw(HEAD, &[0x78, 0x00, 0x00, 0x04, 0x01]), "pitch index 1 of one");
    refused(raw(Head { voices: 2, ..HEAD }, &[0x78, 0x00, 0x00, 0x02]), "voice 2 of two");
    refused(raw(HEAD, &[0x78, 0x00, 0xC4, 0x08, 0x00]), "an event at the intro's end");
    refused(raw(HEAD, &[0x78, 0x00, 0xFF, 0xFF, 0xFF, 0x7F, 0x00]), "an event far past the intro's end");
    let lh = Head { loop_t: F32, buckets: (1, 1), events: (0, 1), ..HEAD };
    assert!(decodes(&raw(lh, &[0x78, 0x80, 0x07, 0xC3, 0x08, 0x00])), "unit 1091 of a one-bucket loop");
    refused(raw(lh, &[0x78, 0x80, 0x07, 0xC4, 0x08, 0x00]), "an event at the loop's end");
    refused(raw(HEAD, &[0x78, 0x00, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00]), "a varint of five bytes");
    refused(raw(Head { events: (2, 0), ..HEAD }, &[0x78, 0x00, 0x00, 0x00, 0x00]), "an event cut off at its byte");
    refused(raw(HEAD, &[0x78, 0x00, 0x80, 0x00, 0x04]), "a note on cut off at its pitch");
    refused(raw(HEAD, &[0x78, 0x00, 0x80, 0x80, 0x80]), "a varint running off the record");
    // one bad score refuses the list, its learned entries too
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    refused(encode_list_scores(&[], &[roles], &[score(), ScoreRecord { voices: 0, ..score() }]), "a bad second score");
}

/// [`looped`] with drum hits and offs in both sections, one voice.
fn drummed() -> ScoreRecord {
    ScoreRecord {
        voices: 1,
        intro_events: vec![on(0, 0, 2716, 200), hit(0, 255), off(2000, SCORE_DRUM), hit(F32, 9)],
        loop_events: vec![hit(0, 120), on(0, 0, 3000, 200), off(640, SCORE_DRUM), hit(2 * F32 + 50, 1)],
        ..looped()
    }
}

#[test]
fn drums_in_scores() {
    // a drum event is its byte alone (voice 3, the level's step), no pitch byte; drum periods are
    // not in the pitch table
    let l = list(&[drummed()]);
    let r = &l[4..];
    assert_eq!(r[3], 1, "voices counts the tonal voices");
    assert_eq!((r[28], &r[32..38]), (2, &[0x9C, 0x0A, 0, 0xB8, 0x0B, 0][..]), "2716 and 3000 T");
    let (_, ev) = parse(&l);
    let l50 = |level: u8| (u32::from(level) + 2) / 4;
    assert_eq!(ev[0], [(0, 0, 50, 2716), (0, 3, 63, 0), ((2000 + 32) / 64, 3, 0, 0), (F32 / 64, 3, l50(9) as u8, 0)]);
    assert_eq!(ev[1], [(0, 3, l50(120) as u8, 0), (0, 0, 50, 3000), (10, 3, 0, 0), ((2 * F32 + 50 + 32) / 64, 3, 1, 0)]);
    // the fingerprint: 40 and 52 two escapes, 30 an escape and +1: 10 nibbles
    let fp_bytes = (3 + 3 + 3 + 1usize).div_ceil(2);
    let events_at = 32 + 6 + fp_bytes;
    assert_eq!(&r[events_at..events_at + 6], [0x00, 50 << 2, 0, 0x00, 0x03 | 63 << 2, 0x1F], "voice 0 on, pitch 0; the drum hit; dt 31");
    // the decoder takes drum events in both sections whatever `voices` is; a drum hit's byte is
    // followed by the next event's time
    assert!(decodes(&l));
    assert!(decodes(&raw(HEAD, &[0x78, 0x00, 0x00, 0x03])), "a drum off, one voice");
    assert!(decodes(&raw(HEAD, &[0x78, 0x00, 0x00, 0xFF])), "a drum hit of step 63, one pitch, no pitch byte");
    assert!(decodes(&raw(Head { events: (2, 0), ..HEAD }, &[0x78, 0x00, 0x00, 0xFF, 0xC3, 0x08, 0x07])), "a hit, then the drum off at unit 1091");
    let lh = Head { loop_t: F32, buckets: (1, 1), events: (0, 1), ..HEAD };
    assert!(decodes(&raw(lh, &[0x78, 0x80, 0x07, 0xC3, 0x08, 0x07])), "a drum hit at unit 1091 of a one-bucket loop");
    // the drum alone; a score with no tonal note on still has one pitch
    let drum_only = ScoreRecord { intro_events: vec![hit(0, 100), off(64, SCORE_DRUM)], loop_events: vec![hit(0, 100)], ..drummed() };
    let l = list(&[drum_only]);
    assert_eq!(l[4 + 28], 1);
    assert!(decodes(&l));
}

#[test]
fn decoder_refuses_bad_drum_events() {
    let refused = |l: Vec<u8>, what: &str| assert!(!decodes(&l), "{what}: {l:02X?}");
    // the time: past its section's end, in the loop of a score with no loop
    refused(raw(HEAD, &[0x78, 0x00, 0xC4, 0x08, 0x07]), "a drum hit at the intro's end");
    refused(raw(HEAD, &[0x78, 0x00, 0xC4, 0x08, 0x03]), "the drum off at the intro's end");
    let lh = Head { loop_t: F32, buckets: (1, 1), events: (0, 1), ..HEAD };
    refused(raw(lh, &[0x78, 0x80, 0x07, 0xC4, 0x08, 0x07]), "a drum hit at the loop's end");
    refused(raw(Head { events: (1, 1), ..HEAD }, &[0x78, 0x00, 0x00, 0x03, 0x00, 0x07]), "a drum hit in a loop that is not there");
    let d = drummed();
    let no_loop = ScoreRecord { loop_t: 0, loop_fp: vec![], loop_events: vec![], ..d.clone() };
    refused(patched_of(&no_loop, 26, &[1, 0]), "a loop's drum event with no loop");
    // the record: a drum event cut off at its byte (36 bytes: no padding), or its time
    refused(raw(Head { events: (2, 0), ..HEAD }, &[0x78, 0x00, 0x00, 0xFF, 0x00]), "an event after a drum hit cut off at its byte");
    refused(raw(Head { events: (2, 0), ..HEAD }, &[0x78, 0x00, 0x00, 0xFF, 0x80, 0x80, 0x80, 0x80, 0x00]), "a five-byte time after a drum hit");
    // a tonal voice at or past `voices` is still refused
    refused(raw(Head { voices: 2, ..HEAD }, &[0x78, 0x00, 0x00, 0x02]), "voice 2 of two");
}

#[test]
fn scores_read_back_and_are_replaced() {
    let e = LearnedEntry {
        digest: 1,
        len: 4,
        wild: vec![],
        reloc: vec![],
        sites: vec![(0, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        observed: false,
        drum: 0,
        stale: None,
        level: vec![200],
        instrument: [0; 3],
        progs: vec![vec![Op::K(300)]],
        timing: None,
    };
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    // the encoder's rounding: times to 64 T, levels to their step x 4, periods the pitch table's
    let round = |s: &ScoreRecord| {
        let ev = |es: &[ScoreEvent], len: u32| {
            es.iter()
                .map(|e| ScoreEvent {
                    t: ((e.t + 32) / 64).min((len - 1) / 64) * 64,
                    on: e.on.map(|(p, l)| (if e.voice == SCORE_DRUM { 0 } else { p }, ((u32::from(l) + 2) / 4).clamp(1, 63) as u8 * 4)),
                    ..*e
                })
                .collect::<Vec<_>>()
        };
        ScoreRecord { intro_events: ev(&s.intro_events, s.intro_t), loop_events: ev(&s.loop_events, s.loop_t.max(1)), ..s.clone() }
    };
    let scores = [score(), looped(), drummed()];
    let l = encode_list_scores(std::slice::from_ref(&e), &[roles], &scores);
    assert!(decodes(&l));
    let back = decode_scores(&l).expect("reads back");
    assert_eq!(back, scores.iter().map(round).collect::<Vec<_>>());
    assert_eq!(back[2].intro_events[1], hit(0, 252), "a drum hit: period 0, level its step x 4");
    // written back as they read: the same bytes; read back again, the same scores
    assert_eq!(replace_scores(&l, &back).as_ref(), Ok(&l));
    assert_eq!(replace_scores(&l, &scores).as_ref(), Ok(&l), "the encoder's own rounding");
    // replaced: the header, the learned entry and the kind-1 record as they were; the new scores
    // read back
    let new = [drummed(), score(), ScoreRecord { loop_events: vec![], ..drummed() }];
    let r = replace_scores(&l, &new).expect("replaced");
    let head = l.len() - (encode_list_scores(&[], &[], &scores).len() - 4);
    assert_eq!(r[..head], l[..head], "the header and kinds 0 and 1 byte for byte");
    assert_eq!(r, encode_list_scores(std::slice::from_ref(&e), &[roles], &new));
    assert_eq!(decode_scores(&r), Ok(new.iter().map(round).collect::<Vec<_>>()));
    assert!(decodes(&r));
    // bytes after the last entry stay
    let mut tail = l.clone();
    tail.extend([1, 2, 3, 4]);
    assert_eq!(&replace_scores(&tail, &new).unwrap()[r.len()..], [1, 2, 3, 4]);
    // a list of learned entries alone: no scores, none to give
    let plain = encode_list_with(std::slice::from_ref(&e), &[roles]);
    assert_eq!(decode_scores(&plain), Ok(vec![]));
    assert_eq!(replace_scores(&plain, &[]).as_ref(), Ok(&plain));
    // refused: another count, a score the decoder would refuse, a record too big, a bad list
    assert!(replace_scores(&l, &scores[..2]).is_err(), "two for three");
    assert!(replace_scores(&plain, &[score()]).is_err(), "one for none");
    assert!(replace_scores(&l, &[score(), ScoreRecord { voices: 0, ..score() }, score()]).is_err(), "no voice");
    let late = ScoreRecord { intro_events: vec![on(0, 2, 100, 9)], ..score() };
    assert!(replace_scores(&l, &[score(), late.clone(), score()]).is_err(), "voice 2 of two");
    let many: Vec<ScoreEvent> = (0..40_000).map(|i| on(i % 100, 0, 1000 + i, 9)).collect();
    let big = ScoreRecord { intro_events: many, ..score() };
    assert!(replace_scores(&l, &[score(), big, score()]).unwrap_err().contains("bytes"), "a record over 65,532 bytes");
    assert!(decode_scores(&list(&[late])).is_err());
    assert!(decode_scores(&l[..l.len() - 4]).is_err(), "cut off");
    assert!(decode_scores(&[1, 0]).is_err());
}

#[test]
fn score_key_names_a_score() {
    // CRC-32 of "123456789": CBF43926h
    let s = ScoreRecord { site: 0x8951, intro_fp: b"12345".to_vec(), loop_fp: b"6789".to_vec(), ..looped() };
    assert_eq!(score_key(&s), "8951-cbf43926");
    assert_eq!(score_key(&ScoreRecord { site: 0x00AB, intro_fp: vec![], loop_fp: vec![], ..s.clone() }), "00AB-00000000");
    // the same fingerprints split elsewhere: the same key; the events, roles and lengths do not
    // change it; a bucket does
    assert_eq!(score_key(&ScoreRecord { intro_fp: b"1234".to_vec(), loop_fp: b"56789".to_vec(), ..s.clone() }), "8951-cbf43926");
    assert_eq!(score_key(&ScoreRecord { loop_events: vec![], roles: [3, 3, 3], intro_t: 1, ..s.clone() }), score_key(&s));
    assert_ne!(score_key(&ScoreRecord { loop_fp: b"6788".to_vec(), ..s.clone() }), score_key(&s));
    // a decoded score keeps its key
    assert_eq!(score_key(&decode_scores(&list(&[looped()])).unwrap()[0]), score_key(&looped()));
}

#[test]
fn a_typical_record_is_small() {
    // 170 s of music, no loop: 1000 notes (each a note on and a note off), a fingerprint of
    // about 50 edges a bucket that changes by a few each bucket and jumps at each note
    let buckets = (170.0 * 3_500_000.0 / f64::from(F32)) as u32;
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let len = buckets * F32;
    let mut events = Vec::new();
    for i in 0..1000u32 {
        let t = (u64::from(len) * u64::from(i) / 1000) as u32;
        events.push(on(t, (i % 2) as u8, 400 + 40 * (next() % 24) as u32, 200));
        events.push(off(t + len / 1500, (i % 2) as u8));
    }
    let mut c = 50i32;
    let fp: Vec<u8> = (0..buckets)
        .map(|b| {
            c = if b % 8 == 0 { 30 + (next() % 60) as i32 } else { c + (next() % 5) as i32 - 2 };
            c as u8
        })
        .collect();
    let s = ScoreRecord { voices: 2, intro_t: len, intro_fp: fp, intro_events: events, ..score() };
    let l = list(&[s]);
    assert!(decodes(&l));
    let size = l.len() - 4;
    assert!(size < 12 * 1024, "{size} bytes");
}

// ---- the player ------------------------------------------------------------

/// Where the tune's notes lie.
const TABLE: u16 = 0x9000;

/// A note of the tune: the `DJNZ` count, the edges it makes, and from which `OUT`.
#[derive(Clone, Copy)]
struct Note {
    n: u8,
    edges: u16,
    b: bool,
}

impl Note {
    /// Its half period (T): `58 + 13n`, 4 more from `OUT` b.
    fn half(&self) -> u32 {
        58 + 13 * u32::from(self.n) + if self.b { 4 } else { 0 }
    }
}

/// About 8 frames of each of `ns`, from `OUT` a.
fn tune(ns: &[u8]) -> Vec<Note> {
    ns.iter()
        .map(|&n| {
            let half = 58 + 13 * u32::from(n);
            Note { n, edges: (8 * ZX_FRAME_TSTATES / half) as u16 & !1, b: false }
        })
        .collect()
}

/// A tune after a delay: `BC` x 26 T, then the notes from `TABLE` (n, edges as a word, 0 for
/// `OUT` a, else b; n 0 ends it), each a square of half period `58 + 13n` (b: 4 more) from a
/// `DJNZ` loop (the level in A'), then silence.
///
/// ```text
/// start: LD BC,delay
/// dly:   DEC BC ; LD A,B ; OR C ; JR NZ,dly
///        LD HL,TABLE ; XOR A ; EX AF,AF'
/// note:  LD A,(HL) ; OR A ; JR Z,end
///        LD C,A ; INC HL ; LD E,(HL) ; INC HL ; LD D,(HL) ; INC HL ; LD A,(HL) ; INC HL
///        OR A ; JR NZ,togb
/// toga:  EX AF,AF' ; XOR 10h ; OUT (FEh),A ; EX AF,AF' ; DEC DE ; LD A,D ; OR E ; JR Z,note
///        LD B,C ; DJNZ $ ; JR toga
/// togb:  EX AF,AF' ; XOR 10h ; NOP ; OUT (FEh),A ; EX AF,AF' ; DEC DE ; LD A,D ; OR E ; JR Z,note
///        LD B,C ; DJNZ $ ; JR togb
/// end:   JR end
/// ```
fn program(delay: u16) -> (Vec<u8>, Labels) {
    program_from(delay, None)
}

/// [`program`], its notes from `TABLE`, or with `sel` from the address in the word at `sel`
/// (`LD HL,(sel)` in place of `LD HL,TABLE`: the same code around the `OUT`s).
fn program_from(delay: u16, sel: Option<u16>) -> (Vec<u8>, Labels) {
    program_with(delay, sel, false)
}

/// [`program_from`]; with `again`, at the notes' end (n 0) the notes go on from the address in
/// the word at [`AGAIN`] (`LD HL,(AGAIN) ; JR note` in place of `JR end`): a tune that repeats a
/// phrase forever.
fn program_with(delay: u16, sel: Option<u16>, again: bool) -> (Vec<u8>, Labels) {
    let mut a = Asm::default();
    a.label("start").b(&[0x01]).b(&delay.to_le_bytes());
    a.label("dly").b(&[0x0B, 0x78, 0xB1]).r(0x20, "dly");
    match sel {
        Some(at) => a.b(&[0x2A]).b(&at.to_le_bytes()),
        None => a.b(&[0x21]).b(&TABLE.to_le_bytes()),
    };
    a.b(&[0xAF, 0x08]);
    a.label("note").b(&[0x7E, 0xB7]).r(0x28, "end");
    a.b(&[0x4F, 0x23, 0x5E, 0x23, 0x56, 0x23, 0x7E, 0x23, 0xB7]).r(0x20, "togb");
    a.label("toga").b(&[0x08, 0xEE, 0x10]).label("out_a").b(&[0xD3, 0xFE, 0x08, 0x1B, 0x7A, 0xB3]).r(0x28, "note");
    a.b(&[0x41, 0x10, 0xFE]).r(0x18, "toga");
    a.label("togb").b(&[0x08, 0xEE, 0x10, 0x00]).label("out_b").b(&[0xD3, 0xFE, 0x08, 0x1B, 0x7A, 0xB3]).r(0x28, "note");
    a.b(&[0x41, 0x10, 0xFE]).r(0x18, "togb");
    if again {
        a.label("end").b(&[0x2A]).b(&AGAIN.to_le_bytes()).r(0x18, "note");
    } else {
        a.label("end").r(0x18, "end");
    }
    a.finish()
}

fn table_bytes(notes: &[Note]) -> Vec<u8> {
    let mut t: Vec<u8> = notes.iter().flat_map(|n| [n.n, n.edges as u8, (n.edges >> 8) as u8, u8::from(n.b)]).collect();
    t.push(0);
    t
}

fn machine(delay: u16, notes: &[Note]) -> Machine {
    let mut m = fixtures::bare(&program(delay).0);
    for (i, &b) in table_bytes(notes).iter().enumerate() {
        m.poke(TABLE + i as u16, b);
    }
    m
}

/// A score's event at its time from T0: (T, voice, a note on's period and level).
type Timed = (u64, u8, Option<(u32, u8)>);

/// A run: each frame's edges and the level before them, and method B's decision about it.
struct Run {
    frames: Vec<(Vec<zx_edge>, u8)>,
    dec: Vec<SndFrame>,
    /// The mixer's FM choice per decided frame (with `mixer`).
    fm: Vec<bool>,
}

/// Runs the tune `frames` frames, with method B on `list`'s table and scores (and the mixer in
/// FM: engines on it too); `poke`: from that frame on, the notes are these.
fn run(delay: u16, notes: &[Note], frames: usize, list: &[u8], poke: Option<(usize, &[Note])>) -> Run {
    run_on(machine(delay, notes), frames, list, poke)
}

/// [`run`] on the machine `m`.
fn run_on(mut m: Machine, frames: usize, list: &[u8], poke: Option<(usize, &[Note])>) -> Run {
    let mut b = SndEngines::with_learned(list).expect("the list decodes");
    let mut mix = SndMixer::new(750_000, 17);
    mix.set_learned(list).expect("the mixer takes the list");
    let mut beeper = zx_sys::Beeper::new(750_000, 17);
    let delay_f = SND_DELAY_FRAMES as usize;
    let mut r = Run { frames: Vec::new(), dec: Vec::new(), fm: Vec::new() };
    for f in 0..frames {
        if let Some((_, new)) = poke.filter(|p| p.0 == f) {
            for (i, &x) in table_bytes(new).iter().enumerate() {
                m.poke(TABLE + i as u16, x);
            }
        }
        m.run_frame();
        let (edges, level) = (m.edges().to_vec(), m.edges_start_level());
        let d = b.frame(&m, &edges, level);
        beeper.set_level(level);
        let samples = beeper.render(&edges);
        mix.frame(SND_MODE_FM_B, Some(&m), &edges, level, &samples);
        if f >= delay_f {
            r.dec.push(d);
            r.fm.push(mix.fm_on());
        }
        r.frames.push((edges, level));
    }
    r
}

/// The run's first speaker edge: (frame, T-state).
fn first_edge(r: &Run) -> (usize, u32) {
    r.frames.iter().enumerate().find_map(|(f, (e, l))| ear_edges(*l, e).next().map(|e| (f, e.tstate))).expect("the tune sounds")
}

/// Every speaker edge of the run as T from its first.
fn edge_times(r: &Run) -> Vec<u64> {
    let (f0, t0) = first_edge(r);
    let base = f0 as u64 * F + u64::from(t0);
    r.frames.iter().enumerate().flat_map(|(f, (e, l))| ear_edges(*l, e).map(move |e| f as u64 * F + u64::from(e.tstate) - base).collect::<Vec<_>>()).collect()
}

/// A score recorded from `r` (the tune of `notes`): the fingerprint from its first edge to its
/// last, and by hand, voice 0 each note's pitch from its first edge (to 64 T) and off after the
/// last, voice 1 a low note over the first two.
fn record(r: &Run, notes: &[Note], m: &Machine, labels: &Labels) -> (ScoreRecord, Vec<Timed>) {
    let (f0, t0) = first_edge(r);
    let times = edge_times(r);
    let total: usize = notes.iter().map(|n| usize::from(n.edges)).sum();
    assert_eq!(times.len(), total, "every note's edges");
    let buckets = ((*times.last().unwrap() + 64) / F) as usize + 1;
    let mut abs: Vec<Timed> = Vec::new();
    let mut k = 0;
    for (i, n) in notes.iter().enumerate() {
        let at = times[k] / 64 * 64;
        abs.push((at, 0, Some((2 * n.half(), 200))));
        if i == 0 {
            abs.push((at, 1, Some((8 * n.half(), 120))));
        }
        if i == 2 {
            abs.push((at, 1, None));
        }
        k += usize::from(n.edges);
    }
    abs.push(((times[total - 1] + 64) / 64 * 64, 0, None));
    abs.sort_by_key(|e| e.0);
    let events = abs.iter().map(|&(t, voice, on)| ScoreEvent { t: t as u32, voice, on }).collect();
    let site = labels["out_a"];
    let s = ScoreRecord {
        site,
        quiet: 1,
        code: score_code(|a| m.peek(a), site),
        roles: [SND_ROLE_LEAD, SND_ROLE_BASS, 0],
        voices: 2,
        intro_t: buckets as u32 * F32,
        loop_t: 0,
        intro_fp: live_fp(r, f0, t0, buckets),
        loop_fp: vec![],
        intro_events: events,
        loop_events: vec![],
    };
    (s, abs)
}

/// The decisions' events as (T from the run's first edge, voice, period, level), in the frames
/// method B gave `engine`.
fn played(r: &Run, engine: u8) -> Vec<(u64, u8, u32, u8)> {
    let (f0, t0) = first_edge(r);
    let base = f0 as u64 * F + u64::from(t0);
    r.dec
        .iter()
        .enumerate()
        .filter(|(_, d)| d.confident != 0 && d.engine == engine)
        .flat_map(|(k, d)| d.events().iter().map(move |e| (k as u64 * F + u64::from(e.t) - base, e.voice, e.period_q8, e.level)).collect::<Vec<_>>())
        .collect()
}

fn claimed(r: &Run, engine: u8) -> Vec<usize> {
    r.dec.iter().enumerate().filter(|(_, d)| d.confident != 0 && d.engine == engine).map(|(k, _)| k).collect()
}

/// The events `abs` as the player gives them: (T, voice, period in T x 256, the level it plays).
fn want(abs: &[Timed]) -> Vec<(u64, u8, u32, u8)> {
    abs.iter().map(|&(t, v, on)| (t, v, on.map_or(0, |o| o.0 << 8), on.map_or(0, |o| score_level(o.1)))).collect()
}

/// The live fingerprint of `r`'s first `buckets` buckets from T0, T-state `t0` of frame `f0`.
fn live_fp(r: &Run, f0: usize, t0: u32, buckets: usize) -> Vec<u8> {
    score_fingerprint(&r.frames[f0..], t0, 0, buckets as u64 * F)
}

/// Do a live bucket and the recorded one agree (|live - rec| <= max(2, rec / 8))?
fn agree(live: u8, rec: u8) -> bool {
    live.abs_diff(rec) <= (rec / 8).max(2)
}

const NS: [u8; 6] = [100, 80, 120, 60, 90, 70];
const FRAMES: usize = 90;

/// The recording's run (delay 3000) and the score with its events.
fn recorded() -> (Vec<Note>, ScoreRecord, Vec<Timed>) {
    let notes = tune(&NS);
    let r = run(3000, &notes, FRAMES, &encode_list(&[]), None);
    let (code, labels) = program(3000);
    let m = fixtures::bare(&code);
    let (s, abs) = record(&r, &notes, &m, &labels);
    (notes, s, abs)
}

#[test]
fn score_plays_the_tune_at_any_phase() {
    let (notes, s, abs) = recorded();
    let list = list(std::slice::from_ref(&s));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let mut phases = Vec::new();
    for delay in [3000u16, 5000, 7000, 7700, 12000] {
        let r = run(delay, &notes, FRAMES, &list, None);
        let (f0, t0) = first_edge(&r);
        phases.push(t0);
        // the code around the site is the same whatever the delay
        let (code, labels) = program(delay);
        assert_eq!(score_code(|a| fixtures::bare(&code).peek(a), labels["out_a"]), s.code);
        // the live edges agree with the recording bucket by bucket
        let live = live_fp(&r, f0, t0, s.intro_fp.len());
        assert!(live.iter().zip(&s.intro_fp).all(|(&l, &x)| agree(l, x)), "delay {delay}: {live:?} against {:?}", s.intro_fp);
        // the score claims every frame from T0's to the fingerprint's end, nothing else
        let frames = s.intro_fp.len() + usize::from(t0 != 0);
        assert_eq!(claimed(&r, mine), (f0..f0 + frames).collect::<Vec<_>>(), "delay {delay}, T0 {t0} in frame {f0}");
        assert!(r.dec.iter().enumerate().all(|(k, d)| d.confident == 0 || claimed(&r, mine).contains(&k)), "delay {delay}: only the score claims");
        // its events at their T-states from T0, the first at T0's
        assert_eq!(played(&r, mine), want(&abs), "delay {delay}");
        assert_eq!(r.dec[f0].events()[0].t, t0, "delay {delay}");
        // the roles, the engine's name
        let d = &r.dec[f0];
        assert_eq!(d.role, [SND_ROLE_LEAD, SND_ROLE_BASS, SND_ROLE_AUTO]);
        assert_eq!(SndEngines::with_learned(&list).unwrap().name(d.engine), "score 1");
        // the mixer plays FM from the score's first frame to its last
        assert!((f0 + 1..f0 + frames).all(|k| r.fm[k]), "delay {delay}: FM throughout");
        assert!(!r.fm[f0 + frames + 2], "delay {delay}: the beeper after");
    }
    phases.sort_unstable();
    phases.dedup();
    assert!(phases.len() >= 4, "the tune starts at different T-states in its frame: {phases:?}");
}

#[test]
fn score_stops_when_the_tune_changes() {
    let (notes, s, _) = recorded();
    let list = list(std::slice::from_ref(&s));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let mut changed = notes.clone();
    for n in &mut changed[3..] {
        n.n = 20;
    }
    for delay in [3000u16, 7700] {
        let plain = run(delay, &notes, FRAMES, &list, None);
        let (f0, t0) = first_edge(&plain);
        let poke = f0 + 12;
        let r = run(delay, &notes, FRAMES, &list, Some((poke, &changed)));
        // the first frame whose edges differ from the plain run's
        let key = |(e, _): &(Vec<zx_edge>, u8)| e.iter().map(|e| (e.tstate, e.level, e.pc)).collect::<Vec<_>>();
        let differs = (0..FRAMES).find(|&f| key(&r.frames[f]) != key(&plain.frames[f])).expect("the tune changes");
        // the first of three disagreeing buckets in a row
        let live = live_fp(&r, f0, t0, s.intro_fp.len());
        let bad: Vec<bool> = live.iter().zip(&s.intro_fp).map(|(&l, &x)| !agree(l, x)).collect();
        let stop = (0..bad.len() - 2).find(|&b| bad[b] && bad[b + 1] && bad[b + 2]).expect("three buckets disagree");
        // the score's frames end where those buckets begin: f0 + stop is the first not played
        let c = claimed(&r, mine);
        assert_eq!(c, (f0..f0 + stop).collect::<Vec<_>>(), "delay {delay}: change in frame {differs}");
        let last = *c.last().unwrap();
        assert!(last <= differs + 1, "delay {delay}: the score plays to frame {last}, the tune changed in {differs}");
        assert!(last + 1 > differs.saturating_sub(2), "delay {delay}: the score stops at {last}, before the change ({differs})");
        // nothing claims the frames after; the beeper returns two frames later and stays
        assert!(r.dec[last + 1..].iter().all(|d| d.confident == 0), "delay {delay}");
        assert!(r.fm[last + 2..].iter().all(|&on| !on), "delay {delay}: the beeper from frame {}", last + 2);
    }
}

#[test]
fn table_wins_the_frames_it_claims() {
    // the tune's third note from OUT b, which an observed learned entry knows
    let mut notes = tune(&NS);
    notes[2] = Note { b: true, edges: notes[2].edges + 40, ..notes[2] };
    let (code, labels) = program(3000);
    let anchor = labels["togb"];
    let mut entry = LearnedEntry {
        digest: 0,
        len: labels["end"] - anchor,
        wild: vec![],
        reloc: vec![],
        sites: vec![(labels["out_b"] - anchor, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        observed: true,
        drum: 0,
        stale: None,
        level: vec![180],
        instrument: [0; 3],
        progs: vec![vec![Op::K((2 * notes[2].half()) << 8)]],
        timing: None,
    };
    let table = SndEngines::new().entries().len();
    let probe = SndEngines::with_learned(&encode_list(std::slice::from_ref(&entry))).unwrap();
    entry.digest = probe.digest(&fixtures::bare(&code), table, anchor);
    let (learned, mine) = (table as u8 + 1, table as u8 + 2);

    let plain = run(3000, &notes, FRAMES, &encode_list(&[]), None);
    let (s, abs) = record(&plain, &notes, &fixtures::bare(&code), &labels);
    let l = encode_list_scores(std::slice::from_ref(&entry), &[], std::slice::from_ref(&s));
    assert_eq!(SndEngines::with_learned(&l).unwrap().name(mine), "score 1");
    for delay in [3000u16, 7700] {
        let r = run(delay, &notes, FRAMES, &l, None);
        let (f0, t0) = first_edge(&r);
        let frames = s.intro_fp.len() + usize::from(t0 != 0);
        let theirs = claimed(&r, learned);
        let ours = claimed(&r, mine);
        assert!(theirs.len() >= 5, "delay {delay}: the learned entry claims the note from OUT b: {theirs:?}");
        // the table's frames: all edges (but one) from OUT b
        for &k in &theirs {
            let (e, _) = &r.frames[k];
            assert!(e.iter().filter(|e| e.pc != labels["out_b"]).count() <= 1, "delay {delay}: frame {k}");
            assert!(r.dec[k].events().iter().all(|e| e.period_q8 == 0 || e.period_q8 == (2 * notes[2].half()) << 8), "delay {delay}: frame {k} has the table's voices");
        }
        // every frame of the score's span is the table's or the score's; the score's after the table's
        let mut all: Vec<usize> = theirs.iter().chain(&ours).copied().collect();
        all.sort_unstable();
        assert_eq!(all, (f0..f0 + frames).collect::<Vec<_>>(), "delay {delay}");
        let after = theirs.last().unwrap() + 1;
        assert!(ours.contains(&after), "delay {delay}: the score plays on after the table's frames");
        // the score's events at their times in its own frames: those falling in the table's left out
        let frame_of = |t: u64| ((t + f0 as u64 * F + u64::from(t0)) / F) as usize;
        let expect: Vec<_> = want(&abs).into_iter().filter(|e| !theirs.contains(&frame_of(e.0))).collect();
        assert_eq!(played(&r, mine), expect, "delay {delay}");
        // the score's voices again at the start of its first frame after the table's: the third
        // note's (its on fell in a frame of the score's), voice 1 off
        let d = &r.dec[after];
        assert_eq!((d.tone[0].period_q8, d.tone[0].level, d.tone[1].period_q8), ((2 * notes[2].half()) << 8, score_level(200), 0), "delay {delay}");
    }
}

#[test]
fn score_releases_its_voices_where_its_fingerprint_ends() {
    let (notes, mut s, abs) = recorded();
    // no note off for the last note: it sounds to the fingerprint's end
    assert_eq!(s.intro_events.pop().map(|e| (e.voice, e.on)), Some((0, None)));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let r = run(5000, &notes, FRAMES, &list(&[s.clone()]), None);
    let (f0, t0) = first_edge(&r);
    let end = f0 + s.intro_fp.len();
    assert_eq!(*claimed(&r, mine).last().unwrap(), end, "T0 at {t0}: the frame the fingerprint ends in");
    let mut expect = want(&abs[..abs.len() - 1]);
    expect.push((s.intro_fp.len() as u64 * F, 0, 0, 0));
    assert_eq!(played(&r, mine), expect);
    assert_eq!(r.dec[end].events().last().map(|e| e.t), Some(t0), "released at T0's T-state in that frame");
}

/// The run's drum as each decided frame starts (`d.drum`) and as its events leave it, checked
/// frame by frame against the events before: (frame, the drum at its start).
fn drum_starts(r: &Run, engine: u8) -> Vec<(usize, u8)> {
    let mut drum = 0;
    let mut out = Vec::new();
    for (k, d) in r.dec.iter().enumerate().filter(|(_, d)| d.confident != 0 && d.engine == engine) {
        out.push((k, d.drum));
        assert_eq!(d.drum, drum, "frame {k}: the drum at its start as the frame before left it");
        for e in d.events().iter().filter(|e| usize::from(e.voice) == 3) {
            drum = e.level;
        }
    }
    out
}

#[test]
fn score_plays_its_drum() {
    let (notes, s, abs) = recorded();
    let mine = SndEngines::new().entries().len() as u8 + 1;
    // a hit 640 T after each note's start, off 3008 T later; the second note's held across
    // frames; the last never off (released where the score ends); times on the 64-T grid
    let starts: Vec<u64> = abs.iter().filter(|e| e.1 == 0 && e.2.is_some()).map(|e| e.0).collect();
    let mut drums: Vec<Timed> = Vec::new();
    for (i, &t) in starts.iter().enumerate() {
        let level = [255, 100, 160, 40, 200, 7][i % 6];
        drums.push((t + 640, SCORE_DRUM, Some((0, level))));
        if i + 1 < starts.len() {
            drums.push((t + 640 + if i == 1 { 2 * F + 1024 } else { 3008 }, SCORE_DRUM, None));
        }
    }
    let mut all: Vec<Timed> = abs.iter().copied().chain(drums.iter().copied()).collect();
    all.sort_by_key(|e| e.0);
    let ds = ScoreRecord { intro_events: all.iter().map(|&(t, voice, on)| ScoreEvent { t: t as u32, voice, on }).collect(), ..s.clone() };
    let list_d = list(std::slice::from_ref(&ds));
    let list_p = list(std::slice::from_ref(&s));
    for delay in [3000u16, 7700] {
        let r = run(delay, &notes, FRAMES, &list_d, None);
        let plain = run(delay, &notes, FRAMES, &list_p, None);
        let (f0, t0) = first_edge(&r);
        // the drum's events at their T-states with the hits' levels, the last released at the
        // score's end; the voices as without the drum, the frames claimed the same
        let mut expect = want(&all);
        expect.push((s.intro_fp.len() as u64 * F, 3, 0, 0));
        assert_eq!(played(&r, mine), expect, "delay {delay}");
        let tonal: Vec<_> = played(&r, mine).into_iter().filter(|e| e.1 < 3).collect();
        assert_eq!(tonal, played(&plain, mine), "delay {delay}: the voices unaffected");
        assert_eq!(claimed(&r, mine), claimed(&plain, mine), "delay {delay}");
        for (a, b) in r.dec.iter().zip(&plain.dec).filter(|(a, _)| a.confident != 0) {
            assert_eq!(a.tone.map(|v| (v.period_q8, v.level)), b.tone.map(|v| (v.period_q8, v.level)), "delay {delay}");
        }
        // the drum held across frames: their starts carry it; a drum at a frame's start somewhere
        let starts = drum_starts(&r, mine);
        assert!(starts.iter().filter(|s| s.1 == score_level(100)).count() >= 2, "delay {delay}: {starts:?}");
        // and nothing of it after the score: no frame after has a drum
        let last = *claimed(&r, mine).last().unwrap();
        assert!(r.dec[last + 1..].iter().all(|d| d.drum == 0 && d.events().iter().all(|e| e.voice < 3)), "delay {delay}");
        assert_eq!(r.dec[f0].events().iter().find(|e| e.voice == 3).map(|e| e.t), Some(t0 + 640).filter(|&t| t < F32), "delay {delay}");
    }
}

#[test]
fn a_stopped_score_leaves_no_drum() {
    let (notes, s, abs) = recorded();
    let mine = SndEngines::new().entries().len() as u8 + 1;
    // a hit at T0 never off: the drum sounds until the score stops
    let mut events = vec![ScoreEvent { t: 0, voice: SCORE_DRUM, on: Some((0, 200)) }];
    events.extend(abs.iter().map(|&(t, voice, on)| ScoreEvent { t: t as u32, voice, on }));
    let ds = ScoreRecord { intro_events: events, ..s.clone() };
    let mut changed = notes.clone();
    for n in &mut changed[3..] {
        n.n = 20;
    }
    let plain = run(3000, &notes, FRAMES, &encode_list(&[]), None);
    let (f0, _) = first_edge(&plain);
    let r = run(3000, &notes, FRAMES, &list(&[ds]), Some((f0 + 12, &changed)));
    let c = claimed(&r, mine);
    let last = *c.last().unwrap();
    assert!(last < f0 + s.intro_fp.len(), "it stops: {c:?}");
    assert!(c[1..].iter().all(|&k| r.dec[k].drum == score_level(200)), "the drum sounds in every frame of the score's after T0's");
    assert!(r.dec[last + 1..].iter().all(|d| d.drum == 0 && d.confident == 0), "none after it stops");
}

// ---- several scores at one site ----------------------------------------------

/// The word naming the tune the selector program plays.
const SEL: u16 = 0x8F00;
/// Where the selector program's tunes lie.
const TABLES: [u16; 3] = [TABLE, 0x9200, 0x9400];

/// Three tunes for one routine: A; B, A's first note with half a frame of a high note inside it
/// (its first bucket differs from A's, the next two agree), then notes of its own; C, its own
/// from the first note.
fn tunes() -> [Vec<Note>; 3] {
    let half_frame = |n: u8| Note { n, edges: (ZX_FRAME_TSTATES / 2 / (58 + 13 * u32::from(n))) as u16 & !1, b: false };
    let mut b = vec![half_frame(100), half_frame(30)];
    b.extend(tune(&[100, 60, 110, 75, 85]));
    [tune(&NS), b, tune(&[20, 120, 70, 95, 55, 65])]
}

/// The selector program (`delay`) with every tune of [`tunes`] in memory, playing tune `k`: the
/// runs differ in the word at `SEL` alone.
fn sel_machine(delay: u16, k: usize) -> Machine {
    let mut m = fixtures::bare(&program_from(delay, Some(SEL)).0);
    for (t, notes) in TABLES.iter().zip(tunes()) {
        for (i, &x) in table_bytes(&notes).iter().enumerate() {
            m.poke(t + i as u16, x);
        }
    }
    m.poke(SEL, TABLES[k] as u8);
    m.poke(SEL + 1, (TABLES[k] >> 8) as u8);
    m
}

/// Of a run's first three buckets (those compared when T0's frame is decided), how many disagree
/// with the fingerprint `fp`.
fn early_misses(r: &Run, fp: &[u8]) -> usize {
    let (f0, t0) = first_edge(r);
    let live = live_fp(r, f0, t0, SND_DELAY_FRAMES as usize);
    live.iter().zip(fp).filter(|(&l, &x)| !agree(l, x)).count()
}

#[test]
fn scores_at_one_site_each_play_their_own_tune() {
    let tunes = tunes();
    let (code, labels) = program_from(3000, Some(SEL));
    let rec: Vec<(ScoreRecord, Vec<Timed>)> = (0..3)
        .map(|k| {
            let r = run_on(sel_machine(3000, k), FRAMES, &encode_list(&[]), None);
            record(&r, &tunes[k], &fixtures::bare(&code), &labels)
        })
        .collect();
    let scores: Vec<ScoreRecord> = rec.iter().map(|r| r.0.clone()).collect();
    assert!(scores.iter().all(|s| (s.site, s.code, s.quiet) == (scores[0].site, scores[0].code, scores[0].quiet)), "one site, one code");
    let l = list(&scores);
    let first = SndEngines::new().entries().len() as u8 + 1;
    for delay in [3000u16, 7700] {
        for (k, (s, abs)) in rec.iter().enumerate() {
            let r = run_on(sel_machine(delay, k), FRAMES, &l, None);
            let (f0, t0) = first_edge(&r);
            // each tune makes its first edge at the same moment: one trigger, three candidates
            assert_eq!((f0, t0), first_edge(&run_on(sel_machine(delay, 0), FRAMES, &encode_list(&[]), None)), "delay {delay}, tune {k}");
            let mine = first + k as u8;
            let frames = s.intro_fp.len() + usize::from(t0 != 0);
            assert_eq!(claimed(&r, mine), (f0..f0 + frames).collect::<Vec<_>>(), "delay {delay}, tune {k}");
            assert_eq!(played(&r, mine), want(abs), "delay {delay}, tune {k}");
            assert_eq!(r.dec[f0].events()[0].t, t0, "delay {delay}, tune {k}");
            for o in (0..3).filter(|&o| o != k) {
                assert!(claimed(&r, first + o as u8).is_empty(), "delay {delay}: tune {k} never plays score {}", o + 1);
            }
            // the choice: its own score agrees throughout; A against B's run (and B against A's)
            // disagrees once, still followed when the choice is made (the fewest disagreeing
            // buckets win, not the lowest index); A and B against C's stop before it
            assert_eq!(early_misses(&r, &s.intro_fp), 0, "delay {delay}, tune {k}");
            let other = early_misses(&r, &scores[[1, 0, 0][k]].intro_fp);
            if k < 2 {
                assert!((1..3).contains(&other), "delay {delay}, tune {k}: {other} buckets disagree");
            } else {
                assert_eq!(other, 3, "delay {delay}, tune {k}");
                assert_eq!(early_misses(&r, &scores[1].intro_fp), 3, "delay {delay}, tune {k}");
            }
        }
    }
    // a lone score of the three plays its own tune as it did in the list, and nothing in the others
    let r = run_on(sel_machine(3000, 1), FRAMES, &list(&scores[1..2]), None);
    assert_eq!(played(&r, first), want(&rec[1].1), "B alone");
    let r = run_on(sel_machine(3000, 2), FRAMES, &list(&scores[1..2]), None);
    assert!(claimed(&r, first).is_empty(), "B alone, C's run");
}

// ---- a loop ----------------------------------------------------------------

/// The word naming where a repeating tune's phrase starts again ([`program_with`]).
const AGAIN: u16 = 0x8F02;

/// A tune of `intro` (maybe no note), then `phrase` repeated forever, from `TABLE`.
fn loop_notes(intro: &[Note], phrase: &[Note]) -> Vec<Note> {
    intro.iter().chain(phrase).copied().collect()
}

/// The repeating program (`delay`) playing [`loop_notes`], the phrase again from its first note.
fn loop_machine(delay: u16, intro: &[Note], phrase: &[Note]) -> Machine {
    let mut m = fixtures::bare(&program_with(delay, None, true).0);
    for (i, &b) in table_bytes(&loop_notes(intro, phrase)).iter().enumerate() {
        m.poke(TABLE + i as u16, b);
    }
    let again = TABLE + 4 * intro.len() as u16;
    m.poke(AGAIN, again as u8);
    m.poke(AGAIN + 1, (again >> 8) as u8);
    m
}

/// A repeating tune's score recorded from its run `r`: the intro to the phrase's first edge, the
/// loop one pass of the phrase (to the first edge of the next), each section's fingerprint from
/// that run, and by hand, voice 0 each note's pitch from its first edge (to 64 T), voice 1 a low
/// note from the intro's first note to its second, and in the loop from the phrase's third note
/// to the pass's end (no note off: the player releases it at the next pass's start). Also the
/// lengths of the run's first passes.
fn record_loop(r: &Run, intro: &[Note], phrase: &[Note], m: &Machine, labels: &Labels) -> (ScoreRecord, Vec<u64>) {
    let (f0, t0) = first_edge(r);
    let times = edge_times(r);
    let edges = |ns: &[Note]| ns.iter().map(|n| usize::from(n.edges)).sum::<usize>();
    let (ie, pe) = (edges(intro), edges(phrase));
    let passes: Vec<u64> = (0..).map(|k| ie + k * pe).take_while(|&i| i + pe < times.len()).map(|i| times[i + pe] - times[i]).collect();
    let (intro_t, loop_t) = (times[ie] as u32, passes[0] as u32);
    let mut intro_events = Vec::new();
    let mut k = 0;
    for (i, n) in intro.iter().enumerate() {
        let at = (times[k] / 64 * 64) as u32;
        intro_events.push(on(at, 0, 2 * n.half(), 200));
        match i {
            0 => intro_events.push(on(at, 1, 8 * n.half(), 120)),
            1 => intro_events.push(off(at, 1)),
            _ => {}
        }
        k += usize::from(n.edges);
    }
    intro_events.sort_by_key(|e| e.t);
    let mut loop_events = Vec::new();
    for (i, n) in phrase.iter().enumerate() {
        let at = ((times[k] - u64::from(intro_t)) / 64 * 64) as u32;
        loop_events.push(on(at, 0, 2 * n.half(), 200));
        if i == 2 {
            loop_events.push(on(at, 1, 6 * n.half(), 120));
        }
        k += usize::from(n.edges);
    }
    let site = labels["out_a"];
    let frames = &r.frames[f0..];
    let s = ScoreRecord {
        site,
        quiet: 1,
        code: score_code(|a| m.peek(a), site),
        roles: [SND_ROLE_LEAD, SND_ROLE_BASS, 0],
        voices: 2,
        intro_t,
        loop_t,
        intro_fp: score_fingerprint(frames, t0, 0, u64::from(intro_t)),
        loop_fp: score_fingerprint(frames, t0, u64::from(intro_t), u64::from(loop_t)),
        intro_events,
        loop_events,
    };
    (s, passes)
}

/// The events of `s` as the player gives them, to T `until` from T0: the intro's, then each
/// pass's from its start, the voices sounding at a pass's start that its first events do not
/// restate released there, and the drum if it sounds.
fn score_timeline(s: &ScoreRecord, until: u64) -> Vec<(u64, u8, u32, u8)> {
    let mut abs: Vec<Timed> = s.intro_events.iter().map(|e| (u64::from(e.t), e.voice, e.on)).collect();
    let mut sounding = [false; 4];
    for e in &s.intro_events {
        sounding[usize::from(e.voice)] = e.on.is_some();
    }
    let restated: Vec<u8> = s.loop_events.iter().filter(|e| e.t == 0 && e.on.is_some() && e.voice != SCORE_DRUM).map(|e| e.voice).collect();
    let mut start = u64::from(s.intro_t);
    while start < until {
        for v in 0..4u8 {
            if sounding[usize::from(v)] && !restated.contains(&v) {
                abs.push((start, v, None));
                sounding[usize::from(v)] = false;
            }
        }
        for e in &s.loop_events {
            abs.push((start + u64::from(e.t), e.voice, e.on));
            sounding[usize::from(e.voice)] = e.on.is_some();
        }
        start += u64::from(s.loop_t);
    }
    want(&abs.into_iter().filter(|e| e.0 < until).collect::<Vec<_>>())
}

const INTRO: [u8; 2] = [100, 80];
const PHRASE: [u8; 4] = [120, 60, 90, 70];
/// Frames of the recording's run (the intro and two passes), of a run that plays on.
const REC_FRAMES: usize = 90;
const LOOP_FRAMES: usize = 200;

#[test]
fn score_follows_its_loop_round_and_round() {
    let phrase = tune(&PHRASE);
    let mine = SndEngines::new().entries().len() as u8 + 1;
    for intro in [tune(&INTRO), vec![]] {
        let (code, labels) = program_with(3000, None, true);
        let rec = run_on(loop_machine(3000, &intro, &phrase), REC_FRAMES, &encode_list(&[]), None);
        let (s, passes) = record_loop(&rec, &intro, &phrase, &fixtures::bare(&code), &labels);
        assert!(passes.len() >= 2, "{passes:?}");
        assert!(s.loop_fp.len() >= 30 && s.intro_fp.len() == s.intro_t as usize / F32 as usize, "{s:?}");
        assert_eq!(s.intro_t == 0, intro.is_empty());
        let l = list(std::slice::from_ref(&s));
        assert!(decodes(&l));
        for delay in [3000u16, 5000, 7700] {
            let r = run_on(loop_machine(delay, &intro, &phrase), LOOP_FRAMES, &l, None);
            let (f0, t0) = first_edge(&r);
            let decided = r.dec.len();
            // the score claims every frame from T0's on, the recorded intro and pass and three
            // passes more at least
            let c = claimed(&r, mine);
            assert_eq!(c, (f0..decided).collect::<Vec<_>>(), "intro {}, delay {delay}: T0 {t0} in frame {f0}", intro.len());
            let horizon = (decided - f0) as u64 * F - u64::from(t0);
            assert!(horizon >= u64::from(s.intro_t) + 4 * u64::from(s.loop_t), "intro {}, delay {delay}: {horizon} T", intro.len());
            // every pass's events at their T-states from its start, the voice the loop does not
            // restate released at each pass's start
            let expect = score_timeline(&s, horizon);
            assert_eq!(played(&r, mine), expect, "intro {}, delay {delay}", intro.len());
            let releases = expect.iter().filter(|e| e.1 == 1 && e.2 == 0).count();
            assert!(releases >= 4, "intro {}, delay {delay}: {releases} releases of voice 1", intro.len());
            assert!((f0 + 1..decided).all(|k| r.fm[k]), "intro {}, delay {delay}: FM throughout", intro.len());
        }
    }
}

#[test]
fn a_looping_score_plays_its_drum() {
    let (intro, phrase) = (tune(&INTRO), tune(&PHRASE));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let (code, labels) = program_with(3000, None, true);
    let rec = run_on(loop_machine(3000, &intro, &phrase), REC_FRAMES, &encode_list(&[]), None);
    let (s, _) = record_loop(&rec, &intro, &phrase, &fixtures::bare(&code), &labels);
    // the intro: a hit never off (released at the first pass's start); each pass: a hit at its
    // start (after the release: it strikes anew), off, a hit in the middle and off, and a last hit
    // never off (released at the next pass's start); times on the 64-T grid
    let mut ds = s.clone();
    ds.intro_events.push(hit(F32 / 2, 90));
    ds.intro_events.sort_by_key(|e| e.t);
    let lt = s.loop_t;
    let g = |t: u32| t / 64 * 64;
    ds.loop_events.extend([hit(0, 255), off(1984, SCORE_DRUM), hit(g(lt / 2), 60), off(g(lt / 2) + 4992, SCORE_DRUM), hit(g(lt - 4000), 140)]);
    ds.loop_events.sort_by_key(|e| e.t);
    let l = list(std::slice::from_ref(&ds));
    assert!(decodes(&l));
    let plain_l = list(std::slice::from_ref(&s));
    for delay in [3000u16, 7700] {
        let r = run_on(loop_machine(delay, &intro, &phrase), LOOP_FRAMES, &l, None);
        let plain = run_on(loop_machine(delay, &intro, &phrase), LOOP_FRAMES, &plain_l, None);
        let (f0, t0) = first_edge(&r);
        let horizon = (r.dec.len() - f0) as u64 * F - u64::from(t0);
        assert!(horizon >= u64::from(s.intro_t) + 4 * u64::from(s.loop_t));
        let expect = score_timeline(&ds, horizon);
        assert_eq!(played(&r, mine), expect, "delay {delay}");
        // the drum released at every pass's start, before its hit there
        for k in 0..4u64 {
            let at = u64::from(s.intro_t) + k * u64::from(lt);
            let drum: Vec<_> = expect.iter().filter(|e| e.0 == at && e.1 == 3).map(|e| e.3).collect();
            assert_eq!(drum, [0, score_level(255)], "delay {delay}: pass {k}");
        }
        let tonal: Vec<_> = played(&r, mine).into_iter().filter(|e| e.1 < 3).collect();
        assert_eq!(tonal, played(&plain, mine), "delay {delay}: the voices unaffected");
        drum_starts(&r, mine);
    }
}

#[test]
fn a_looping_score_stops_when_the_tune_changes() {
    let (intro, phrase) = (tune(&INTRO), tune(&PHRASE));
    let (code, labels) = program_with(3000, None, true);
    let rec = run_on(loop_machine(3000, &intro, &phrase), REC_FRAMES, &encode_list(&[]), None);
    let (s, _) = record_loop(&rec, &intro, &phrase, &fixtures::bare(&code), &labels);
    let l = list(std::slice::from_ref(&s));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let mut changed = phrase.clone();
    for n in &mut changed[1..] {
        n.n = 20;
    }
    let changed = loop_notes(&intro, &changed);
    for delay in [3000u16, 7700] {
        let plain = run_on(loop_machine(delay, &intro, &phrase), LOOP_FRAMES, &l, None);
        let (f0, t0) = first_edge(&plain);
        // in the third pass after the recorded one
        let poke = f0 + ((u64::from(t0) + u64::from(s.intro_t) + 3 * u64::from(s.loop_t)) / F) as usize;
        let r = run_on(loop_machine(delay, &intro, &phrase), LOOP_FRAMES, &l, Some((poke, &changed)));
        let key = |(e, _): &(Vec<zx_edge>, u8)| e.iter().map(|e| (e.tstate, e.level, e.pc)).collect::<Vec<_>>();
        let differs = (0..LOOP_FRAMES).find(|&f| key(&r.frames[f]) != key(&plain.frames[f])).expect("the tune changes");
        let c = claimed(&r, mine);
        let last = *c.last().unwrap();
        assert_eq!(c, (f0..=last).collect::<Vec<_>>(), "delay {delay}");
        assert!(last <= differs + 1 && last + 4 > differs, "delay {delay}: the score plays to frame {last}, the tune changed in {differs}");
        assert!(r.dec[last + 1..].iter().all(|d| d.confident == 0), "delay {delay}");
        assert!(r.fm[last + 2..].iter().all(|&on| !on), "delay {delay}: the beeper from frame {}", last + 2);
        // the events up to its last frame as in the plain run
        let plain_ev: Vec<_> = played(&plain, mine).into_iter().filter(|e| ((e.0 + f0 as u64 * F + u64::from(t0)) / F) as usize <= last).collect();
        assert_eq!(played(&r, mine), plain_ev, "delay {delay}");
    }
}

/// A score whose loop the tune does not repeat: the loop plays its first pass and stops within
/// the delay of where the tune leaves it.
#[test]
fn a_loop_the_tune_does_not_repeat_stops() {
    let (notes, s, _) = recorded();
    let mine = SndEngines::new().entries().len() as u8 + 1;
    // the score's last 30 buckets as a loop (its first event restating the note sounding there):
    // the tune is silent after its notes
    let n = s.intro_fp.len();
    let cut = (n - 30) as u32 * F32;
    let intro_events: Vec<ScoreEvent> = s.intro_events.iter().filter(|e| e.t < cut).copied().collect();
    let sounding = intro_events.iter().rev().find(|e| e.voice == 0).and_then(|e| e.on).expect("a note at the cut");
    let mut loop_events = vec![on(0, 0, sounding.0, sounding.1)];
    loop_events.extend(s.intro_events.iter().filter(|e| e.t >= cut).map(|e| ScoreEvent { t: e.t - cut, ..*e }));
    let lp = ScoreRecord {
        intro_t: cut,
        loop_t: s.intro_t - cut,
        intro_fp: s.intro_fp[..n - 30].to_vec(),
        loop_fp: s.intro_fp[n - 30..].to_vec(),
        intro_events,
        loop_events,
        ..s.clone()
    };
    let r = run(5000, &notes, FRAMES + 40, &list(std::slice::from_ref(&lp)), None);
    let (f0, t0) = first_edge(&r);
    let c = claimed(&r, mine);
    let last = *c.last().unwrap();
    assert_eq!(c, (f0..=last).collect::<Vec<_>>());
    // the second pass begins with silence: three buckets and it stops, the frames from the
    // first of them on cancelled
    let end = f0 + n + usize::from(t0 != 0);
    assert!(last + 2 >= end && last < end + 2, "{c:?}, the first pass ends in frame {end}");
    let frame_of = |t: u64| ((t + f0 as u64 * F + u64::from(t0)) / F) as usize;
    let expect: Vec<_> = score_timeline(&lp, 2 * u64::from(s.intro_t)).into_iter().filter(|e| frame_of(e.0) <= last).collect();
    assert_eq!(played(&r, mine), expect);
}

/// Random corruptions of a score's record: the decoder refuses or takes each without reading
/// outside the list (run with `ZX_SANITIZE=1`), and method B plays what it takes.
#[test]
fn corrupted_scores_are_safe() {
    let (notes, s, _) = recorded();
    let intro_events = s.intro_events.iter().filter(|e| e.t < 4 * F32).copied().collect();
    let short = ScoreRecord { intro_t: 4 * F32, intro_fp: s.intro_fp[..4].to_vec(), intro_events, ..s.clone() };
    // and with a loop: its last two buckets, a note on at its start
    let loop_events = vec![on(0, 0, 3000, 200), off(F32 + 64, 0)];
    let lp = ScoreRecord { loop_t: 2 * F32 + 640, loop_fp: s.intro_fp[4..6].to_vec(), loop_events, ..short.clone() };
    // and drum events in both sections
    for good in [list(&[short]), list(&[lp]), list(&[drummed()])] {
        corrupt(&notes, &good);
    }
}

/// Random corruptions of the list `good`, as [`corrupted_scores_are_safe`].
fn corrupt(notes: &[Note], good: &[u8]) {
    assert!(decodes(good));
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (mut taken, mut refused) = (0, 0);
    for _ in 0..3000 {
        let mut l = good.to_vec();
        for _ in 0..1 + next() % 3 {
            let at = 4 + (next() as usize) % (l.len() - 4);
            l[at] = next() as u8;
        }
        if l.len() > 8 && next() % 4 == 0 {
            l.truncate(l.len() - 1 - (next() as usize) % 8);
        }
        match SndEngines::with_learned(&l) {
            Ok(mut b) => {
                taken += 1;
                // read back, written again and read back: the same scores
                let back = decode_scores(&l).expect("decode_scores takes what the decoder takes");
                let again = replace_scores(&l, &back).expect("the scores written again");
                assert_eq!(decode_scores(&again).as_ref(), Ok(&back));
                if taken % 16 == 0 {
                    let mut m = machine(3000, notes);
                    for _ in 0..12 {
                        m.run_frame();
                        b.frame(&m, m.edges(), m.edges_start_level());
                    }
                }
            }
            Err(_) => {
                assert!(decode_scores(&l).is_err(), "decode_scores refuses what the decoder refuses: {l:02X?}");
                refused += 1;
            }
        }
    }
    assert!(taken > 0 && refused > 0, "{taken} taken, {refused} refused");
}
