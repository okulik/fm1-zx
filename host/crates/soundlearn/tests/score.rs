// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recorded scores (kind-2 records): the encoder against the decoder's checks
//! (`core/sound/learned.c`), and method B's player on a synthetic tune recorded on the host: it
//! claims the tune's frames and plays the score's events at their T-states whatever the tune's
//! phase in the frame, stops within the delay when the tune changes (the beeper returns), and
//! leaves a frame a table entry claims to the table; of several scores at one site, each tune
//! plays its own.

mod fixtures;

use fixtures::{Asm, Labels};
use soundlearn::entry::{ear_edges, score_code, score_fingerprint};
use soundlearn::{encode_list, encode_list_scores, encode_list_with, BuiltinRoles, LearnedEntry, Op, ScoreEvent, ScoreRecord};
use zx_sys::{
    zx_edge, Machine, SndEngines, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_MODE_FM_B, SND_ROLE_AUTO, SND_ROLE_BASS, SND_ROLE_LEAD,
    SND_SCORES_MAX, SND_SITE_TONAL, ZX_FRAME_TSTATES,
};

const F: u64 = ZX_FRAME_TSTATES as u64;

// ---- the decoder ---------------------------------------------------------

/// A valid score: two voices, three buckets, four events.
fn score() -> ScoreRecord {
    ScoreRecord {
        site: 0x8123,
        quiet: 2,
        code: 0xDEAD_BEEF,
        roles: [SND_ROLE_BASS, SND_ROLE_LEAD, 0],
        voices: 2,
        fingerprint: vec![40, 41, 39],
        events: vec![
            ScoreEvent { dt_t: 0, voice: 0, on: Some((2716, 200)) },
            ScoreEvent { dt_t: 640, voice: 1, on: Some((64, 1)) },
            ScoreEvent { dt_t: 64 * 200, voice: 0, on: None },
            ScoreEvent { dt_t: 64 * 2000, voice: 1, on: None },
        ],
    }
}

fn list(scores: &[ScoreRecord]) -> Vec<u8> {
    encode_list_scores(&[], &[], scores)
}

fn decodes(l: &[u8]) -> bool {
    SndEngines::with_learned(l).is_ok()
}

/// The list of [`score`] with bytes from `at` (in the record) replaced.
fn patched(at: usize, bytes: &[u8]) -> Vec<u8> {
    let mut l = list(&[score()]);
    l[4 + at..4 + at + bytes.len()].copy_from_slice(bytes);
    l
}

/// The list of one score whose record ends with `tail` (the events' bytes) after a one-bucket
/// fingerprint, `n` events and `voices`, its size fitted.
fn raw(voices: u8, n: u16, tail: &[u8]) -> Vec<u8> {
    let mut r = vec![0, 0, 2, voices, 0x23, 0x81, 0, 0, 0, 0, 0, 0, 1, 0];
    r.extend(n.to_le_bytes());
    r.extend([0, 0, 0, 0, 7]);
    r.extend(tail);
    r.resize(r.len().next_multiple_of(4), 0);
    let size = r.len() as u16;
    r[..2].copy_from_slice(&size.to_le_bytes());
    let mut l = vec![1, 0, 0, 0];
    l.extend(r);
    l
}

