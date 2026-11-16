// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Learned entries (`core/sound/learned.c`) in method B: the "Test burst" entry of
//! `core/sound/engines_test.c` re-expressed as data decides every frame of `burst_sna` as the
//! built-in reader does, the learned entry follows its code to another address but not into
//! changed code, and the mixer plays FM: engines from learned entries. Stored roles: a learned
//! entry's `instrument` bytes and a kind-1 record (roles for a built-in entry) give B's claimed
//! frames their channels' roles; the live rule fills the rest.

use fmzx_pack::testprogs::{beep_tap, burst_sna};
use soundlearn::{encode_list, LearnedEntry, Op};
use music::{colony8, RATE};
use zx_sys::{
    Beeper, Machine, SndEngines, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_MODE_FM_B, SND_ROLE_AUTO, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD,
    SND_SITE_TONAL,
};

/// The "Test burst" entry as data: period 2 (13n + 54) x 256 from the operand n at +13h, silent
/// for n = 0, as `read_burst`.
fn burst_entry() -> LearnedEntry {
    LearnedEntry {
        digest: 0xd4b5_51f4_6da9_de4c,
        len: 0x2C,
        wild: vec![0x13],
        reloc: vec![0x02, 0x09],
        sites: vec![(0x1A, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        drum: 0,
        stale: None,
        level: vec![200],
        instrument: [0; 3],
        progs: vec![vec![Op::PeekRel(0x13), Op::ZeroSilent, Op::PeekRel(0x13), Op::K(13), Op::Mul, Op::K(54), Op::Add, Op::K(512), Op::Mul]],
        timing: None,
    }
}

/// Method B's decision about each of `frames` frames of `m` (the last SND_DELAY_FRAMES are still
/// to come when the run ends).
fn decisions(mut m: Machine, mut b: SndEngines, frames: usize) -> Vec<SndFrame> {
    (0..frames)
        .map(|_| {
            m.run_frame();
            b.frame(&m, m.edges(), m.edges_start_level())
        })
        .collect()
}

/// `burst_sna` at 8000h with its melody replaced: each of `notes` (n, bursts) a burst of 6 edges
/// every 69,000 T (just under a frame, so one burst in nearly every frame), `bursts` times in a
/// row at the same pitch: a held note the routine plays in short bursts with pauses between, as
/// a game's interrupt-paced music does.
fn burst_held(notes: &[(u8, usize)]) -> Machine {
    let mut sna = burst_sna(0x8000);
    let table = 27 + (0x8000 + 0x2C - 0x4000);
    let mut t = Vec::new();
    for &(n, bursts) in notes {
        let half = 13 * u32::from(n) + 54;
        let gap = ((69_000 - 98 - 6 * half) / 26) as u16;
        for _ in 0..bursts {
            t.push(n);
            t.extend_from_slice(&6u16.to_le_bytes());
            t.extend_from_slice(&gap.to_le_bytes());
        }
    }
    t.push(0);
    sna[table..table + t.len()].copy_from_slice(&t);
    let mut m = Machine::new();
    m.load_sna(&sna).expect("loads");
    m
}

#[test]
fn held_note_in_bursts_stays_confident() {
    // a note held over several frames' bursts is B's on every one of them, not only the first:
    // the pause between bursts (far longer than a period) is the engine stopping, not a
    // contradiction of the note read
    let list = encode_list(&[burst_entry()]);
    let mut m = burst_held(&[(200, 8), (168, 8), (124, 8), (168, 8)]);
    let mut b = SndEngines::with_learned(&list).expect("decodes");
    let (mut edges, mut sounding, mut unclaimed) = (Vec::new(), 0, Vec::new());
    for k in 0..400usize {
        m.run_frame();
        edges.push(m.edges().len());
        let d = b.frame(&m, m.edges(), m.edges_start_level());
        // the decision is about the frame SND_DELAY_FRAMES back
        let Some(f) = k.checked_sub(SND_DELAY_FRAMES as usize) else { continue };
        if f >= 10 && edges[f] > 0 {
            sounding += 1;
            if d.confident == 0 {
                unclaimed.push(f);
            }
        }
    }
    assert!(sounding > 300, "{sounding} sounding decisions");
    assert!(unclaimed.is_empty(), "{} of {sounding} decisions with a note not B's, first at {:?}", unclaimed.len(), &unclaimed[..unclaimed.len().min(8)]);
}

fn burst(base: u16) -> Machine {
    let mut m = Machine::new();
    m.load_sna(&burst_sna(base)).expect("loads");
    m
}

#[test]
fn learned_burst_equals_builtin() {
    let list = encode_list(&[burst_entry()]);
    let builtin = decisions(burst(0x8000), SndEngines::new_test(), 600);
    let learned = decisions(burst(0x8000), SndEngines::with_learned(&list).expect("decodes"), 600);
    let mut confident = 0;
    for (i, (a, b)) in builtin.iter().zip(&learned).enumerate() {
        assert_eq!((a.confident, a.tone, a.drum), (b.confident, b.tone, b.drum), "decision {i}");
        assert_eq!((a.engine, a.events()), (b.engine, b.events()), "decision {i}");
        confident += usize::from(a.confident != 0);
    }
    println!("{confident} confident decisions of 600");
    assert!(confident >= 100, "{confident} confident decisions");
    let b = SndEngines::with_learned(&list).unwrap();
    assert_eq!(b.name(learned.iter().find(|d| d.confident != 0).unwrap().engine), "learned");
}

/// burst_sna's code and melody (at 8000h) copied to `to`, its two absolute words re-relocated
/// (+02h: the melody at +2Ch, +09h: the operand at +13h), and run from there.
fn moved(to: u16) -> Machine {
    const LEN: u16 = 0x2C + 8 * 5 + 1;
    let mut m = burst(0x8000);
    for i in 0..LEN {
        m.poke(to + i, m.peek(0x8000 + i));
        m.poke(0x8000 + i, 0);
    }
    for (at, word) in [(0x02u16, to + 0x2C), (0x09, to + 0x13)] {
        m.poke(to + at, word as u8);
        m.poke(to + at + 1, (word >> 8) as u8);
    }
    let mut r = m.regs();
    r.pc = to;
    m.set_regs(&r);
    m
}

fn claimed(d: &[SndFrame]) -> usize {
    d.iter().filter(|d| d.confident != 0).count()
}

#[test]
fn learned_entry_moves_with_code() {
    const TO: u16 = 0xA123;
    let entry = burst_entry();
    let list = encode_list(std::slice::from_ref(&entry));
    let n = claimed(&decisions(moved(TO), SndEngines::with_learned(&list).unwrap(), 300));
    println!("moved to {TO:04X}: {n} frames claimed");
    assert!(n >= 50, "moved: {n} frames claimed");

    // the same code with the LD C,0 operand at +11h changed (the speaker starts high: the same
    // tones): a signature byte, unless the entry leaves it out
    let changed = || {
        let mut m = moved(TO);
        m.poke(TO + 0x11, 0x10);
        m
    };
    let mut wider = LearnedEntry { wild: vec![0x11, 0x13], digest: 0, ..entry.clone() };
    let probe = SndEngines::with_learned(&encode_list(std::slice::from_ref(&wider))).unwrap();
    let index = probe.entries().len() - 1;
    wider.digest = probe.digest(&changed(), index, TO);
    assert_eq!(probe.digest(&moved(TO), index, TO), wider.digest, "the byte at +11h is left out");
    let n = claimed(&decisions(changed(), SndEngines::with_learned(&encode_list(&[wider])).unwrap(), 300));
    println!("+11h changed, left out: {n} frames claimed");
    assert!(n >= 50, "a wildcard byte changed: {n} frames claimed");

    let mut m = changed();
    let mut edges = 0;
    let mut b = SndEngines::with_learned(&list).unwrap();
    let mut n = 0;
    for _ in 0..300 {
        m.run_frame();
        edges += m.edges().len();
        n += usize::from(b.frame(&m, m.edges(), m.edges_start_level()).confident != 0);
    }
    println!("+11h changed, in the signature: {n} frames claimed ({edges} edges)");
    assert!(edges > 500, "the changed code still plays ({edges} edges)");
    assert_eq!(n, 0, "a signature byte changed: {n} frames claimed");
}

/// `burst_sna` at 8000h with its melody replaced: each of `notes` one burst of 6 edges, then a
/// rest, a note every `frames` frames (at most 24: the gap loop's count is 16 bits).
fn burst_spaced(notes: &[u8], frames: u32) -> Machine {
    let mut sna = burst_sna(0x8000);
    let table = 27 + (0x8000 + 0x2C - 0x4000);
    let mut t = Vec::new();
    for &n in notes {
        let half = 13 * u32::from(n) + 54;
        t.push(n);
        t.extend_from_slice(&6u16.to_le_bytes());
        t.extend_from_slice(&(((frames * 69_888 - 98 - 6 * half) / 26) as u16).to_le_bytes());
    }
    t.push(0);
    sna[table..table + t.len()].copy_from_slice(&t);
    let mut m = Machine::new();
    m.load_sna(&sna).expect("loads");
    m
}

#[test]
fn rests_stay_fm() {
    // a tune whose notes are short bursts with rests of 20 frames (400 ms) between them, and a
    // stray speaker edge from other code (a ROM address) in the middle of some rests: FM: engines
    // plays FM throughout. The engine holds through its rests, and a lone frame of other code's
    // does not end the hold (the mixer passes over one unconfident frame), so the beeper never
    // takes the rests and the stray edge is not played
    let mut m = burst_spaced(&[200, 168, 124, 168], 20);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_learned(&encode_list(&[burst_entry()])).expect("decodes");
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let (mut beeper_frames, mut strays) = (Vec::new(), 0);
    for k in 0..400usize {
        m.run_frame();
        let mut edges = m.edges().to_vec();
        if edges.is_empty() && k % 40 == 30 {
            let level = m.edges_start_level() ^ 2;
            edges.push(zx_sys::zx_edge { tstate: 30_000, level, pc: 0x0D6B });
            strays += 1;
        }
        beeper.set_level(m.edges_start_level());
        let samples = beeper.render(&edges);
        mixer.frame(SND_MODE_FM_B, Some(&m), &edges, m.edges_start_level(), &samples);
        if k >= 30 && !mixer.fm_on() {
            beeper_frames.push(k);
        }
    }
    assert!(strays >= 5, "{strays} stray edges");
    assert!(beeper_frames.is_empty(), "{} beeper frames, first at {:?}", beeper_frames.len(), &beeper_frames[..beeper_frames.len().min(8)]);
}

#[test]
fn a_stray_edge_in_a_note_stays_b() {
    // a speaker edge from other code (a ROM address) among a note's edges, as a game's code that
    // sets the speaker low after each call of its music routine makes one: the frame stays B's,
    // with the same decision as without it; two such edges are other code's sound (an effect)
    // and the frame is not B's
    let list = encode_list(&[burst_entry()]);
    let stray = |n: usize| {
        let mut m = burst(0x8000);
        let mut b = SndEngines::with_learned(&list).expect("decodes");
        let (mut out, mut hit) = (Vec::new(), Vec::new());
        for k in 0..300usize {
            m.run_frame();
            let mut edges = m.edges().to_vec();
            if n > 0 && edges.len() >= 4 && k % 3 == 0 && k < 290 {
                // just after the burst's first edge, the level the edge before left
                let (t, level) = (edges[0].tstate + 20, edges[0].level ^ 2);
                for j in 0..n {
                    edges.insert(1 + j, zx_sys::zx_edge { tstate: t + 10 * j as u32, level: level ^ (2 * (j as u8 & 1)), pc: 0x0D6B });
                }
                hit.push(k);
            }
            out.push(b.frame(&m, &edges, m.edges_start_level()));
        }
        (out, hit)
    };
    let (clean, _) = stray(0);
    let (one, hit) = stray(1);
    let delay = SND_DELAY_FRAMES as usize;
    let claimed = |d: &[SndFrame]| hit.iter().filter(|&&k| d[k + delay].confident != 0).count();
    assert!(hit.len() >= 10, "{} frames with a stray edge", hit.len());
    assert_eq!(claimed(&clean), hit.len(), "the frames are B's without the stray edge");
    let lost: Vec<usize> = hit.iter().copied().filter(|&k| one[k + delay].confident == 0).collect();
    assert!(lost.is_empty(), "{} of {} frames with one stray edge not B's: {lost:?}", lost.len(), hit.len());
    for &k in &hit {
        assert_eq!(one[k + delay].tone, clean[k + delay].tone, "frame {k}: the same voices");
    }
    let (two, _) = stray(2);
    assert_eq!(claimed(&two), 0, "two stray edges in a frame are other code's sound");
}

#[test]
fn mixer_plays_learned_entries() {
    // frames the mixer plays FM for in FM: engines, on burst_sna, with the built-in table and then
    // with the burst entry learned
    let fm_frames = |mixer: &mut SndMixer| {
        let mut m = burst(0x8000);
        let mut beeper = Beeper::new(RATE.0, RATE.1);
        (0..200)
            .filter(|_| {
                m.run_frame();
                beeper.set_level(m.edges_start_level());
                let samples = beeper.render(m.edges());
                mixer.frame(SND_MODE_FM_B, Some(&m), m.edges(), m.edges_start_level(), &samples);
                mixer.fm_on()
            })
            .count()
    };
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    assert_eq!(fm_frames(&mut mixer), 0, "the built-in table does not know burst_sna");
    assert!(mixer.set_learned(&[9, 0, 0, 0]).is_err(), "a count of 9");
    assert_eq!(fm_frames(&mut mixer), 0, "a refused list leaves the engines as they were");
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_learned(&encode_list(&[burst_entry()])).expect("decodes");
    let n = fm_frames(&mut mixer);
    println!("FM for {n} of 200 frames");
    assert!(n >= 150, "FM for {n} of 200 frames");
}

// ---- stored roles ---------------------------------------------------------------------------

const A: u8 = SND_ROLE_AUTO;
const B: u8 = SND_ROLE_BASS;
const L: u8 = SND_ROLE_LEAD;
const P: u8 = SND_ROLE_PAD;

/// The burst entry with `channels` channels, the others' programs and levels copies of channel
/// 0's (decodes; only the decoder sees it).
fn burst_channels(channels: usize) -> LearnedEntry {
    let mut e = burst_entry();
    e.level = vec![200; channels];
    e.pos = vec![0; channels];
    e.progs = vec![e.progs[0].clone(); channels];
    e
}

/// The stored roles of the last entry of the table `list` makes.
fn decoded_roles(list: &[u8]) -> [u8; 3] {
    SndEngines::with_learned(list).expect("decodes").entries().last().unwrap().role
}

/// `list` (an entry list) with entry `index`'s `instrument` bytes set.
fn with_instruments(mut list: Vec<u8>, index: usize, instrument: [u8; 3]) -> Vec<u8> {
    let mut off = 4;
    for _ in 0..index {
        off += usize::from(u16::from_le_bytes([list[off], list[off + 1]]));
    }
    list[off + 29..off + 32].copy_from_slice(&instrument);
    list
}

/// A kind-1 record: roles for the built-in entry `digest` (16 bytes).
fn kind1(digest: u64, channels: u8, role: [u8; 3]) -> Vec<u8> {
    let mut b = vec![16, 0, 1, channels];
    b.extend(digest.to_le_bytes());
    b.extend(role);
    b.push(0);
    b
}

/// An entry list of already encoded entries and records.
fn list_of(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![parts.len() as u8, 0, 0, 0];
    for p in parts {
        b.extend(p);
    }
    b
}

/// The encoded entries of `list` (without its 4-byte head).
fn body(list: &[u8]) -> Vec<u8> {
    list[4..].to_vec()
}

/// The roles of B's claimed (confident) frames, each different set once, in order; and checks
/// that every unclaimed frame carries none (all AUTO).
fn claimed_roles(d: &[SndFrame]) -> Vec<[u8; 3]> {
    let mut seen: Vec<[u8; 3]> = Vec::new();
    for (i, f) in d.iter().enumerate() {
        if f.confident == 0 {
            assert_eq!(f.role, [A; 3], "unclaimed frame {i} carries roles");
        } else if seen.last() != Some(&f.role) {
            seen.push(f.role);
        }
    }
    seen
}

#[test]
fn instrument_bytes_become_roles() {
    // the decoder: instrument[c] is channel c's role, and a channel the entry lacks has none
    assert_eq!(decoded_roles(&with_instruments(encode_list(&[burst_channels(3)]), 0, [1, 2, 0])), [B, L, A]);
    assert_eq!(decoded_roles(&with_instruments(encode_list(&[burst_channels(3)]), 0, [3, 3, 2])), [P, P, L]);
    assert_eq!(decoded_roles(&with_instruments(encode_list(&[burst_channels(2)]), 0, [3, 1, 2])), [P, B, A]);
    assert_eq!(decoded_roles(&encode_list(&[burst_channels(3)])), [A; 3]);
    let two = encode_list(&[burst_entry(), burst_channels(2)]);
    assert_eq!(decoded_roles(&with_instruments(two, 1, [2, 1, 1])), [L, B, A], "the second entry's bytes");
    for e in SndEngines::new().entries() {
        assert_eq!(e.role, [A; 3], "{}: a built-in entry has no stored role", e.name);
    }

    // B: the frames the entry claims carry its roles, its unclaimed ones none (one channel:
    // instrument [1, 2, 0] gives channel 0 the bass)
    let list = with_instruments(encode_list(&[burst_entry()]), 0, [1, 2, 0]);
    let roled = decisions(burst(0x8000), SndEngines::with_learned(&list).expect("decodes"), 300);
    assert!(claimed(&roled) >= 50, "{} frames claimed", claimed(&roled));
    assert_eq!(claimed_roles(&roled), [[B, A, A]], "instrument [1, 2, 0]");
    let list = with_instruments(encode_list(&[burst_entry()]), 0, [2, 0, 0]);
    assert_eq!(claimed_roles(&decisions(burst(0x8000), SndEngines::with_learned(&list).unwrap(), 300)), [[L, A, A]]);

    // the roles change no decision: the frames are those of the entry without them
    let plain = decisions(burst(0x8000), SndEngines::with_learned(&encode_list(&[burst_entry()])).unwrap(), 300);
    assert_eq!(claimed_roles(&plain), [[A; 3]]);
    for (i, (a, b)) in plain.iter().zip(&roled).enumerate() {
        assert_eq!((a.confident, a.tone, a.drum, a.engine, a.events()), (b.confident, b.tone, b.drum, b.engine, b.events()), "decision {i}");
    }
}

#[test]
fn instrument_above_3_is_auto() {
    for bad in [4, 5, 0x80, 0xFF] {
        assert_eq!(decoded_roles(&with_instruments(encode_list(&[burst_channels(3)]), 0, [bad, 1, bad])), [A, B, A], "instrument [{bad}, 1, {bad}]");
        let list = with_instruments(encode_list(&[burst_entry()]), 0, [bad, 0, 0]);
        let d = decisions(burst(0x8000), SndEngines::with_learned(&list).expect("decodes"), 300);
        assert!(claimed(&d) >= 50);
        assert_eq!(claimed_roles(&d), [[A; 3]], "instrument [{bad}, 0, 0]");
    }
}

/// The ROM BEEP entry's digest (the built-in table's).
fn rom_beep_digest() -> u64 {
    SndEngines::new().entries().into_iter().find(|e| e.name == "ROM BEEP").expect("ROM BEEP").digest
}

/// B's decisions about 300 frames of `beep_tap()` (its melody is played by the ROM's BEEP).
fn beep_decisions(b: SndEngines) -> Vec<SndFrame> {
    let mut m = Machine::new();
    m.tape_insert(&beep_tap()).expect("inserts");
    m.tape_autostart();
    decisions(m, b, 300)
}

#[test]
fn kind1_sets_builtin_roles() {
    // a list of one kind-1 record and no learned entry: the built-in ROM BEEP plays bass
    let list = list_of(&[kind1(rom_beep_digest(), 1, [1, 0, 0])]);
    let d = beep_decisions(SndEngines::with_learned(&list).expect("decodes"));
    let n = claimed(&d);
    println!("{n} frames claimed by ROM BEEP");
    assert!(n >= 20, "{n} frames claimed");
    assert_eq!(claimed_roles(&d), [[B, A, A]]);
    // the same frames as without it
    let plain = beep_decisions(SndEngines::new());
    assert_eq!(claimed_roles(&plain), [[A; 3]]);
    for (i, (a, b)) in plain.iter().zip(&d).enumerate() {
        assert_eq!((a.confident, a.tone, a.engine, a.events()), (b.confident, b.tone, b.engine, b.events()), "decision {i}");
    }

    // beside a learned entry, a role above 3 or for a channel ROM BEEP lacks is none, and the
    // last record for a digest wins
    let list = list_of(&[kind1(rom_beep_digest(), 1, [2, 0, 0]), body(&encode_list(&[burst_entry()])), kind1(rom_beep_digest(), 3, [3, 1, 1])]);
    assert_eq!(claimed_roles(&beep_decisions(SndEngines::with_learned(&list).unwrap())), [[P, A, A]]);
    let list = list_of(&[kind1(rom_beep_digest(), 1, [9, 0, 0])]);
    assert_eq!(claimed_roles(&beep_decisions(SndEngines::with_learned(&list).unwrap())), [[A; 3]]);
}

#[test]
fn kind1_unknown_digest_ignored() {
    // a digest no entry has; the learned burst entry's own digest (a kind-1 record is for
    // built-in entries only): both decode and change nothing
    let entry = encode_list(&[burst_entry()]);
    for digest in [0x0123_4567_89ab_cdef, burst_entry().digest] {
        let list = list_of(&[body(&entry), kind1(digest, 1, [1, 1, 1])]);
        let d = decisions(burst(0x8000), SndEngines::with_learned(&list).expect("decodes"), 300);
        assert!(claimed(&d) >= 50);
        assert_eq!(claimed_roles(&d), [[A; 3]], "digest {digest:016x}");
    }
    let list = list_of(&[kind1(0x0123_4567_89ab_cdef, 1, [1, 0, 0])]);
    assert_eq!(claimed_roles(&beep_decisions(SndEngines::with_learned(&list).expect("decodes"))), [[A; 3]]);
}

#[test]
fn kind1_counts_towards_bounds() {
    let entry = body(&encode_list(&[burst_entry()]));
    let rec = kind1(rom_beep_digest(), 1, [1, 0, 0]);
    // 8 entries, 7 of them kind-1: accepted; 9: refused, as a list of 9 entries is
    let mut parts = vec![entry.clone()];
    parts.extend(std::iter::repeat_n(rec.clone(), 7));
    assert!(SndEngines::with_learned(&list_of(&parts)).is_ok(), "8 entries");
    parts.push(rec.clone());
    assert!(SndEngines::with_learned(&list_of(&parts)).is_err(), "9 entries");
    assert!(SndEngines::with_learned(&list_of(&vec![rec.clone(); 9])).is_err(), "9 kind-1 records");
    let mut m = SndMixer::new(RATE.0, RATE.1);
    assert!(m.set_learned(&list_of(&vec![rec.clone(); 9])).is_err(), "the mixer: 9 kind-1 records");

    // a kind-1 record is size-checked like an entry
    let bad = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut r = rec.clone();
        f(&mut r);
        list_of(&[entry.clone(), r])
    };
    assert!(SndEngines::with_learned(&bad(&|_| {})).is_ok(), "the record as it is");
    assert!(SndEngines::with_learned(&bad(&|r| r.extend([0; 4]))).is_ok(), "bytes after the last entry are ignored");
    assert!(SndEngines::with_learned(&bad(&|r| {
        r[0] = 20;
        r.extend([0; 4]);
    }))
    .is_ok(), "size 20");
    assert!(SndEngines::with_learned(&bad(&|r| r[0] = 12)).is_err(), "size 12");
    assert!(SndEngines::with_learned(&bad(&|r| r[0] = 18)).is_err(), "size 18 (not a multiple of 4)");
    assert!(SndEngines::with_learned(&bad(&|r| r[0] = 20)).is_err(), "size 20 past the list");
    assert!(SndEngines::with_learned(&bad(&|r| r.truncate(12))).is_err(), "the record cut short");
    assert!(SndEngines::with_learned(&bad(&|r| r[3] = 0)).is_err(), "0 channels");
    assert!(SndEngines::with_learned(&bad(&|r| r[3] = 4)).is_err(), "4 channels");
    assert!(SndEngines::with_learned(&bad(&|r| r[2] = 2)).is_err(), "kind 2");
}

/// Colony 8's built-in entry's digest.
fn colony8_digest() -> u64 {
    SndEngines::new().entries().into_iter().find(|e| e.name == "Colony 8").expect("Colony 8").digest
}

#[test]
fn partial_stored_roles_fall_back() {
    // Colony 8 (the built-in entry, three channels) with a kind-1 record giving channel 1 the pad
    // and channels 0 and 2 none
    let frames = 40 * 50;
    let list = list_of(&[kind1(colony8_digest(), 3, [0, 3, 0])]);

    // B: its claimed frames carry the stored roles, its unclaimed ones none
    let d = decisions(colony8(), SndEngines::with_learned(&list).expect("decodes"), frames);
    println!("{} of {frames} frames claimed", claimed(&d));
    assert_eq!(claimed_roles(&d), [[A, P, A]]);

    // the mixer: the synth's voice 1 plays the pad whenever it sounds; voices 0 and 2 take the
    // live rule's roles among the voices without one (voice 0 alone: the lead; from channel 2's
    // entry at about 11.5 s, 2 semitones below voice 0 and ranked by its own pitch, voice 2 the
    // bass and voice 0 the lead; from about 35 s, voice 2 the margin above voice 0, voice 0 the
    // bass and voice 2 the lead), and the live rule tracks every voice, voice 1 included
    let mut m = colony8();
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    mixer.set_learned(&list).expect("decodes");
    let mut seen: Vec<[u8; 3]> = Vec::new();
    for _ in 0..frames {
        m.run_frame();
        beeper.set_level(m.edges_start_level());
        let samples = beeper.render(m.edges());
        mixer.frame(SND_MODE_FM_B, Some(&m), m.edges(), m.edges_start_level(), &samples);
        let v: [u8; 3] = std::array::from_fn(|k| mixer.voice_role(k));
        if mixer.fm_on() && seen.last() != Some(&v) {
            seen.push(v);
        }
    }
    println!("the synth's roles: {seen:?}");
    // (a voice that has not played yet shows the synth's start-up role, the lead)
    let first = seen.iter().position(|v| v[1] == P).expect("voice 1 plays the pad");
    assert!(seen[first..].iter().all(|v| v[1] == P), "voice 1 the pad from its first note on");
    assert!(seen.iter().any(|v| v[0] == L && v[1] == P), "voice 0 alone without a role: the lead");
    assert!(seen.contains(&[L, P, B]), "voice 2 enters as the bass, by its own pitch");
    assert_eq!(seen.last(), Some(&[B, P, L]), "at the end");
    assert!(mixer.roles().1.iter().all(|&a| a != -(1 << 30)), "every voice tracked (SND_ROLES_NONE: never heard)");
}
