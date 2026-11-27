// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine reader (`chipmusic::engine`) on a fixture per engine, each a small program run in
//! the Spectrum machine core (tests/engines/README.md says where each comes from):
//!
//! - Tritone: the engine's own code (public domain, from its author's release) with an invented
//!   song of ours in its documented format;
//! - ntropic and Stocker: programs of ours that reproduce each engine's row read, sound loop and
//!   drum code as the reader recognises them (the signatures), with invented songs.
//!
//! Each test checks that the engine is found (also moved to another address), that every note
//! and drum hit of the song is read with its channel, start, end, pitch and level, and that the
//! pass time is the sound loop's.

use chipmusic::engine::{identify, identify_all, locate_in_run, memory, read_run, EngineNote, Reading};
use std::collections::BTreeMap;
use zx_sys::Machine;

/// A hand-assembler with labels: bytes, and 16-bit or relative references to labels fixed up at
/// the end.
struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: BTreeMap<&'static str, u16>,
    abs: Vec<(usize, &'static str)>,
    rel: Vec<(usize, &'static str)>,
}

impl Asm {
    fn new(org: u16) -> Asm {
        Asm { org, code: Vec::new(), labels: BTreeMap::new(), abs: Vec::new(), rel: Vec::new() }
    }
    fn here(&self) -> u16 {
        self.org + self.code.len() as u16
    }
    fn label(&mut self, l: &'static str) -> &mut Self {
        self.labels.insert(l, self.here());
        self
    }
    fn b(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }
    /// An opcode, then a label's address.
    fn a(&mut self, op: &[u8], l: &'static str) -> &mut Self {
        self.b(op);
        self.abs.push((self.code.len(), l));
        self.b(&[0, 0])
    }
    /// An opcode, then a jump to a label relative to the next instruction.
    fn r(&mut self, op: u8, l: &'static str) -> &mut Self {
        self.b(&[op]);
        self.rel.push((self.code.len(), l));
        self.b(&[0])
    }
    fn done(mut self) -> (u16, Vec<u8>, BTreeMap<&'static str, u16>) {
        for &(at, l) in &self.abs {
            let v = self.labels[l];
            self.code[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }
        for &(at, l) in &self.rel {
            let d = i32::from(self.labels[l]) - i32::from(self.org + at as u16 + 1);
            assert!((-128..128).contains(&d), "{l} out of reach");
            self.code[at] = d as i8 as u8;
        }
        (self.org, self.code, self.labels)
    }
}

/// A machine with `blocks` in memory, started at `pc` with interrupts off.
fn machine(blocks: &[(u16, &[u8])], pc: u16) -> Machine {
    let mut m = Machine::new();
    for &(at, bytes) in blocks {
        for (k, &b) in bytes.iter().enumerate() {
            m.poke(at + k as u16, b);
        }
    }
    let mut r = m.regs();
    r.pc = pc;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m
}

/// Reads the program over `frames` frames with no key pressed.
fn read(load: &dyn Fn() -> Machine, frames: usize) -> Reading {
    let none = |_| ([0u8; 8], 0u8);
    let (e, sites, _) = locate_in_run(load, &none, frames, &|_| true).ok().expect("the engine is found");
    read_run(load, &none, frames, e, &sites, 0, &mut |_, _| {})
}

/// The notes as (channel, value, level) in order, and the starts of each row they span.
fn shape(r: &Reading) -> Vec<(u8, u32, u8)> {
    r.notes.iter().map(|n| (n.channel, n.value, n.level)).collect()
}

/// The rows' starts.
fn row_t(r: &Reading) -> Vec<u64> {
    r.rows.iter().map(|x| x.t).collect()
}

/// Whether a note starts at row `a` and ends at row `b` (None: the last row's end).
fn spans(n: &EngineNote, rows: &[u64], a: usize, b: Option<usize>) -> bool {
    n.start_t == rows[a] && b.is_none_or(|b| n.end_t == rows[b])
}

// ---------------------------------------------------------------------------------------------
// ntropic: a program of ours with ntropic's row read, sound loop and click drum

/// Rows of 4 bytes: drum flag in bit 0, channel 1's counter, channel 2's, the noise (bit 7: none).
const NTROPIC_ROWS: [[u8; 4]; 5] = [[0, 60, 120, 0x80], [1, 60, 120, 0x80], [0, 45, 90, 2], [0, 0, 90, 0x80], [0, 80, 0, 0x80]];

fn ntropic_program(org: u16) -> (u16, Vec<u8>) {
    let mut a = Asm::new(org);
    a.b(&[0xF3]) // DI
        .a(&[0x21], "stop") // LD HL,stop: where RET Z at the song's end goes
        .b(&[0xE5]) // PUSH HL
        .a(&[0x21], "rows") // LD HL,rows
        .b(&[0xE5]) // PUSH HL
        .label("row")
        .b(&[0xFD, 0x26, 0x10, 0xE1]) // LD IYH,10h; POP HL
        .b(&[0x7E, 0x3C, 0xC8]) // LD A,(HL); INC A; RET Z: the row read
        .b(&[0x7E, 0x1F]) // LD A,(HL); RRA
        .a(&[0xDC], "drum") // CALL C,drum
        .b(&[0x23, 0x56, 0x23, 0x23, 0x23, 0xE5]) // INC HL; LD D,(HL); INC HL x3; PUSH HL
        .b(&[0x6A, 0x01, 0x00, 0x10, 0xAF, 0x08]) // LD L,D; LD BC,1000h; XOR A; EX AF,AF'
        .label("snd")
        .b(&[0x08, 0xD3, 0xFE, 0x15]) // EX AF,AF'; OUT (FEh),A; DEC D
        .a(&[0xC2], "cont") // JP NZ,cont
        .b(&[0x55, 0xEE, 0x10]) // LD D,L; XOR 10h: the level flips every `counter` passes
        .label("cont")
        .b(&[0x08, 0x0B, 0x78, 0xB1]) // EX AF,AF'; DEC BC; LD A,B; OR C
        .a(&[0xC2], "snd") // JP NZ,snd
        .a(&[0xC3], "row") // JP row
        .label("drum")
        .b(&[0xE5, 0xC5, 0x21, 0x00, 0x10, 0x01, 0x00, 0x01]) // PUSH HL; PUSH BC; LD HL,1000h; LD BC,0100h
        .label("dl")
        .b(&[0x7E, 0xE6, 0xF8, 0xD3, 0xFE, 0x2B, 0x0B, 0x78, 0xB1]) // LD A,(HL); AND F8h; OUT (FEh),A; DEC HL; DEC BC; LD A,B; OR C
        .r(0x20, "dl") // JR NZ,dl
        .b(&[0xC1, 0xE1, 0xC9]) // POP BC; POP HL; RET
        .label("stop")
        .r(0x18, "stop") // JR stop
        .label("rows");
    for r in NTROPIC_ROWS {
        a.b(&r);
    }
    a.b(&[0xFF]);
    let (org, code, _) = a.done();
    (org, code)
}

#[test]
fn ntropic_is_read_from_its_rows() {
    for org in [0x8000u16, 0xB123] {
        let (at, code) = ntropic_program(org);
        let load = || machine(&[(at, &code)], at);
        let r = read(&load, 60);
        assert_eq!((r.engine, r.channels, r.sites.anchor, r.sites.row_pc), ("ntropic", 2, org + 9, org + 13), "found at {org:04X}h");
        assert_eq!(r.rows.len(), NTROPIC_ROWS.len(), "a row each, none at the end marker");
        let rows = row_t(&r);
        // a note while a counter stays the same; 0 is silence
        assert_eq!(shape(&r), [(0, 60, 255), (1, 120, 255), (0, 45, 255), (1, 90, 255), (0, 80, 255)]);
        let n = &r.notes;
        assert!(spans(&n[0], &rows, 0, Some(2)) && spans(&n[1], &rows, 0, Some(2)) && spans(&n[2], &rows, 2, Some(3)) && spans(&n[3], &rows, 2, Some(4)));
        assert!(spans(&n[4], &rows, 4, None) && n[4].end_t - rows[4] > (rows[4] - rows[3]) * 9 / 10, "the last row about as long as the others");
        // the pass: 57 T, 11 more where the level flips (the average over a half period counts
        // it), a little more where the OUTs meet the screen's contention
        let flip = |v: f64| 57.0 + 11.0 / v;
        assert!((flip(60.0)..flip(60.0) + 4.0).contains(&r.pass_t), "pass {}", r.pass_t);
        for x in n {
            assert!((x.period_t - 2.0 * f64::from(x.value) * r.pass_t).abs() < 1e-6, "{x:?}");
        }
        // the click drum at the drum row, the noise at the noise row (2 x 256 + 1 passes, within its row)
        let d: Vec<(u64, &str)> = r.drums.iter().map(|h| (h.t, h.kind)).collect();
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[0].1 == "kick" && d[0].0 > rows[1] && d[0].0 < rows[1] + 200, "{d:?}");
        assert_eq!((d[1], r.drums[1].end_t), ((rows[2], "noise"), rows[2] + (513.0 * r.pass_t) as u64));
    }
}

// ---------------------------------------------------------------------------------------------
// Tritone: the engine's own code (tests/engines/README.md) with an invented song

/// The engine as its release assembles it at C009h.
fn tritone_code() -> Vec<u8> {
    include_str!("engines/tritone-v2-c009.hex").split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
}

/// Our song in the engine's format: the order list (a pattern, 0, the loop to the list), then
/// the pattern: its tempo (passes a row) and rows of a byte per channel (0 a cut, 1 no change,
/// 80h-FFh a note: the duty's 4 bits and the step's 12 over two bytes), a drum's number before a
/// row (2-13 a tone burst, 14-25 noise), FFh its end.
fn tritone_song(at: u16) -> Vec<u8> {
    let pat = at + 6;
    let mut s = Vec::new();
    s.extend_from_slice(&pat.to_le_bytes());
    s.extend_from_slice(&[0, 0]);
    s.extend_from_slice(&at.to_le_bytes());
    s.extend_from_slice(&3000u16.to_le_bytes());
    s.extend_from_slice(&[0x81, 0x23, 0x82, 0x46, 0x00]); // row 0: steps 123h and 246h, channel 2 cut
    s.extend_from_slice(&[0x01, 0x01, 0xC3, 0x0A]); // row 1: channel 2 30Ah, duty C0h
    s.extend_from_slice(&[0x02, 0x81, 0x80, 0x01, 0x01]); // row 2: a tone burst; channel 0 180h
    s.extend_from_slice(&[0x00, 0xA2, 0x00, 0x01]); // row 3: channel 0 cut, channel 1 200h, duty A0h
    s.extend_from_slice(&[0x10, 0x01, 0x01, 0x01]); // row 4: a noise burst
    s.push(0xFF);
    s
}

#[test]
fn tritone_is_read_from_its_own_code() {
    let code = tritone_code();
    assert_eq!(code.len(), 446);
    let song = tritone_song(0xE000);
    // the release's start: LD HL,song; CALL C009h; JP C000h
    let start = [0x21, 0x00, 0xE0, 0xCD, 0x09, 0xC0, 0xC3, 0x00, 0xC0];
    let load = || machine(&[(0xC000, &start), (0xC009, &code), (0xE000, &song)], 0xC000);
    let r = read(&load, 100);
    assert_eq!((r.engine, r.channels), ("Tritone", 3));
    let kinds: Vec<&str> = r.sites.drums.iter().map(|d| d.1).collect();
    assert_eq!(kinds, ["tone", "noise", "noise"]);
    let rows = row_t(&r);
    assert!(rows.len() >= 10, "the song loops: {} rows", rows.len());
    assert_eq!(r.rows[0].passes, 0xB8 + 10 * 256, "the tempo, 3000 (0BB8h): E passes, then 256 for each of D - 1");
    assert!(r.rows[2].passes < 3000, "a drum row is shorter by the drum's time");
    // the first pass of the song: a note while a step stays the same; a cut or duty 0 is silence
    let first: Vec<(u8, u32, u8, u8)> = r.notes.iter().filter(|n| n.start_t < rows[5]).map(|n| (n.channel, n.value, n.duty, n.level)).collect();
    assert_eq!(first, [(0, 0x123, 0x80, 255), (1, 0x246, 0x80, 255), (2, 0x30A, 0xC0, 213), (0, 0x180, 0x80, 255), (1, 0x200, 0xA0, 244)]);
    let n: Vec<&EngineNote> = r.notes.iter().filter(|n| n.start_t < rows[5]).collect();
    assert!(spans(n[0], &rows, 0, Some(2)) && spans(n[1], &rows, 0, Some(3)) && spans(n[3], &rows, 2, Some(3)));
    assert!(spans(n[2], &rows, 1, Some(5)), "channel 2 sounds until the next pass's row 0 cuts it");
    assert!(spans(n[4], &rows, 3, Some(5)), "until row 0 sets 246h again");
    // the pass: 153 T as its source counts it, a little more where the OUTs meet the screen's contention
    assert!((153.0..156.0).contains(&r.pass_t), "pass {}", r.pass_t);
    for x in &n {
        assert!((x.period_t - 65536.0 * r.pass_t / f64::from(x.value)).abs() < 1e-6);
    }
    let d: Vec<(usize, &str)> = r.drums.iter().take(2).map(|h| (rows.iter().rposition(|&t| t <= h.t).unwrap(), h.kind)).collect();
    assert_eq!(d, [(1, "tone"), (3, "noise")], "each burst plays before its row's notes, after the row before");
}

// ---------------------------------------------------------------------------------------------
// Stocker: a program of ours with Stocker's tick, sound loop and drums

/// Ticks of 7 bytes: channel 1's step, channel 2's (words), their volumes, the drum (1 kick, 2
/// hat); FFFFh ends.
const STOCKER_TICKS: [(u16, u16, u8, u8, u8); 6] = [(2000, 3000, 16, 12, 1), (2000, 3000, 14, 12, 0), (2000, 3000, 16, 12, 0), (2500, 3000, 16, 12, 2), (2500, 0, 16, 0, 0), (0, 4000, 0, 10, 0)];

fn stocker_program(org: u16) -> (u16, Vec<u8>, u16) {
    let mut a = Asm::new(org);
    a.b(&[0xF3, 0xDD, 0x21, 0x00, 0x00, 0xD9, 0x21, 0x00, 0x00, 0xD9]) // DI; LD IX,0; EXX; LD HL,0; EXX
        .a(&[0xFD, 0x21], "ticks") // LD IY,ticks
        .label("tick")
        .b(&[0xFD, 0x7E, 0x00, 0xFD, 0xA6, 0x01, 0x3C]) // LD A,(IY+0); AND (IY+1); INC A
        .a(&[0xCA], "stop") // JP Z,stop
        .b(&[0xFD, 0x7E, 0x06, 0x3D]) // LD A,(IY+6); DEC A
        .a(&[0xCC], "kick") // CALL Z,kick
        .b(&[0xFD, 0x7E, 0x06, 0xFE, 0x02]) // LD A,(IY+6); CP 2
        .a(&[0xCC], "hat") // CALL Z,hat
        .b(&[0xFD, 0x5E, 0x00, 0xFD, 0x56, 0x01, 0xD9, 0xFD, 0x5E, 0x02, 0xFD, 0x56, 0x03, 0xD9]) // DE, DE' the steps
        .b(&[0xFD, 0x7E, 0x04])
        .a(&[0x32], "vol1") // LD (vol1),A
        .b(&[0xFD, 0x7E, 0x05])
        .a(&[0x32], "vol2") // LD (vol2),A
        .b(&[0x01, 0x07, 0x00, 0xFD, 0x09, 0x01, 0xFE, 0x00]) // LD BC,7; ADD IY,BC; LD BC,00FEh
        .b(&[0x21, 0x10, 0x80]) // LD HL,8010h: the tick (H its passes)
        .label("gen")
        .b(&[0xDD, 0x19, 0x9F, 0xE6]) // ADD IX,DE; SBC A,A; AND vol1
        .label("vol1")
        .b(&[0x00, 0x47, 0xD9, 0x19, 0xD9, 0x9F, 0xE6]) // LD B,A; EXX; ADD HL,DE; EXX; SBC A,A; AND vol2
        .label("vol2")
        .b(&[0x00, 0xB0, 0x28, 0x07]) // OR B; JR Z,+7
        .b(&[0xED, 0x69, 0x47, 0x10, 0xFE, 0xED, 0x41]) // OUT (C),L; LD B,A; DJNZ $; OUT (C),B
        .b(&[0x2F, 0xC6, 0x20, 0x47, 0x10, 0xFE, 0x25]) // CPL; ADD A,20h; LD B,A; DJNZ $; DEC H
        .a(&[0xC2], "gen") // JP NZ,gen
        .a(&[0xC3], "tick") // JP tick
        .label("kick")
        .b(&[0xE5, 0xC5, 0xD5, 0x21, 0x00, 0x00, 0x01, 0x10, 0x16]) // PUSH HL, BC, DE; LD HL,0; LD BC,1610h
        .label("kl")
        .b(&[0x7E, 0xA1, 0xD3, 0xFE, 0x23, 0x3E, 0x6E, 0x90, 0x90, 0x5F, 0xA1, 0xD3, 0xFE]) // LD A,(HL); AND C; OUT; INC HL; LD A,6Eh; SUB B x2; LD E,A; AND C; OUT
        .label("kd")
        .b(&[0x1D])
        .r(0x20, "kd") // DEC E; JR NZ,kd
        .r(0x10, "kl") // DJNZ kl
        .b(&[0xD1, 0xC1, 0xE1, 0xC9]) // POP DE, BC, HL; RET
        .label("hat")
        .b(&[0xE5, 0xC5, 0x21, 0x00, 0x00, 0x06, 0x80]) // PUSH HL, BC; LD HL,0; LD B,80h
        .label("hl")
        .b(&[0x7E, 0x23, 0xE6, 0x10, 0xD3, 0xFE, 0xAF, 0x05, 0xD3, 0xFE]) // LD A,(HL); INC HL; AND 10h; OUT; XOR A; DEC B; OUT
        .r(0x20, "hl") // JR NZ,hl
        .b(&[0xC1, 0xE1, 0xC9]) // POP BC, HL; RET
        .label("stop")
        .r(0x18, "stop") // JR stop
        .label("ticks");
    for (s1, s2, v1, v2, d) in STOCKER_TICKS {
        a.b(&s1.to_le_bytes()).b(&s2.to_le_bytes()).b(&[v1, v2, d]);
    }
    a.b(&[0xFF, 0xFF, 0, 0, 0, 0, 0]);
    let (org, code, labels) = a.done();
    (org, code, labels["gen"])
}

#[test]
fn stocker_is_read_from_its_ticks() {
    for org in [0x8000u16, 0xA321] {
        let (at, code, gen) = stocker_program(org);
        let load = || machine(&[(at, &code)], at);
        let r = read(&load, 30);
        assert_eq!((r.engine, r.channels, r.sites.anchor, r.sites.row_pc), ("Stocker", 2, gen, gen - 3), "found at {org:04X}h");
        let kinds: Vec<&str> = r.sites.drums.iter().map(|d| d.1).collect();
        assert_eq!(kinds, ["kick", "hat"]);
        assert_eq!(r.rows.len(), STOCKER_TICKS.len());
        assert!(r.rows.iter().all(|x| x.passes == 0x80));
        let rows = row_t(&r);
        // a note while a channel sounds, a new one when its volume rises or its step moves; its
        // level from its volume (255 x volume / 30)
        let lv = |v: f64| (255.0 * v / 30.0).round() as u8;
        assert_eq!(shape(&r), [(0, 2000, lv(16.0)), (1, 3000, lv(12.0)), (0, 2000, lv(16.0)), (0, 2500, lv(16.0)), (1, 4000, lv(10.0))]);
        let n = &r.notes;
        assert!(spans(&n[0], &rows, 0, Some(2)) && spans(&n[1], &rows, 0, Some(4)) && spans(&n[2], &rows, 2, Some(3)));
        assert!(spans(&n[3], &rows, 3, Some(5)) && spans(&n[4], &rows, 5, None));
        // the pass time is the ticks' over their passes; the drums' ticks take longer
        let tick = (rows[2] - rows[1]) as f64;
        assert!((r.pass_t * 128.0 / tick - 1.0).abs() < 0.05, "pass {} against a tick of {tick}", r.pass_t);
        assert!(rows[3] - rows[2] > rows[2] - rows[1], "the tick before the hat's is longer");
        for x in n {
            assert!((x.period_t - 65536.0 * r.pass_t / f64::from(x.value)).abs() < 1e-6);
        }
        let d: Vec<(usize, &str)> = r.drums.iter().map(|h| (rows.iter().position(|&t| t > h.t).unwrap(), h.kind)).collect();
        assert_eq!(d, [(0, "kick"), (3, "hat")], "each before its tick's sound");
    }
}

/// Two engines in one program are both found; the run's search takes the one asked for, and
/// says which it found when none fits.
#[test]
fn the_engine_asked_for() {
    let (a, ntropic) = ntropic_program(0x8000);
    let (b, stocker, _) = stocker_program(0xA000);
    let load = || machine(&[(a, &ntropic), (b, &stocker)], a);
    let none = |_| ([0u8; 8], 0u8);
    let names: Vec<&str> = identify_all(&memory(&load())).iter().map(|(e, _)| e.name()).collect();
    assert_eq!(names, ["ntropic", "Stocker"]);
    let (e, s, _) = locate_in_run(&load, &none, 30, &|s| s.near(0xA040, 0x100)).ok().unwrap();
    assert_eq!((e.name(), s.anchor > 0xA000), ("Stocker", true));
    let other = locate_in_run(&load, &none, 30, &|s| s.near(0x6000, 0x100)).err().unwrap().unwrap();
    assert_eq!(other.0.name(), "ntropic");
}

/// No engine is found in memory without one, nor in a program that only resembles one.
#[test]
fn no_engine_without_one() {
    let m = Machine::new();
    assert!(identify(&memory(&m)).is_none());
    // ntropic's row read without its sound loop
    let (at, mut code) = ntropic_program(0x8000);
    let p = code.windows(5).position(|w| w == [0x08, 0xD3, 0xFE, 0x15, 0xC2]).unwrap();
    code[p] = 0x00;
    let m = machine(&[(at, &code)], at);
    assert!(identify(&memory(&m)).is_none());
}
