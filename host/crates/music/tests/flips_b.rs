// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Method B's check of an engine whose channels flip one shared OUT (`core/sound/engines.c`,
//! flip), on `flips_sna`'s two-channel XOR engine: a note change between close pitches whose
//! first new flip is the second channel's, and an octave in step whose low channel's flips all
//! cancel in the XOR, are B's in every frame, with the melody's notes, and FM: engines never
//! drops to the beeper; a reader that reads one channel an octave off is still refused.

use fmzx_pack::testprogs::{flips_hz, flips_sna, FLIPS_MELODY, FLIPS_TABLE};
use music::{capture, hz_of_period_q8, RATE};
use std::cell::RefCell;
use std::rc::Rc;
use zx_sys::{Beeper, Machine, SndEngines, SndFrame, SndMixer, SndVoice, SND_DELAY_FRAMES, SND_MODE_FM_B, ZX_TR_INSN};

/// Where the engine sits (any address: its one absolute operand is relocated).
const BASE: u16 = 0xA157;
/// Frames run: 8 times the melody (25 frames).
const FRAMES: usize = 200;
/// The first melody's frames are left out (B starts on its first notes).
const FROM: usize = 25;
/// The melody's rows: (a) the close pitches, (b) the octave.
const CLOSE: usize = 2;
const OCTAVE: usize = 3;

/// A run of `flips_sna(BASE)` with B, and the mixer in FM: engines, fed every frame.
struct Run {
    /// B's decision about each frame.
    dec: Vec<SndFrame>,
    /// The melody row playing at each frame's end.
    row: Vec<usize>,
    /// Each frame's edges.
    edges: Vec<usize>,
    /// Frames in which a note begins with fewer than two of its edges: too few to see both
    /// channels, which the check refuses (a channel's level never known), whatever the voices.
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
    m.set_trace(Some(Box::new(move |ev| if ev.pc == BASE + 0x15 { s.borrow_mut().push(ev.t) })), 1 << ZX_TR_INSN);
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let (mut dec, mut row, mut edges, mut few, mut fm) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..FRAMES + SND_DELAY_FRAMES as usize {
        let f = capture(&mut m, &mut beeper, 1).pop().unwrap();
        // IX points at the note playing (past the last row, for a moment: the first again)
        row.push(usize::from(m.regs().ix.wrapping_sub(BASE + FLIPS_TABLE) / 3) % FLIPS_MELODY.len());
        edges.push(f.edges.len());
        few.push(starts.borrow_mut().drain(..).any(|t| f.edges.iter().filter(|e| e.tstate > t).count() < 2));
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
    Run { dec, row, edges, few, fm }
}

fn test_mixer() -> SndMixer {
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_test_engines();
    mixer
}

impl Run {
    /// The frames from FROM on with edges that play some of melody row r.
    fn frames_of(&self, r: usize) -> Vec<usize> {
        (FROM..FRAMES).filter(|&f| self.edges[f] > 0 && (self.row[f] == r || self.row[f - 1] == r)).collect()
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
    assert!(few.len() <= frames.len() / 20, "{} frames with too few edges", few.len());
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
fn flips_misread_octave_refused() {
    // channel 1 read an octave up (in the octave, at the high channel's pitch, so its flips would
    // fall on every flip of channel 0): no missing flip may be explained by it, and no frame of
    // the melody is B's
    let r = run(SndEngines::new_misread(), test_mixer());
    let frames: Vec<usize> = (FROM..FRAMES).filter(|&f| r.edges[f] > 0).collect();
    let claimed: Vec<usize> = frames.iter().copied().filter(|&f| r.dec[f].confident != 0).collect();
    println!("misread: {} frames, B's {claimed:?}", frames.len());
    assert!(r.frames_of(OCTAVE).len() >= 50);
    assert!(claimed.is_empty(), "{} of {} frames B's with channel 1 misread: {claimed:?}", claimed.len(), frames.len());
}
