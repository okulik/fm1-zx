// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! AY songs assembled here (no collection files): ZXAYEMUL files with a small Z80 player,
//! parsed, played in the Spectrum machine core and turned into notes.

use chipmusic::ay::{capture, memory, AY_HZ};
use chipmusic::notes::{DRUM, FRAME_T};
use chipmusic::{chip_notes, info, AyFile, ChipNotes, Note};
use std::collections::HashMap;

/// A small Z80 assembler: bytes, labels, and absolute and relative fixups.
struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: HashMap<String, u16>,
    fixups: Vec<(usize, String, bool)>,
}

impl Asm {
    fn new(org: u16) -> Asm {
        Asm { org, code: Vec::new(), labels: HashMap::new(), fixups: Vec::new() }
    }
    fn b(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }
    fn label(&mut self, l: &str) -> &mut Self {
        self.labels.insert(l.to_string(), self.org + self.code.len() as u16);
        self
    }
    /// `op` (one or more bytes) with the label's address as its 16-bit operand.
    fn abs(&mut self, op: &[u8], l: &str) -> &mut Self {
        self.code.extend_from_slice(op);
        self.fixups.push((self.code.len(), l.to_string(), true));
        self.b(&[0, 0])
    }
    fn rel(&mut self, op: u8, l: &str) -> &mut Self {
        self.code.push(op);
        self.fixups.push((self.code.len(), l.to_string(), false));
        self.b(&[0])
    }
    fn addr(&self, l: &str) -> u16 {
        self.labels[l]
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, l, abs) in std::mem::take(&mut self.fixups) {
            let a = *self.labels.get(&l).unwrap_or_else(|| panic!("label {l}"));
            if abs {
                self.code[at..at + 2].copy_from_slice(&a.to_le_bytes());
            } else {
                let off = a.wrapping_sub(self.org + at as u16 + 1) as i16;
                assert!((-128..128).contains(&off), "jump to {l} too far");
                self.code[at] = off as u8;
            }
        }
        self.code
    }
}

/// The AY register writes of each frame: (register, value), in order.
type Frames = Vec<Vec<(u8, u8)>>;

const ORG: u16 = 0x8000;

/// The write-list player at $8000: `play` performs one frame's writes (`reg, value` pairs up to
/// $FF; $FE `lo hi` jumps on), `init` points it at the frames; with `self_playing` the init
/// plays them itself (`EI`, then `HALT` and `play` forever) instead of returning.
fn player(frames: &Frames, loop_to: Option<usize>, self_playing: bool) -> Asm {
    let mut a = Asm::new(ORG);
    a.label("play").abs(&[0x2A], "ptr");
    a.label("next").b(&[0x7E, 0x23, 0xFE, 0xFF]).rel(0x28, "done").b(&[0xFE, 0xFE]).rel(0x28, "wrap");
    a.b(&[0x01, 0xFD, 0xFF, 0xED, 0x79, 0x7E, 0x23, 0x06, 0xBF, 0xED, 0x79]).rel(0x18, "next");
    a.label("wrap").b(&[0x5E, 0x23, 0x56, 0xEB]).rel(0x18, "next");
    a.label("done").abs(&[0x22], "ptr").b(&[0xC9]);
    a.label("init").abs(&[0x21], "f0").abs(&[0x22], "ptr");
    if self_playing {
        a.b(&[0xFB]).label("idle").b(&[0x76]).abs(&[0xCD], "play").rel(0x18, "idle");
    } else {
        a.b(&[0xC9]);
    }
    a.label("ptr").b(&[0, 0]);
    for (k, f) in frames.iter().enumerate() {
        a.label(&format!("f{k}"));
        for &(r, v) in f {
            a.b(&[r, v]);
        }
        a.b(&[0xFF]);
    }
    let to = match loop_to {
        Some(k) => format!("f{k}"),
        None => {
            a.label("end").b(&[0xFF]);
            "end".to_string()
        }
    };
    a.b(&[0xFE]).abs(&[], &to);
    a
}

