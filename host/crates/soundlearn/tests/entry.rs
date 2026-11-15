// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Learned entries as data: the program encoder and reference evaluator against the C
//! interpreter (`core/sound/learned.c`), and the decoder's validation of entry lists.

use soundlearn::{encode_list, encode_prog, eval_prog, LearnedEntry, Op};
use zx_sys::{zx_regs, Machine, SndEngines, SND_SITE_TONAL};

/// xorshift64*: a seeded generator, so a failure repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_regs(r: &mut Rng) -> zx_regs {
    let mut w = || r.next() as u16;
    zx_regs {
        af: w(),
        bc: w(),
        de: w(),
        hl: w(),
        af_: w(),
        bc_: w(),
        de_: w(),
        hl_: w(),
        ix: w(),
        iy: w(),
        sp: w(),
        pc: w(),
        i: 0,
        r: 0,
        iff1: 0,
        iff2: 0,
        im: 1,
    }
}

/// An operand that pushes a value.
fn leaf(r: &mut Rng) -> Op {
    match r.below(8) {
        0 => Op::K(match r.below(4) {
            0 => r.below(4) as u32,
            1 => r.below(256) as u32,
            2 => r.below(65536) as u32,
            _ => r.next() as u32,
        }),
        1 => Op::PeekRel(r.next() as u16),
        2 => Op::Peek16Rel(r.next() as u16),
        3 => Op::Peek(r.next() as u16),
        4 => Op::Peek16(r.next() as u16),
        5 => Op::PeekReg(8 + r.below(5) as u8, r.next() as i8),
        _ => Op::Reg(r.below(18) as u8),
    }
}

/// A random valid program: at most 8 deep, at most 64 bytes, one value at its end.
fn random_prog(r: &mut Rng) -> Vec<Op> {
    loop {
        let mut ops = Vec::new();
        let mut depth = 0u32;
        let steps = 1 + r.below(14);
        for _ in 0..steps {
            let roll = r.below(10);
            if depth == 0 || (depth < 8 && roll < 4) {
                ops.push(leaf(r));
                depth += 1;
            } else if depth >= 2 && roll < 8 {
                ops.push([Op::Add, Op::Sub, Op::Mul, Op::Div, Op::And][r.below(5) as usize]);
                depth -= 1;
            } else if roll == 8 {
                ops.push(if r.below(2) == 0 { Op::Shr(r.below(32) as u8) } else { Op::Shl(r.below(32) as u8) });
            } else {
                ops.push(Op::ZeroSilent);
                depth -= 1;
            }
        }
        while depth != 1 {
            if depth == 0 {
                ops.push(leaf(r));
                depth = 1;
            } else {
                ops.push([Op::Add, Op::Mul, Op::Div][r.below(3) as usize]);
                depth -= 1;
            }
        }
        if encode_prog(&ops).len() <= 64 {
            return ops;
        }
    }
}

#[test]
fn k_takes_the_shortest_form() {
    assert_eq!(encode_prog(&[Op::K(5)]), [0x01, 5]);
    assert_eq!(encode_prog(&[Op::K(0x1234)]), [0x02, 0x34, 0x12]);
    assert_eq!(encode_prog(&[Op::K(0x12_3456)]), [0x03, 0x56, 0x34, 0x12, 0x00]);
    assert_eq!(encode_prog(&[Op::PeekReg(11, -2), Op::Shr(3)]), [0x14, 11, 0xFE, 0x35, 3]);
}

