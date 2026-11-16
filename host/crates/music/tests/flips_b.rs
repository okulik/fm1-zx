// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Method B's check of an engine whose channels flip one shared OUT (`core/sound/engines.c`,
//! flip), on `flips_sna`'s two-channel XOR engine: a note change between close pitches whose
//! first new flip is the second channel's, and an octave in step whose low channel's flips all
//! cancel in the XOR (with the high voice on either channel), are B's in every frame, with the
//! melody's notes, and FM: engines never drops to the beeper; readers that read one channel an
//! octave off are still refused, even where a new note shows only two edges.

use fmzx_pack::testprogs::{flips_hz, flips_sna, FLIPS_MELODY, FLIPS_PASS, FLIPS_TABLE};
use music::{capture, hz_of_period_q8, RATE};
use std::cell::RefCell;
use std::rc::Rc;
use zx_sys::{
    Beeper, Machine, SndEngines, SndFrame, SndMixer, SndVoice, MISREAD_FLIPS_0_HALF, MISREAD_FLIPS_1_DOUBLE, MISREAD_FLIPS_1_HALF, SND_DELAY_FRAMES,
    SND_MODE_FM_B, ZX_FRAME_TSTATES, ZX_TR_INSN,
};

/// Where the engine sits (any address: its one absolute operand is relocated).
const BASE: u16 = 0xA157;
/// Frames run: about 8 times the melody (38 frames).
const FRAMES: usize = 300;
/// The first melody's frames are left out (B starts on its first notes).
const FROM: usize = 40;
/// The melody's rows: (a) the close pitches, (b) the octave, the note whose channel 1 is slow,
/// and the octave with the high voice on channel 1.
const CLOSE: usize = 2;
const OCTAVE: usize = 3;
const SLOW_1: usize = 5;
const OCTAVE_HIGH_1: usize = 6;

/// A run of `flips_sna(BASE)` with B, and the mixer in FM: engines, fed every frame.
struct Run {
    /// B's decision about each frame.
    dec: Vec<SndFrame>,
    /// The melody row playing at each frame's end.
    row: Vec<usize>,
    /// Each frame's edges.
    edges: Vec<usize>,
    /// Per frame, the notes that begin in it: their melody row and how many of the frame's edges
    /// come after their start.
    starts: Vec<Vec<(usize, usize)>>,
    /// Per frame, where the notes begin and the edges' times.
    times: Vec<(Vec<u32>, Vec<u32>)>,
    /// Frames in which a note begins with too few of its edges to see both channels, which the
    /// check refuses (a channel's level never known), whatever the voices: a channel whose first
    /// flip comes after the frame's end, or fewer than three edges (an octave's hidden channel
    /// is known only when a second hidden flip fits the first, guessed one).
    few: Vec<bool>,
    /// The mixer plays FM for the frame.
    fm: Vec<bool>,
}

fn run(mut b: SndEngines, mut mixer: SndMixer) -> Run {
    let mut m = Machine::new();
    m.load_sna(&flips_sna(BASE)).expect("loads");
    // a note begins at the LD D,H after the row's reads
    let starts = Rc::new(RefCell::new(Vec::new()));
    let s = starts.clone();
    let row_of = |ix: u16| usize::from(ix.wrapping_sub(BASE + FLIPS_TABLE) / 3) % FLIPS_MELODY.len();
    m.set_trace(Some(Box::new(move |ev| if ev.pc == BASE + 0x15 { s.borrow_mut().push((ev.t, row_of(ev.regs.ix))) })), 1 << ZX_TR_INSN);
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let (mut dec, mut row, mut edges, mut few, mut fm, mut begun, mut times) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..FRAMES + SND_DELAY_FRAMES as usize {
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        // IX points at the note playing (past the last row, for a moment: the first again)
        row.push(row_of(m.regs().ix));
        edges.push(f.edges.len());
        let here: Vec<(u32, usize, usize)> = starts.borrow_mut().drain(..).map(|(t, r)| (t, r, f.edges.iter().filter(|e| e.tstate > t).count())).collect();
        let late = |t: u32, n: u8| f64::from(t) + f64::from(n) * FLIPS_PASS + 82.0 > f64::from(ZX_FRAME_TSTATES);
        few.push(here.iter().any(|&(t, r, n)| n < 3 || late(t, FLIPS_MELODY[r].0) || late(t, FLIPS_MELODY[r].1)));
        begun.push(here.iter().map(|&(_, r, n)| (r, n)).collect());
        times.push((here.iter().map(|&(t, _, _)| t).collect(), f.edges.iter().map(|e| e.tstate).collect()));
        mixer.frame(SND_MODE_FM_B, Some(&m), &f.edges, f.start_level, &f.beeper);
        let d = b.frame(&m, &f.edges, f.start_level);
        if k >= SND_DELAY_FRAMES as usize {
            dec.push(d);
            fm.push(mixer.fm_on());
        }
    }
    row.truncate(FRAMES);
    edges.truncate(FRAMES);
    few.truncate(FRAMES);
    begun.truncate(FRAMES);
    times.truncate(FRAMES);
    Run { dec, row, edges, starts: begun, times, few, fm }
}