/// A song of an `.ay` file being built.
#[derive(Clone)]
struct Song {
    name: &'static str,
    stack: u16,
    /// 0: the first block's address.
    init: u16,
    interrupt: u16,
    hi: u8,
    lo: u8,
    blocks: Vec<(u16, Vec<u8>)>,
}

/// A song playing `frames` (looping back to frame `loop_to`) with the write-list player.
fn song(frames: &Frames, loop_to: Option<usize>) -> Song {
    let a = player(frames, loop_to, false);
    let (init, play) = (a.addr("init"), a.addr("play"));
    Song { name: "A Made-up Tune", stack: 0xFF00, init, interrupt: play, hi: 0, lo: 0, blocks: vec![(ORG, a.finish())] }
}

struct File {
    author: &'static str,
    misc: &'static str,
    first: u8,
    special: u16,
    kind: &'static [u8; 4],
    songs: Vec<Song>,
}

impl File {
    fn of(songs: Vec<Song>) -> File {
        File { author: "Nobody", misc: "1987 Nowhere", first: 0, special: 0, kind: b"EMUL", songs }
    }
}

/// Builds a ZXAYEMUL file: the header, the strings, the songs' table, each song's name, data,
/// points, blocks table and blocks.
fn ay_file(f: &File) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(b"ZXAY");
    d.extend_from_slice(f.kind);
    d.extend_from_slice(&[3, 3]);
    d.extend_from_slice(&f.special.to_be_bytes());
    d.extend_from_slice(&[0; 4]);
    d.extend_from_slice(&[(f.songs.len() - 1) as u8, f.first, 0, 0]);
    let patch = |d: &mut Vec<u8>, at: usize, to: usize| {
        let rel = (to as i64 - at as i64) as i16 as u16;
        d[at..at + 2].copy_from_slice(&rel.to_be_bytes());
    };
    let string = |d: &mut Vec<u8>, at: usize, s: &str| {
        let to = d.len();
        d.extend_from_slice(s.as_bytes());
        d.push(0);
        patch(d, at, to);
    };
    string(&mut d, 12, f.author);
    string(&mut d, 14, f.misc);
    let table = d.len();
    patch(&mut d, 18, table);
    d.resize(table + 4 * f.songs.len(), 0);
    for (k, s) in f.songs.iter().enumerate() {
        let at = table + 4 * k;
        string(&mut d, at, s.name);
        let data = d.len();
        patch(&mut d, at + 2, data);
        d.extend_from_slice(&[0, 1, 2, 3, 0x0B, 0xB8, 0, 50, s.hi, s.lo, 0, 0, 0, 0]);
        let points = d.len();
        patch(&mut d, data + 10, points);
        for w in [s.stack, s.init, s.interrupt] {
            d.extend_from_slice(&w.to_be_bytes());
        }
        let blocks = d.len();
        patch(&mut d, data + 12, blocks);
        d.resize(blocks + 6 * s.blocks.len() + 2, 0);
        for (j, (addr, bytes)) in s.blocks.iter().enumerate() {
            let e = blocks + 6 * j;
            d[e..e + 2].copy_from_slice(&addr.to_be_bytes());
            d[e + 2..e + 4].copy_from_slice(&(bytes.len() as u16).to_be_bytes());
            let to = d.len();
            d.extend_from_slice(bytes);
            patch(&mut d, e + 4, to);
        }
    }
    d
}

fn frames(n: usize) -> Frames {
    vec![Vec::new(); n]
}

fn tone(f: &mut Vec<(u8, u8)>, ch: u8, period: u16) {
    f.push((2 * ch, period as u8));
    f.push((2 * ch + 1, (period >> 8) as u8));
}