#[test]
fn prog_matches_reference() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let mut m = Machine::new();
    for a in 0x4000u32..0x10000 {
        m.poke(a as u16, r.next() as u8);
    }
    let (mut values, mut silent) = (0, 0);
    for k in 0..200 {
        let ops = random_prog(&mut r);
        let prog = encode_prog(&ops);
        let ctx = random_regs(&mut r);
        let anchor = r.next() as u16;
        let want = eval_prog(&ops, |a| m.peek(a), anchor, &ctx);
        let (rc, v) = zx_sys::snd_prog_eval(&prog, &m, anchor, &ctx);
        let got = match rc {
            0 => Some(Some(v)),
            1 => Some(None),
            _ => None,
        };
        assert_eq!(got, want, "program {k}: {ops:?} ({prog:02X?})");
        match want {
            Some(Some(_)) => values += 1,
            Some(None) => silent += 1,
            None => panic!("program {k} is invalid: {ops:?}"),
        }
    }
    println!("{values} values, {silent} silent");
    assert!(values >= 100 && silent >= 10, "{values} values, {silent} silent");
    // invalid programs: both sides say so
    for ops in [vec![Op::Add], vec![Op::K(1), Op::K(2)], vec![Op::Reg(18)], vec![Op::PeekReg(7, 0)], vec![Op::K(1), Op::Shl(32)]] {
        let ctx = random_regs(&mut r);
        assert_eq!(eval_prog(&ops, |a| m.peek(a), 0, &ctx), None, "{ops:?}");
        assert_eq!(zx_sys::snd_prog_eval(&encode_prog(&ops), &m, 0, &ctx).0, -1, "{ops:?}");
    }
}

// Byte offsets in an encoded list with one entry (the list's header is 4 bytes).
const ENTRY: usize = 4;
const SIZE: usize = ENTRY;
const CHANNELS: usize = ENTRY + 3;
const N_SITES: usize = ENTRY + 16;
const PROG_LEN: usize = ENTRY + 32;

/// A valid one-channel entry: 36 bytes of header, wild, reloc, a site, pos, a 3-byte program.
fn base() -> LearnedEntry {
    LearnedEntry {
        kind: 0,
        digest: 0x0123_4567_89AB_CDEF,
        len: 0x20,
        wild: vec![0x05],
        reloc: vec![0x08],
        sites: vec![(0x10, SND_SITE_TONAL, 0)],
        pass: 0,
        pos: vec![0],
        flips: false,
        drum: 0,
        stale: None,
        level: vec![200],
        progs: vec![vec![Op::K(300)]],
        timing: None,
    }
}

fn with_prog(ops: Vec<Op>) -> Vec<u8> {
    encode_list(&[LearnedEntry { progs: vec![ops], ..base() }])
}

fn patched(at: usize, bytes: &[u8]) -> Vec<u8> {
    let mut l = encode_list(&[base()]);
    l[at..at + bytes.len()].copy_from_slice(bytes);
    l
}

#[test]
fn decode_accepts_good_lists() {
    let l = encode_list(&[base()]);
    assert_eq!(l.len(), 56, "{l:02X?}");
    assert_eq!(&l[..4], [1, 0, 0, 0]);
    assert_eq!(u16::from_le_bytes([l[SIZE], l[SIZE + 1]]), 52);
    let b = SndEngines::with_learned(&l).expect("decodes");
    let builtin = SndEngines::new().entries().len();
    let e = b.entries();
    assert_eq!(e.len(), builtin + 1);
    let x = e.last().unwrap();
    assert_eq!((x.name.as_str(), x.len, x.channels, x.pass, x.flips), ("learned", 0x20, 1, 0, false));
    assert_eq!(x.sites.len(), 1);
    assert_eq!((x.sites[0].off, x.sites[0].role, x.sites[0].chan), (0x10, SND_SITE_TONAL, 0));

    // no entries; and three channels with timing, stale and a drum site
    assert!(SndEngines::with_learned(&encode_list(&[])).is_ok());
    let big = LearnedEntry {
        sites: vec![(0x10, SND_SITE_TONAL, 0), (0x14, SND_SITE_TONAL, 1), (0x18, SND_SITE_TONAL, 2), (0x02, zx_sys::SND_SITE_DRUM, 0)],
        pass: 150,
        pos: vec![0, 40, 90],
        drum: 200,
        stale: Some((0x00, 0x08)),
        level: vec![200, 150, 100],
        progs: vec![vec![Op::PeekRel(3), Op::ZeroSilent, Op::K(300)], vec![Op::Reg(10), Op::K(2), Op::Mul], vec![Op::Peek16(0x5C00)]],
        timing: Some(vec![Op::Reg(11), Op::K(156), Op::Mul]),
        ..base()
    };
    let l = encode_list(&[base(), big.clone(), big]);
    assert_eq!(l.len() % 4, 0);
    let e = SndEngines::with_learned(&l).expect("decodes").entries();
    assert_eq!(e.len(), builtin + 3);
    let x = e.last().unwrap();
    assert_eq!((x.channels, x.pass, x.pos.clone(), x.sites.len()), (3, 150, vec![0, 40, 90], 4));
}