#[test]
fn encoder_writes_the_record() {
    let l = list(&[score()]);
    assert_eq!(&l[..4], [1, 0, 0, 0]);
    let r = &l[4..];
    let size = usize::from(u16::from_le_bytes([r[0], r[1]]));
    assert_eq!((size, r[2], r[3]), (l.len() - 4, 2, 2));
    assert_eq!(size % 4, 0);
    assert_eq!(&r[4..20], [0x23, 0x81, 2, 0, 0xEF, 0xBE, 0xAD, 0xDE, 3, 0, 4, 0, SND_ROLE_BASS, SND_ROLE_LEAD, 0, 0]);
    assert_eq!(&r[20..23], [40, 41, 39]);
    // events: dt 0, on 2716 (0x000A9C) level 200; dt 10, voice 1 on 64 level 1; dt 200 (two
    // bytes), voice 0 off; dt 2000, voice 1 off
    #[rustfmt::skip]
    let ev = [0, 0, 0x9C, 0x0A, 0, 200, 10, 1, 64, 0, 0, 1, 0xC8, 0x01, 0x80, 0xD0, 0x0F, 0x81];
    assert_eq!(&r[23..23 + ev.len()], ev);
    assert!(r[23 + ev.len()..].iter().all(|&b| b == 0), "zero padding");

    // the times are rounded from T0, so the rounding does not add up
    let s = ScoreRecord {
        events: (0..4).map(|_| ScoreEvent { dt_t: 40, voice: 0, on: None }).collect(),
        voices: 1,
        ..score()
    };
    let r = &list(&[s])[4 + 23..];
    assert_eq!(&r[..8], [1, 0x80, 0, 0x80, 1, 0x80, 1, 0x80], "40, 80, 120, 160 T: 1, 1, 2, 3 units from T0");

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
fn decoder_takes_good_scores() {
    let b = SndEngines::with_learned(&list(&[score()])).expect("decodes");
    let n = SndEngines::new().entries().len();
    assert_eq!((b.entries().len(), b.scores()), (n, 1), "a score is no table entry");
    assert_eq!(b.name(n as u8 + 1), "score 1");
    assert_eq!(b.name(n as u8 + 2), "");
    let eight = SndEngines::with_learned(&list(&vec![score(); SND_SCORES_MAX])).expect("eight scores");
    assert_eq!((eight.scores(), eight.name(n as u8 + 8).as_str()), (8, "score 8"));
    assert_eq!(eight.name(n as u8 + 9), "");
    // a score with no events, one bucket, the longest fingerprint, three voices
    assert!(decodes(&list(&[ScoreRecord { events: vec![], ..score() }])));
    assert!(decodes(&list(&[ScoreRecord { fingerprint: vec![9], events: vec![], ..score() }])));
    let long = ScoreRecord { fingerprint: vec![0; zx_sys::SND_SCORE_BUCKETS_MAX], ..score() };
    assert!(decodes(&list(&[long])));
    assert!(decodes(&list(&[ScoreRecord { voices: 3, events: vec![ScoreEvent { dt_t: 0, voice: 2, on: Some((0xFF_FFFF, 255)) }], ..score() }])));
    // the last unit before the fingerprint's end (3 buckets: 3 x 1092 units)
    assert!(decodes(&raw(1, 1, &[0xC3, 0x08, 0x80])), "unit 1091 of one bucket");
    // a varint of four bytes; a padded record; roles above 3 (read as none)
    assert!(decodes(&raw(1, 2, &[0x80, 0x80, 0x80, 0x00, 0x80, 0x01, 0x80])));
    let mut padded = list(&[score()]);
    padded.extend([0; 8]);
    let size = u16::from_le_bytes([padded[4], padded[5]]) + 8;
    padded[4..6].copy_from_slice(&size.to_le_bytes());
    assert!(decodes(&padded));
    assert!(decodes(&patched(16, &[9, 200, 4])));
    // 8 entries of kinds 0 and 1 and 8 scores: 16 records
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    assert!(decodes(&encode_list_scores(&[], &[roles; 8], &vec![score(); 8])));
    assert!(!decodes(&encode_list_scores(&[], &[roles; 9], &[score()])), "9 of kinds 0 and 1");
}

#[test]
fn decoder_refuses_bad_scores() {
    let refused = |l: Vec<u8>, what: &str| assert!(!decodes(&l), "{what}: {l:02X?}");
    refused(list(&vec![score(); SND_SCORES_MAX + 1]), "nine scores");
    refused(patched(2, &[3]), "kind 3");
    refused(patched(3, &[0]), "no voice");
    refused(patched(3, &[4]), "four voices");
    refused(patched(12, &[0, 0]), "no bucket");
    let over = ScoreRecord { fingerprint: vec![0; zx_sys::SND_SCORE_BUCKETS_MAX + 1], events: vec![], ..score() };
    refused(list(&[over]), "a fingerprint over the most buckets");
    refused(patched(12, &[200, 0]), "a fingerprint past the record");
    refused(patched(14, &[5, 0]), "an event past the record");
    refused(patched(0, &[16, 0]), "a size under the fixed part");
    let l = list(&[score()]);
    let size = u16::from_le_bytes([l[4], l[5]]);
    refused(patched(0, &(size - 2).to_le_bytes()), "a size not a multiple of 4");
    refused(patched(0, &(size + 4).to_le_bytes()), "a size past the list");
    refused(patched(23 + 7, &[2]), "voice 2 of two");
    refused(patched(23 + 7, &[3]), "voice 3");
    refused(patched(23 + 7, &[0x41]), "a flag bit");
    refused(patched(23 + 8, &[63]), "a period under 64");
    refused(patched(23 + 5, &[0]), "level 0");
    refused(raw(1, 1, &[0x80, 0x80, 0x80, 0x80, 0x00, 0x80]), "a varint of five bytes");
    refused(raw(1, 1, &[0xC4, 0x08, 0x80]), "an event at the fingerprint's end");
    refused(raw(1, 1, &[0xFF, 0xFF, 0xFF, 0x7F, 0x80]), "an event far past the fingerprint's end");
    refused(raw(1, 2, &[0x00, 0x80, 0x00]), "an event cut off at its flags");
    refused(raw(1, 1, &[0x00, 0x00, 0x40]), "a note on cut off");
    refused(raw(1, 1, &[0x80, 0x80, 0x80]), "a varint running off the record");
    // one bad score refuses the list, its learned entries too
    let roles = BuiltinRoles { digest: 2, channels: 1, roles: [1, 0, 0] };
    refused(encode_list_scores(&[], &[roles], &[score(), ScoreRecord { voices: 0, ..score() }]), "a bad second score");
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
    a.label("end").r(0x18, "end");
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
    let mut prev = 0;
    let events = abs
        .iter()
        .map(|&(t, voice, on)| {
            let e = ScoreEvent { dt_t: t - prev, voice, on };
            prev = t;
            e
        })
        .collect();
    let site = labels["out_a"];
    let s = ScoreRecord {
        site,
        quiet: 1,
        code: score_code(|a| m.peek(a), site),
        roles: [SND_ROLE_LEAD, SND_ROLE_BASS, 0],
        voices: 2,
        fingerprint: score_fingerprint(&r.frames, f0, t0, buckets),
        events,
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

fn want(abs: &[Timed]) -> Vec<(u64, u8, u32, u8)> {
    abs.iter().map(|&(t, v, on)| (t, v, on.map_or(0, |o| o.0 << 8), on.map_or(0, |o| o.1))).collect()
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
        let live = score_fingerprint(&r.frames, f0, t0, s.fingerprint.len());
        assert!(live.iter().zip(&s.fingerprint).all(|(&l, &x)| agree(l, x)), "delay {delay}: {live:?} against {:?}", s.fingerprint);
        // the score claims every frame from T0's to the fingerprint's end, nothing else
        let frames = s.fingerprint.len() + usize::from(t0 != 0);
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
        let live = score_fingerprint(&r.frames, f0, t0, s.fingerprint.len());
        let bad: Vec<bool> = live.iter().zip(&s.fingerprint).map(|(&l, &x)| !agree(l, x)).collect();
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
        let frames = s.fingerprint.len() + usize::from(t0 != 0);
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
        assert_eq!((d.tone[0].period_q8, d.tone[0].level, d.tone[1].period_q8), ((2 * notes[2].half()) << 8, 200, 0), "delay {delay}");
    }
}

#[test]
fn score_releases_its_voices_where_its_fingerprint_ends() {
    let (notes, mut s, abs) = recorded();
    // no note off for the last note: it sounds to the fingerprint's end
    assert_eq!(s.events.pop().map(|e| (e.voice, e.on)), Some((0, None)));
    let mine = SndEngines::new().entries().len() as u8 + 1;
    let r = run(5000, &notes, FRAMES, &list(&[s.clone()]), None);
    let (f0, t0) = first_edge(&r);
    let end = f0 + s.fingerprint.len();
    assert_eq!(*claimed(&r, mine).last().unwrap(), end, "T0 at {t0}: the frame the fingerprint ends in");
    let mut expect = want(&abs[..abs.len() - 1]);
    expect.push((s.fingerprint.len() as u64 * F, 0, 0, 0));
    assert_eq!(played(&r, mine), expect);
    assert_eq!(r.dec[end].events().last().map(|e| e.t), Some(t0), "released at T0's T-state in that frame");
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
    let live = score_fingerprint(&r.frames, f0, t0, SND_DELAY_FRAMES as usize);
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
            let frames = s.fingerprint.len() + usize::from(t0 != 0);
            assert_eq!(claimed(&r, mine), (f0..f0 + frames).collect::<Vec<_>>(), "delay {delay}, tune {k}");
            assert_eq!(played(&r, mine), want(abs), "delay {delay}, tune {k}");
            assert_eq!(r.dec[f0].events()[0].t, t0, "delay {delay}, tune {k}");
            for o in (0..3).filter(|&o| o != k) {
                assert!(claimed(&r, first + o as u8).is_empty(), "delay {delay}: tune {k} never plays score {}", o + 1);
            }
            // the choice: its own score agrees throughout; A against B's run (and B against A's)
            // disagrees once, still followed when the choice is made (the fewest disagreeing
            // buckets win, not the lowest index); A and B against C's stop before it
            assert_eq!(early_misses(&r, &s.fingerprint), 0, "delay {delay}, tune {k}");
            let other = early_misses(&r, &scores[[1, 0, 0][k]].fingerprint);
            if k < 2 {
                assert!((1..3).contains(&other), "delay {delay}, tune {k}: {other} buckets disagree");
            } else {
                assert_eq!(other, 3, "delay {delay}, tune {k}");
                assert_eq!(early_misses(&r, &scores[1].fingerprint), 3, "delay {delay}, tune {k}");
            }
        }
    }
    // a lone score of the three plays its own tune as it did in the list, and nothing in the others
    let r = run_on(sel_machine(3000, 1), FRAMES, &list(&scores[1..2]), None);
    assert_eq!(played(&r, first), want(&rec[1].1), "B alone");
    let r = run_on(sel_machine(3000, 2), FRAMES, &list(&scores[1..2]), None);
    assert!(claimed(&r, first).is_empty(), "B alone, C's run");
}

/// Random corruptions of a score's record: the decoder refuses or takes each without reading
/// outside the list (run with `ZX_SANITIZE=1`), and method B plays what it takes.
#[test]
fn corrupted_scores_are_safe() {
    let (notes, s, _) = recorded();
    let good = list(&[ScoreRecord { fingerprint: s.fingerprint[..4].to_vec(), events: s.events[..4].to_vec(), ..s.clone() }]);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (mut taken, mut refused) = (0, 0);
    for _ in 0..3000 {
        let mut l = good.clone();
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
                if taken % 16 == 0 {
                    let mut m = machine(3000, &notes);
                    for _ in 0..12 {
                        m.run_frame();
                        b.frame(&m, m.edges(), m.edges_start_level());
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(taken > 0 && refused > 0, "{taken} taken, {refused} refused");
}