/// The pitch of a tone period, and of an envelope period as a buzzer (`div` 256 or 512).
fn tone_hz(period: u16) -> f64 {
    f64::from(AY_HZ) / (16.0 * f64::from(period))
}
fn buzz_hz(period: u16, div: f64) -> f64 {
    f64::from(AY_HZ) / (div * f64::from(period))
}
fn period_t(hz: f64) -> u32 {
    (3_500_000.0 / hz).round() as u32
}
/// The start of frame `k` (T).
fn t(k: u64) -> u64 {
    k * FRAME_T
}

fn play(file: &[u8], song: u16, seconds: f64) -> ChipNotes {
    chip_notes(file, Some(song), seconds).unwrap_or_else(|e| panic!("cannot be played: {e}"))
}

fn voice(n: &ChipNotes, v: u8) -> Vec<Note> {
    n.notes.notes.iter().filter(|x| x.voice == v).copied().collect()
}

#[test]
fn parse_header_songs_and_blocks() {
    let mut s2 = song(&frames(1), None);
    s2.name = "";
    s2.init = 0;
    s2.blocks.push((0xFFF0, vec![0xAA; 0x20]));
    let file = ay_file(&File { first: 1, ..File::of(vec![song(&frames(1), None), s2]) });
    let ay = AyFile::parse(&file).unwrap();
    assert_eq!((ay.count(), ay.start_song(), ay.author.as_str(), ay.misc.as_str()), (2, 2, "Nobody", "1987 Nowhere"));
    assert_eq!((ay.title(1), ay.title(2)), ("A Made-up Tune".to_string(), "1987 Nowhere".to_string()));
    let s = &ay.songs[0];
    assert_eq!((s.length, s.fade, s.stack, s.interrupt), (3000, 50, 0xFF00, ORG));
    assert_eq!(s.blocks.len(), 1);
    assert_eq!(s.blocks[0].addr, ORG);
    // a block running past $FFFF is clipped there
    assert_eq!(ay.songs[1].blocks[1].data.len(), 0x10);
    let text = info(&file).unwrap();
    assert!(text.contains("2 songs, start song 2") && text.contains("song 1: A Made-up Tune"), "{text}");
}

#[test]
fn malformed_files_are_errors_not_panics() {
    let good = ay_file(&File::of(vec![song(&vec![vec![(8, 15)]; 4], Some(0))]));
    for n in 0..good.len() {
        // every truncation either parses (the tail held only block bytes) or says why not
        if let Ok(f) = AyFile::parse(&good[..n]) {
            let _ = memory(&f, 1);
            if n % 16 == 0 {
                let _ = capture(&f, 1, 0.1);
            }
        }
    }
    // garbage after the magic, and the good file with bytes flipped
    let mut x = 0x2545_F491_u32;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut played = 0;
    for _ in 0..3000 {
        let mut d = good.clone();
        for _ in 0..1 + rnd() % 6 {
            let at = rnd() as usize % d.len();
            d[at] = rnd() as u8;
        }
        if let Ok(f) = AyFile::parse(&d) {
            for s in 1..=f.count() {
                if f.refusal(s).is_none() {
                    let _ = memory(&f, s);
                }
                // (a garbage song may idle for the 30 s it is given to start: only some run)
                if played < 12 {
                    played += 1;
                    let _ = capture(&f, s, 0.2);
                }
            }
        }
        let n = 8 + rnd() as usize % 64;
        let mut g: Vec<u8> = b"ZXAYEMUL".to_vec();
        g.extend((0..n).map(|_| rnd() as u8));
        let _ = AyFile::parse(&g);
    }
    let err = |d: &[u8]| AyFile::parse(d).unwrap_err();
    assert!(err(b"ZXAYEMUL").contains("shorter than a header"));
    assert!(err(&ay_file(&File { kind: b"AMAD", ..File::of(vec![song(&frames(1), None)]) })).contains("type AMAD"));
    let mut d = good.clone();
    d[18..20].copy_from_slice(&0x7000u16.to_be_bytes());
    assert!(err(&d).contains("past the file's end"), "{}", err(&d));
    let mut d = good.clone();
    d[17] = 3;
    assert!(err(&d).contains("first song is 4 of 1"));
}