/// The table the firmware composes at game start (platform/fm1/main.c, game_start): the built-in
/// entries in their order, then the game's learned ones.
#[test]
fn builtin_then_learned_table() {
    let names: Vec<String> = SndEngines::with_learned(&encode_list(&[base()])).expect("decodes").entries().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["Castaway", "Colony 8", "ROM BEEP", "Cave Runner title", "Cave Runner in game", "Unison", "learned"]);
}

#[test]
fn decode_rejects_bad_lists() {
    let mut bad: Vec<(&str, Vec<u8>)> = vec![
        ("a count of 9", patched(0, &[9])),
        ("a size not a multiple of 4", patched(SIZE, &50u16.to_le_bytes())),
        ("a size past the list", patched(SIZE, &56u16.to_le_bytes())),
        ("n_sites arrays past the size", patched(N_SITES, &[4])),
        ("a stack underflow", with_prog(vec![Op::Add])),
        ("a final depth of 2", with_prog(vec![Op::K(1), Op::K(2)])),
        ("an operand past the end", patched(PROG_LEN, &[2])),
        ("channels 0", patched(CHANNELS, &[0])),
        ("channels 4", patched(CHANNELS, &[4])),
        ("a list shorter than its header", vec![1, 0]),
    ];
    let mut long = vec![Op::K(70_000)];
    for _ in 0..10 {
        long.extend([Op::K(70_000), Op::Add]);
    }
    assert_eq!(encode_prog(&long).len(), 65);
    bad.push(("a program of 65 bytes", with_prog(long[..long.len() - 2].to_vec())));
    assert!(SndEngines::with_learned(&bad.last().unwrap().1).is_ok(), "59 bytes are fine");
    bad.pop();
    bad.push(("a program of 65 bytes", with_prog(long)));
    for (what, l) in &bad {
        assert!(SndEngines::with_learned(l).is_err(), "{what}: accepted {l:02X?}");
    }
    // each truncation of a good list
    let good = encode_list(&[base()]);
    for n in 0..good.len() {
        assert!(SndEngines::with_learned(&good[..n]).is_err(), "truncated to {n}");
    }
}

/// Raw bytes of a valid list, patched where the encoder would never put them: each is a boundary
/// the firmware's decoder alone defends. The offsets follow `base()`: wild at +36, reloc at +38,
/// one site at +40 (off, role, chan), pos at +44, the program at +46.
#[test]
fn decode_rejects_raw_bytes() {
    const KIND: usize = ENTRY + 2;
    const FLIPS: usize = ENTRY + 17;
    const SITE_OFF: usize = ENTRY + 40;
    const SITE_CHAN: usize = ENTRY + 43;
    const PROG: usize = ENTRY + 46;
    assert_eq!(patched(PROG, &[0x02]), encode_list(&[base()]), "the program offset");
    let mut push9 = vec![Op::K(1); 9];
    push9.extend([Op::Add; 8]);
    let two_relocs = |reloc: Vec<u16>| encode_list(&[LearnedEntry { reloc, ..base() }]);
    let bad: Vec<(&str, Vec<u8>)> = vec![
        ("a program that pushes 9 values", with_prog(push9)),
        ("an unknown opcode 0xFF", patched(PROG, &[0xFF])),
        ("kind 1", patched(KIND, &[1])),
        ("flips 2", patched(FLIPS, &[2])),
        ("a tonal site channel past the channels", patched(SITE_CHAN, &[1])),
        ("reloc offsets descending", two_relocs(vec![0x0C, 0x08])),
        ("reloc offsets equal", two_relocs(vec![0x08, 0x08])),
        ("reloc offsets closer than 2", two_relocs(vec![0x08, 0x09])),
        ("a reloc offset past len", two_relocs(vec![0x08, 0x20])),
        ("a site offset equal to len", patched(SITE_OFF, &0x20u16.to_le_bytes())),
        ("a site offset past len", patched(SITE_OFF, &0xFFFFu16.to_le_bytes())),
        ("a wild offset past len", patched(ENTRY + 36, &0x20u16.to_le_bytes())),
        ("a site role past STOP", patched(ENTRY + 42, &[0x7F])),
    ];
    for (what, l) in &bad {
        assert!(SndEngines::with_learned(l).is_err(), "{what}: accepted {l:02X?}");
    }
}