fn test_mixer() -> SndMixer {
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_test_engines();
    mixer
}

impl Run {
    /// The frames from FROM on with edges of melody row r that the check looks at.
    fn frames_of(&self, r: usize) -> Vec<usize> {
        (FROM..FRAMES).filter(|&f| self.edges_of(f, r) > 0).collect()
    }

    /// Frame f's edges while melody row r plays, but those the check leaves out: within 400 T
    /// (engines.c's MARGIN) of a note's start.
    fn edges_of(&self, f: usize, r: usize) -> usize {
        let (starts, edges) = &self.times[f];
        let row_at = |t: u32| self.starts[f].iter().zip(starts).filter(|(_, &s)| s <= t).last().map_or(self.row[f - 1], |((row, _), _)| *row);
        edges.iter().filter(|&&t| row_at(t) == r && starts.iter().all(|&s| t.abs_diff(s) >= 400)).count()
    }

    /// Those of `frames` that B does not claim, but for a frame with too few of a new note's
    /// edges.
    fn unclaimed(&self, frames: &[usize]) -> Vec<usize> {
        frames.iter().copied().filter(|&f| self.dec[f].confident == 0 && !self.few[f]).collect()
    }
}

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

/// Do voices v play melody row r?
fn plays(v: &[SndVoice; 3], r: usize) -> bool {
    let (n0, n1, _) = FLIPS_MELODY[r];
    let near = |v: SndVoice, n: u8| v.period_q8 != 0 && v.level != 0 && (hz_of_period_q8(v.period_q8) / flips_hz(n) - 1.0).abs() < 0.001;
    near(v[0], n0) && near(v[1], n1) && v[2].period_q8 == 0
}

#[test]
fn flips_entry_matches_its_code() {
    // the entry's digest is its signature's wherever the code sits
    let b = SndEngines::new_test();
    let idx = b.entries().iter().position(|e| e.name == "Test flips").expect("Test flips");
    let want = b.entries()[idx].digest;
    for base in [0x8000u16, BASE, 0xF000] {
        let mut m = Machine::new();
        m.load_sna(&flips_sna(base)).expect("loads");
        let got = b.digest(&m, idx, base);
        assert_eq!(got, want, "at {base:04X}: digest {got:#018x}");
    }
}

#[test]
fn flips_close_pitches_after_pause() {
    // a new note of two close pitches (11% apart): its first edge is channel 1's flip, taken by
    // order for channel 0's; every frame of the note is still B's
    let r = run(SndEngines::new_test(), test_mixer());
    let frames = r.frames_of(CLOSE);
    let off = r.unclaimed(&frames);
    println!("close pitches: frames {frames:?}, not B's {off:?}");
    assert!(frames.len() >= 30, "{} frames", frames.len());
    assert!(off.is_empty(), "{} of {} frames of the close pitches not B's: {off:?}", off.len(), frames.len());
}

#[test]
fn flips_octave_cancellation() {
    // an octave in step: every flip of the low channel falls in a pass with one of the high
    // channel's and cancels, so the edges are the high channel's alone, every other one missing;
    // every frame of the note is still B's
    let r = run(SndEngines::new_test(), test_mixer());
    let frames = r.frames_of(OCTAVE);
    let off = r.unclaimed(&frames);
    println!("octave: frames {frames:?}, not B's {off:?}");
    assert!(frames.len() >= 50, "{} frames", frames.len());
    assert!(off.is_empty(), "{} of {} frames of the octave not B's: {off:?}", off.len(), frames.len());
}

#[test]
fn flips_notes_are_the_melodys() {
    // every frame of the melody is B's (but those a new note has too few edges in), and the
    // voices it ends with are the table's notes (the note playing at the frame's end, or the one
    // before: its last edge may come before the change)
    let r = run(SndEngines::new_test(), test_mixer());
    let frames: Vec<usize> = (FROM..FRAMES).filter(|&f| r.edges[f] > 0).collect();
    let off = r.unclaimed(&frames);
    let few: Vec<usize> = frames.iter().copied().filter(|&f| r.few[f]).collect();
    let wrong: Vec<usize> = frames.iter().copied().filter(|&f| r.dec[f].confident != 0 && !plays(&end_voices(&r.dec[f]), r.row[f]) && !plays(&end_voices(&r.dec[f]), r.row[f - 1])).collect();
    println!("{} frames, not B's {off:?}, too few of a new note's edges {few:?}, wrong notes {wrong:?}", frames.len());
    assert!(few.len() <= frames.len() / 10, "{} frames with too few edges", few.len());
    assert!(wrong.is_empty(), "wrong notes in frames {wrong:?}: {:?}", wrong.iter().map(|&f| end_voices(&r.dec[f])).collect::<Vec<_>>());
    assert!(off.is_empty(), "{} of {} frames not B's: {off:?}", off.len(), frames.len());
}