#[test]
fn memory_and_player_as_the_format_lays_out() {
    let s = song(&frames(1), None);
    let ay = AyFile::parse(&ay_file(&File::of(vec![s.clone()]))).unwrap();
    let m = memory(&ay, 1);
    let [il, ih] = s.init.to_le_bytes();
    let [jl, jh] = s.interrupt.to_le_bytes();
    assert_eq!(m[..13], [0xF3, 0xCD, il, ih, 0xED, 0x56, 0xFB, 0x76, 0xCD, jl, jh, 0x18, 0xF7]);
    assert_eq!((m[13], m[0x37], m[0x38], m[0x39], m[0xFF]), (0xC9, 0xC9, 0xFB, 0xC9, 0xC9));
    assert!(m[0x100..0x4000].iter().all(|&b| b == 0xFF));
    assert!(m[0x4000..usize::from(ORG)].iter().all(|&b| b == 0));
    assert_eq!(m[usize::from(ORG)..usize::from(ORG) + s.blocks[0].1.len()], s.blocks[0].1[..]);
    // init 0: the first block's address; no interrupt: the init plays by itself, IM 2
    let ay = AyFile::parse(&ay_file(&File::of(vec![Song { init: 0, interrupt: 0, ..s }]))).unwrap();
    let [bl, bh] = ORG.to_le_bytes();
    assert_eq!(memory(&ay, 1)[..10], [0xF3, 0xCD, bl, bh, 0xED, 0x5E, 0xFB, 0x76, 0x18, 0xFA]);
}

#[test]
fn registers_captured_each_frame() {
    let mut f = frames(6);
    tone(&mut f[0], 0, 0x1FC);
    f[0].extend([(7, 0xFE), (8, 0x0F), (6, 0x3F)]);
    f[2].push((8, 0x0A));
    f[3].extend([(13, 0x08), (13, 0x08), (14, 0x55), (0x1D, 0x44)]);
    f[4].extend([(1, 0xFF)]);
    let ay = AyFile::parse(&ay_file(&File::of(vec![song(&f, None)]))).unwrap();
    let cap = capture(&ay, 1, 0.2).unwrap();
    // 0.2 s: 11 frames of 1/50.08 s
    assert_eq!(cap.rows.len(), 11);
    assert_eq!(cap.end, t(11));
    let r = &cap.rows;
    assert_eq!(r[1].t, t(1));
    // values kept to their registers' bits; the I/O port register ignored; register 0x1D
    // selects 13 (the value & 15)
    assert_eq!(r[0].regs[..9], [0xFC, 0x01, 0, 0, 0, 0, 0x1F, 0xFE, 0x0F]);
    assert_eq!(r[0].rise, 0);
    assert_eq!(r[2].regs[8], 0x0A);
    assert!(r[3].env_restart && !r[2].env_restart && !r[4].env_restart);
    assert_eq!(r[3].regs[13], 0x04);
    assert_eq!(r[4].regs[1], 0x0F);
}

#[test]
fn a_song_that_stops_writing_ends() {
    let mut f = frames(10);
    f[0].extend([(7, 0b0011_1110), (8, 0x0F)]);
    tone(&mut f[0], 0, 0x100);
    f[9].push((8, 0x0E));
    let ay = AyFile::parse(&ay_file(&File::of(vec![song(&f, None)]))).unwrap();
    // the last write at frame 9: the rows end 50 frames (a second) after it
    let cap = capture(&ay, 1, 60.0).unwrap();
    assert_eq!(cap.rows.len(), 60);
    let n = play(&ay_file(&File::of(vec![song(&f, None)])), 1, 60.0);
    assert_eq!((n.notes.intro_t, n.notes.loop_t), (t(60), 0));
}