#[test]
fn generic_reader() {
    // one channel, its period HL at the site; timing IX; stale while the frame ends at +40h-4Fh
    let entry = LearnedEntry {
        progs: vec![vec![Op::Reg(10)]],
        timing: Some(vec![Op::Reg(11)]),
        stale: Some((0x40, 0x10)),
        level: vec![123],
        ..base()
    };
    let b = SndEngines::with_learned(&encode_list(&[entry.clone(), LearnedEntry { timing: None, ..entry }])).unwrap();
    let (timed, untimed) = (b.entries().len() - 2, b.entries().len() - 1);
    const ANCHOR: u16 = 0x8000;
    let mut m = Machine::new();
    let mut r = Rng(1);
    let mut at_end = |m: &mut Machine, pc: u16, ix: u16| {
        let mut now = random_regs(&mut r);
        now.pc = pc;
        now.ix = ix;
        m.set_regs(&now);
    };
    let site = |hl: u16, ix: u16| zx_regs { hl, ix, ..random_regs(&mut Rng(2)) };

    at_end(&mut m, ANCHOR + 0x10, 300);
    let rd = b.read_full(&m, timed, ANCHOR, 0, &site(1234, 100));
    assert_eq!((rd.v[0].period_q8, rd.v[0].level, rd.v[1].period_q8), (1234, 123, 0));
    assert_eq!((rd.since, rd.row, rd.stale), (100, 100, 0), "the note held 100 T at the edge");
    let rd = b.read_full(&m, untimed, ANCHOR, 0, &site(1234, 100));
    assert_eq!((rd.since, rd.row, rd.stale), (zx_sys::SND_B_SINCE_ALL, zx_sys::SND_B_SINCE_ALL, 0), "no timing");
    assert_eq!(b.read_full(&m, timed, ANCHOR, 0, &site(0, 100)).v[0].period_q8, 0, "HL 0: silent");

    at_end(&mut m, ANCHOR + 0x10, 50);
    let rd = b.read_full(&m, timed, ANCHOR, 0, &site(1234, 100));
    assert_eq!((rd.since, rd.row, rd.stale), (50, 50, 1), "a new note began after the edge");

    for pc in [ANCHOR + 0x40, ANCHOR + 0x4F] {
        at_end(&mut m, pc, 300);
        assert_eq!(b.read_full(&m, timed, ANCHOR, 0, &site(1234, 100)).stale, 2, "pc {pc:04X}");
    }
    at_end(&mut m, ANCHOR + 0x50, 300);
    assert_eq!(b.read_full(&m, timed, ANCHOR, 0, &site(1234, 100)).stale, 0, "pc past the range");
    // the range wraps with the anchor
    let b2 = SndEngines::with_learned(&encode_list(&[LearnedEntry { stale: Some((0xFFF0, 0x20)), ..base() }])).unwrap();
    at_end(&mut m, 0x0005, 0);
    assert_eq!(b2.read_full(&m, b2.entries().len() - 1, 0x0000, 0, &site(1, 0)).stale, 2);
}