#[test]
fn flips_stay_fm() {
    // FM: engines plays the whole melody as FM: no drop to the beeper (each a click out and a
    // click back), not even at a frame B does not claim
    let r = run(SndEngines::new_test(), test_mixer());
    let off: Vec<usize> = (FROM..FRAMES).filter(|&f| !r.fm[f]).collect();
    println!("beeper in frames {off:?}");
    assert!(off.is_empty(), "beeper in {} frames: {off:?}", off.len());
}

#[test]
fn flips_octave_high_on_channel_1() {
    // the octave the other way round: the high voice on channel 1, so every flip of channel 0
    // cancels and the edges, one low half-period apart, fit channel 0 alone; every frame of the
    // note is still B's
    let r = run(SndEngines::new_test(), test_mixer());
    let frames = r.frames_of(OCTAVE_HIGH_1);
    let off = r.unclaimed(&frames);
    println!("octave, high voice on channel 1: frames {frames:?}, not B's {off:?}");
    assert!(frames.len() >= 40, "{} frames", frames.len());
    assert!(off.is_empty(), "{} of {} frames of the octave not B's: {off:?}", off.len(), frames.len());
}

/// The frames from FROM on with edges that B claims, with `entry` of the misread table.
fn claimed_misread(entry: usize) -> (Run, Vec<usize>) {
    let r = run(SndEngines::new_misread(entry), test_mixer());
    let claimed = (FROM..FRAMES).filter(|&f| r.edges[f] > 0 && r.dec[f].confident != 0).collect();
    (r, claimed)
}

#[test]
fn flips_misread_octave_refused() {
    // channel 1 read an octave up (in the octave, at the high channel's pitch, so its flips would
    // fall on every flip of channel 0): no missing flip may be explained by it, and no frame of
    // the melody is B's
    let (r, claimed) = claimed_misread(MISREAD_FLIPS_1_HALF);
    println!("channel 1 an octave up: B's {claimed:?}");
    assert!(r.frames_of(OCTAVE).len() >= 50);
    assert!(claimed.is_empty(), "{} frames B's with channel 1 misread: {claimed:?}", claimed.len());
}

#[test]
fn flips_misread_low_octave_refused() {
    // channel 1 read an octave down: no frame of the melody is B's, but for the note whose
    // channel 1 is slow. There every other flip of channel 1 fits its misread period, and each one
    // between fits no channel and is charged to the nearest, channel 0, whose many good pairs keep
    // it under the one-in-eight share: the check has let that through since before the flips
    // rules of this file (a slow channel's misread hides behind a busy one)
    let (r, claimed) = claimed_misread(MISREAD_FLIPS_1_DOUBLE);
    let slow = r.frames_of(SLOW_1);
    let wrong: Vec<usize> = claimed.iter().copied().filter(|f| !slow.contains(f)).collect();
    println!("channel 1 an octave down: B's {claimed:?}, outside the slow note {wrong:?}");
    assert!(wrong.is_empty(), "{} frames B's with channel 1 misread: {wrong:?}", wrong.len());
}

#[test]
fn flips_misread_two_edges_refused() {
    // channel 0 read an octave up in the slow note: its flips one apart are read two apart, and
    // the flip between could be channel 1's hidden one. Where the note shows only two of channel
    // 0's flips (begun late in a frame), that one guess is all the evidence channel 1 has: it must
    // not count as seeing channel 1, so no frame of the slow note is B's
    let (r, claimed) = claimed_misread(MISREAD_FLIPS_0_HALF);
    let two: Vec<usize> = (FROM..FRAMES).filter(|&f| r.starts[f].iter().any(|&(row, n)| row == SLOW_1 && n == 2)).collect();
    let slow = r.frames_of(SLOW_1);
    let wrong: Vec<usize> = claimed.iter().copied().filter(|f| slow.contains(f)).collect();
    println!("channel 0 an octave up in the slow note: its frames B's {wrong:?}; it begins with two edges in {two:?}");
    assert!(!two.is_empty(), "no frame where the slow note begins with two edges");
    assert!(wrong.is_empty(), "{} frames of the slow note B's with channel 0 misread: {wrong:?}", wrong.len());
}