#[test]
fn tones_buzzer_drum_and_loop() {
    // A: a lead stepping through four tone periods, 15 frames each; B: a buzzer bass (tone and
    // noise off, sawtooth envelope 8) on two envelope periods; C: a noise drum every 30 frames;
    // the 60 frames (1.2 s: a loop is at least a second) loop
    let leads = [0x0FE, 0x0E2, 0x0C9, 0x0BE];
    let buzz = [63u16, 84];
    let mut f = frames(60);
    f[0].extend([(7, 0b0001_1110), (8, 0x0D), (9, 0x10), (6, 0x01)]);
    for (k, &p) in leads.iter().enumerate() {
        tone(&mut f[15 * k], 0, p);
    }
    for (k, &e) in buzz.iter().enumerate() {
        f[30 * k].extend([(11, e as u8), (12, (e >> 8) as u8), (13, 0x08)]);
        f[30 * k].push((10, 0x0F));
        f[30 * k + 2].push((10, 0x00));
    }
    let n = play(&ay_file(&File::of(vec![song(&f, Some(0))])), 1, 4.0);
    assert_eq!(n.clock_hz, AY_HZ);
    assert_eq!(n.notes.chip_voice, [Some(0), Some(1), None]);
    let lead = voice(&n, 0);
    let want: Vec<(u64, u64, u32, u8)> =
        leads.iter().enumerate().map(|(k, &p)| (t(15 * k as u64), t(15 * k as u64 + 15), period_t(tone_hz(p)), 181)).collect();
    assert_eq!(lead.iter().map(|x| (x.start_t, x.end_t, x.period_t, x.level)).collect::<Vec<_>>(), want);
    let bass = voice(&n, 1);
    assert_eq!(
        bass.iter().map(|x| (x.start_t, x.end_t, x.period_t, x.level)).collect::<Vec<_>>(),
        vec![(0, t(30), period_t(buzz_hz(63, 256.0)), 255), (t(30), t(60), period_t(buzz_hz(84, 256.0)), 255)]
    );
    let drums = voice(&n, DRUM);
    assert_eq!(drums.iter().map(|x| (x.start_t, x.end_t, x.level)).collect::<Vec<_>>(), vec![(0, t(2), 255), (t(30), t(32), 255)]);
    assert_eq!((n.notes.intro_t, n.notes.loop_t), (0, t(60)));
    let s = n.summary();
    assert!(s.contains("loop of 1.20 s from 0.00 s"), "{s}");
}

#[test]
fn triangle_buzzer_envelope_decay_and_retriggers() {
    let mut f = frames(40);
    // A: a decaying envelope (shape 0) restarted every 10 frames, the same value rewritten:
    // each restart a note; the decay lasts 256 * 400 / 1,773,400 s (= 3.3 frames)
    f[0].extend([(7, 0b0011_1010), (8, 0x10), (11, 0x90), (12, 0x01)]);
    tone(&mut f[0], 0, 0x11D);
    for k in 0..4 {
        f[10 * k].push((13, 0x00));
    }
    // B: from frame 30 (A silent), a triangle buzzer (tone off, shape 14, period 50)
    f[30].extend([(8, 0x00), (11, 50), (12, 0), (13, 0x0E), (9, 0x10)]);
    // C: tone with a software volume: 15, 13, 11, then 14 (a jump of 3: a new note), legato
    // move to another pitch at frame 6, off at 8
    f[0].extend([(10, 0x0F)]);
    tone(&mut f[0], 2, 0x200);
    f[1].push((10, 0x0D));
    f[2].push((10, 0x0B));
    f[3].push((10, 0x0E));
    tone(&mut f[6], 2, 0x1C8);
    f[8].push((10, 0));
    let n = play(&ay_file(&File::of(vec![song(&f, None)])), 1, 1.0);
    let by_chip = |ch: u8| -> Vec<Note> {
        let v = n.notes.chip_voice.iter().position(|&c| c == Some(ch)).unwrap() as u8;
        voice(&n, v)
    };
    let decay = (256.0 * 400.0 * 3_500_000.0 / f64::from(AY_HZ) / 64.0).round() as u64 * 64;
    let a = by_chip(0);
    let starts: Vec<u64> = (0..3).map(|k| t(10 * k)).collect();
    assert_eq!(a.iter().map(|x| x.start_t).collect::<Vec<_>>(), starts);
    for x in &a {
        assert!((x.end_t - x.start_t).abs_diff(decay) <= 64, "{x:?} vs decay {decay}");
        assert_eq!((x.period_t, x.level), (period_t(tone_hz(0x11D)), 255));
    }
    let b = by_chip(1);
    assert_eq!(b.iter().map(|x| (x.start_t, x.period_t)).collect::<Vec<_>>(), vec![(t(30), period_t(buzz_hz(50, 512.0)))]);
    let c = by_chip(2);
    assert_eq!(
        c.iter().map(|x| (x.start_t, x.end_t, x.period_t, x.level)).collect::<Vec<_>>(),
        vec![
            (0, t(3), period_t(tone_hz(0x200)), 255),
            (t(3), t(6), period_t(tone_hz(0x200)), 215),
            (t(6), t(8), period_t(tone_hz(0x1C8)), 215)
        ]
    );
}

#[test]
fn noise_bursts_are_drums() {
    // A: noise and tone alternating frame by frame (a drum's sound), then from frame 10 tone
    // and noise together held: a drum hit, then tonal once the drum's tail (0.16 s) is over;
    // B: a steady note
    let mut f = frames(40);
    f[0].extend([(8, 0x0F), (9, 0x0C), (6, 0x05)]);
    tone(&mut f[0], 0, 0x03B);
    tone(&mut f[0], 1, 0x200);
    for (k, fk) in f.iter_mut().take(4).enumerate() {
        fk.push((7, if k % 2 == 0 { 0b0011_0101 } else { 0b0011_1100 }));
    }
    f[4].push((8, 0));
    f[10].extend([(7, 0b0011_0100), (8, 0x0C)]);
    f[30].push((8, 0));
    f[35].push((9, 0));
    let n = play(&ay_file(&File::of(vec![song(&f, None)])), 1, 1.0);
    let drums = voice(&n, DRUM);
    assert_eq!(drums.iter().map(|x| x.start_t).collect::<Vec<_>>(), vec![0, t(10)]);
    // the first hit lasts its 0.06 s through the alternation
    assert_eq!(drums[0].end_t, (0.06 * 3_500_000.0 / 64.0_f64).round() as u64 * 64);
    let a = n.notes.chip_voice.iter().position(|&c| c == Some(0)).unwrap() as u8;
    assert_eq!(voice(&n, a).iter().map(|x| (x.start_t, x.end_t)).collect::<Vec<_>>(), vec![(t(19), t(30))]);
    let b = n.notes.chip_voice.iter().position(|&c| c == Some(1)).unwrap() as u8;
    assert_eq!(voice(&n, b).iter().map(|x| (x.start_t, x.end_t)).collect::<Vec<_>>(), vec![(0, t(35))]);
}

#[test]
fn self_playing_init_and_levels() {
    // the init plays by itself (no interrupt address): EI, HALT, play
    let mut f = frames(8);
    f[0].extend([(7, 0b0011_1110), (8, 0x01)]);
    tone(&mut f[0], 0, 0x100);
    f[4].push((8, 0x00));
    let a = player(&f, None, true);
    let init = a.addr("init");
    let s = Song { name: "Self", stack: 0xFF00, init, interrupt: 0, hi: 0x12, lo: 0x34, blocks: vec![(ORG, a.finish())] };
    let n = play(&ay_file(&File::of(vec![s])), 1, 0.5);
    let lead = voice(&n, 0);
    assert_eq!(lead.iter().map(|x| (x.start_t, x.end_t, x.level)).collect::<Vec<_>>(), vec![(0, t(4), 32)]);
}

#[test]
fn toml_and_subtunes() {
    let mut f = frames(4);
    f[0].extend([(7, 0b0011_1110), (8, 0x0F)]);
    tone(&mut f[0], 0, 0x100);
    f[2].push((8, 0));
    let mut quiet = f.clone();
    quiet[0][1] = (8, 0x0E);
    let file = ay_file(&File { first: 1, ..File::of(vec![song(&f, None), Song { name: "Second", ..song(&quiet, None) }]) });
    // no subtune: the header's first song (the second)
    let n = chip_notes(&file, None, 0.5).unwrap();
    assert_eq!(n.subtune, 2);
    assert_eq!(voice(&n, 0)[0].level, 215);
    let text = n.toml("dir/tune.ay");
    let v: toml::Table = text.parse().unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(v["format"].as_str(), Some("AY"));
    assert_eq!((v["title"].as_str(), v["author"].as_str(), v["released"].as_str()), (Some("Second"), Some("Nobody"), Some("1987 Nowhere")));
    assert_eq!((v["subtune"].as_integer(), v["clock_hz"].as_integer()), (Some(2), Some(1_773_400)));
    assert!(text.contains("lead AY channel A"), "{text}");
    assert_eq!(play(&file, 1, 0.5).notes.notes[0].level, 255);
    assert!(chip_notes(&file, Some(3), 0.5).unwrap_err().contains("no subtune 3"));
}

#[test]
fn refusals() {
    let s = song(&frames(2), None);
    let err = |songs: Vec<Song>| chip_notes(&ay_file(&File::of(songs)), None, 1.0).unwrap_err();
    // writes nothing
    assert!(err(vec![s.clone()]).contains("wrote nothing to the AY"));
    // a beeper tune: OUTs to port $FE from the interrupt
    let beeper = vec![0x3E, 0x10, 0xD3, 0xFE, 0xAF, 0xD3, 0xFE, 0xC9];
    let e = err(vec![Song { init: 0x9000, interrupt: 0x9000, blocks: vec![(0x9000, beeper)], ..s.clone() }]);
    assert!(e.contains("a beeper tune"), "{e}");
    // an Amstrad CPC song: OUTs to the CPC's PPI ports
    let cpc = vec![0x01, 0x00, 0xF4, 0x3E, 0x07, 0xED, 0x79, 0x06, 0xF6, 0x3E, 0xC0, 0xED, 0x79, 0xC9];
    let e = err(vec![Song { init: 0x9000, interrupt: 0x9000, blocks: vec![(0x9000, cpc)], ..s.clone() }]);
    assert!(e.contains("Amstrad CPC"), "{e}");
    // a SAM Coupe song: OUTs to the SAA 1099's ports
    let sam = vec![0x01, 0xFF, 0x01, 0x3E, 0x1C, 0xED, 0x79, 0xC9];
    let e = err(vec![Song { init: 0x9000, interrupt: 0x9000, blocks: vec![(0x9000, sam)], ..s.clone() }]);
    assert!(e.contains("SAM Coupe"), "{e}");
    // DI; HALT
    let e = err(vec![Song { init: 0x9000, interrupt: 0, blocks: vec![(0x9000, vec![0xF3, 0x76])], ..s.clone() }]);
    assert!(e.contains("halted with interrupts off"), "{e}");
    // a special player
    let e = chip_notes(&ay_file(&File { special: 2, ..File::of(vec![s.clone()]) }), None, 1.0).unwrap_err();
    assert!(e.contains("special player"), "{e}");
    // no blocks
    assert!(err(vec![Song { blocks: vec![], ..s }]).contains("no code or data"));
    assert!(chip_notes(b"ZXAYEMUL", None, 1.0).unwrap_err().contains("shorter than a header"));
}
