// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Speaker routines for the code reader's tests: `games/test/tune.sna` and routines generated
//! here as 48K snapshots, each with its listing and its true half period.
//!
//! # Layout
//!
//! As `fmzx-pack`'s test programs (`testprogs.rs`): 48K of RAM from 4000h, the routine at 8000h,
//! SP FD00h (the `.sna` keeps PC on the stack), the IM 2 handler `RET` at FDFDh and its vector
//! table (257 bytes of FDh at FE00h), I = FEh, IM 2. Interrupts are off at load and the
//! generated routines never enable them, so they run undisturbed and keep exact time (only
//! `tune.sna` waits for the frame interrupt with `EI; HALT`, as its listing in `testprogs.rs`
//! says). Every routine and its data lie in uncontended RAM (8000h-FFFFh): the only contention
//! is the `OUT (FEh)` I/O cycle's, which [`trace_outs`] takes out.
//!
//! # Truth
//!
//! Each fixture's `truth` gives, per channel, the true half period in T-states at one of its
//! `OUT`s, from the memory and the registers then, computed from the listing (not by the code
//! reader). The test `fixtures_sound_as_listed` runs every fixture and checks its edges against
//! it.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use zx_sys::{zx_regs, Machine, TraceEv, ZX_FRAME_TSTATES, ZX_TR_OUT, ZX_TR_WRITE};

/// Where the routines start.
pub const CODE: u16 = 0x8000;
const RAM: u16 = 0x4000;
const STACK: u16 = 0xFD00;
const HANDLER: u16 = 0xFDFD;
const VECTORS: u16 = 0xFE00;

/// Per channel, the true half period (T) at an `OUT`, from the memory and the registers then.
pub type Truth = fn(&dyn Fn(u16) -> u8, &zx_regs) -> Vec<f64>;

/// One routine.
pub struct Fixture {
    pub name: &'static str,
    pub sna: Vec<u8>,
    /// The speaker `OUT` sites, ascending.
    pub sites: Vec<u16>,
    /// How each channel's half period is measured from the run (see [`Kind`]).
    pub kind: Kind,
    pub truth: Truth,
}

/// How a fixture's channels show in its `OUT`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One channel: every speaker edge is a half period.
    Square,
    /// One channel per site, each `OUT` of a site its channel's flip.
    Flips,
    /// One channel per site, each site's `OUT`s setting its channel's level once a pass.
    Slots,
}

/// A 48K `.sna` with `code` at 8000h and `data` blocks at their addresses (see the module docs).
fn sna(code: &[u8], data: &[(u16, &[u8])]) -> Vec<u8> {
    let mut ram = vec![0u8; 0xC000];
    let at = |a: u16| (a - RAM) as usize;
    ram[at(CODE)..at(CODE) + code.len()].copy_from_slice(code);
    for (addr, bytes) in data {
        ram[at(*addr)..at(*addr) + bytes.len()].copy_from_slice(bytes);
    }
    ram[at(HANDLER)] = 0xC9; // RET
    ram[at(VECTORS)..=at(VECTORS) + 256].fill((HANDLER >> 8) as u8);
    let sp = STACK - 2; // PC is popped from the stack
    ram[at(sp)..at(sp) + 2].copy_from_slice(&CODE.to_le_bytes());
    let mut h = vec![0xFEu8]; // I
    h.extend_from_slice(&[0; 18]); // HL' DE' BC' AF' HL DE BC IY IX
    h.push(0); // IFF2 (bit 2) clear: interrupts off
    h.push(0); // R
    h.extend_from_slice(&[0, 0]); // AF
    h.extend_from_slice(&sp.to_le_bytes());
    h.push(2); // IM 2
    h.push(0); // border
    debug_assert_eq!(h.len(), 27);
    h.extend_from_slice(&ram);
    h
}

/// A machine with `sna` loaded.
pub fn machine(sna: &[u8]) -> Machine {
    let mut m = Machine::new();
    m.load_sna(sna).expect("loads");
    m
}

fn peek16(peek: &dyn Fn(u16) -> u8, a: u16) -> u32 {
    u32::from(peek(a)) | u32::from(peek(a.wrapping_add(1))) << 8
}

/// `games/test/tune.sna` (listing in `fmzx-pack`'s `testprogs.rs`): one edge every
/// `87 + 13n + 16m` T, `n` and `m` the note's bytes at IX+2 and IX+3 (`LD A,D` 4, `XOR 10h` 7,
/// `LD D,A` 4, `OUT` 11, `DEC HL` 6, `LD A,H` 4, `OR L` 4, `JR Z` 7, two `LD r,(IX+d)` 19 each,
/// `JR tog` 12: 97 T, the `DJNZ` loop `13n - 5`, the `DEC C; JR NZ` loop `16m - 5`).
pub fn square_tune() -> Fixture {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/test/tune.sna");
    Fixture {
        name: "square_tune",
        sna: std::fs::read(path).expect("tune.sna"),
        sites: vec![0x8014],
        kind: Kind::Square,
        truth: |peek, r| {
            let (n, m) = (peek(r.ix.wrapping_add(2)), peek(r.ix.wrapping_add(3)));
            vec![87.0 + 13.0 * f64::from(n) + 16.0 * f64::from(m)]
        },
    }
}

/// The notes of `tune.sna` as (K, n, m) (`TUNE_TABLE` in `testprogs.rs`) and its record table.
pub const TUNE_TABLE: [(u16, u8, u8); 8] =
    [(523, 244, 5), (585, 207, 13), (656, 192, 5), (700, 173, 10), (780, 157, 7), (875, 132, 12), (989, 123, 5), (1047, 113, 7)];
pub const TUNE_TABLE_AT: u16 = 0x802F;
/// The pitches `fmzx-pack` measured from the host beeper renderer (`TUNE_HZ`).
pub const TUNE_HZ: [f64; 8] = [524.67, 586.65, 657.01, 701.11, 781.02, 876.39, 989.70, 1049.28];

/// A `DJNZ` delay reloaded from C, which the note's setup loads from a table walked by HL; the
/// level lives in A' and DE counts the edges down. Half period `13n + 58` T.
///
/// ```text
/// 8000 21 1F 80     start: LD HL,801Fh    the first record: n, K (edges, 16 bits)
/// 8003 7E           note:  LD A,(HL)      n (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 4F                  LD C,A         the delay
/// 8008 23                  INC HL
/// 8009 5E                  LD E,(HL)
/// 800A 23                  INC HL
/// 800B 56                  LD D,(HL)      DE = K
/// 800C 23                  INC HL
/// 800D 08           tog:   EX AF,AF'      4    A' = the level
/// 800E EE 10               XOR 10h        7
/// 8010 D3 FE               OUT (FEh),A    11   the edge
/// 8012 08                  EX AF,AF'      4
/// 8013 1B                  DEC DE         6
/// 8014 7A                  LD A,D         4
/// 8015 B3                  OR E           4
/// 8016 28 05               JR Z,next      7    the note's last edge
/// 8018 41                  LD B,C         4
/// 8019 10 FE        d:     DJNZ d         13n - 5
/// 801B 18 F0               JR tog         12
/// 801D 18 E4        next:  JR note
/// 801F                     records (n, K) and a 0
/// ```
pub fn square_one_counter() -> Fixture {
    #[rustfmt::skip]
    let mut code = vec![
        0x21, 0x1F, 0x80, 0x7E, 0xB7, 0x28, 0xF9, 0x4F, 0x23, 0x5E, 0x23, 0x56, 0x23,
        0x08, 0xEE, 0x10, 0xD3, 0xFE, 0x08, 0x1B, 0x7A, 0xB3, 0x28, 0x05, 0x41, 0x10, 0xFE,
        0x18, 0xF0, 0x18, 0xE4,
    ];
    assert_eq!(code.len(), 0x1F);
    for n in [250u8, 222, 198, 187, 166, 148, 131, 124] {
        code.extend_from_slice(&[n, 0x2C, 0x01]); // K = 300 edges
    }
    code.push(0);
    Fixture {
        name: "square_one_counter",
        sna: sna(&code, &[]),
        sites: vec![0x8010],
        kind: Kind::Square,
        // HL points past the note's record at its edges
        truth: |peek, r| vec![13.0 * f64::from(peek(r.hl.wrapping_sub(3))) + 58.0],
    }
}

/// One counter, two sites: each half period is a `DJNZ` of B loaded from C (the note's byte at
/// IX+0), the first ending high, the second low; D counts the cycles down from IX+1. From site 1
/// to site 2 `17 + 13n` T, back `33 + 13n`: half period `25 + 13n` on average.
///
/// ```text
/// 8000 DD 21 25 80  start: LD IX,8025h    the first record: n, cycles
/// 8004 DD 4E 00     note:  LD C,(IX+0)    n (0: the end of the table)
/// 8007 79                  LD A,C
/// 8008 B7                  OR A
/// 8009 28 F5               JR Z,start
/// 800B DD 56 01            LD D,(IX+1)    cycles
/// 800E 3E 10        cyc:   LD A,10h       7
/// 8010 D3 FE               OUT (FEh),A    11   site 1: high
/// 8012 41                  LD B,C         4
/// 8013 10 FE        d1:    DJNZ d1        13n - 5
/// 8015 3E 00               LD A,0         7
/// 8017 D3 FE               OUT (FEh),A    11   site 2: low
/// 8019 41                  LD B,C         4
/// 801A 10 FE        d2:    DJNZ d2        13n - 5
/// 801C 15                  DEC D          4
/// 801D 20 EF               JR NZ,cyc      12
/// 801F DD 23               INC IX
/// 8021 DD 23               INC IX
/// 8023 18 DF               JR note
/// 8025                     records (n, cycles) and a 0
/// ```
pub fn toggler_two_sites() -> Fixture {
    #[rustfmt::skip]
    let code = [
        0xDD, 0x21, 0x25, 0x80, 0xDD, 0x4E, 0x00, 0x79, 0xB7, 0x28, 0xF5, 0xDD, 0x56, 0x01,
        0x3E, 0x10, 0xD3, 0xFE, 0x41, 0x10, 0xFE, 0x3E, 0x00, 0xD3, 0xFE, 0x41, 0x10, 0xFE,
        0x15, 0x20, 0xEF, 0xDD, 0x23, 0xDD, 0x23, 0x18, 0xDF,
    ];
    Fixture {
        name: "toggler_two_sites",
        sna: toggler_sna(&code, 0x25),
        sites: vec![0x8010, 0x8017],
        kind: Kind::Square,
        truth: |peek, r| vec![25.0 + 13.0 * f64::from(peek(r.ix))],
    }
}

/// The routine `code` with the togglers' note records after it (at 8000h + `table`).
fn toggler_sna(code: &[u8], table: usize) -> Vec<u8> {
    assert_eq!(code.len(), table);
    let mut code = code.to_vec();
    for (n, cycles) in [(200u8, 150u8), (178, 170), (159, 190), (150, 200), (133, 220), (119, 250)] {
        code.extend_from_slice(&[n, cycles]);
    }
    code.push(0);
    sna(&code, &[])
}

/// As [`toggler_two_sites`], but the two half periods count separate registers loaded from the
/// same byte: B (`DJNZ`) and C (`DEC C; JR NZ`), both from IX+0. From site 1 to site 2
/// `32 + 13n` T, back `48 + 16n`: half period `40 + 14.5n` on average. One channel toggled from
/// two places, or two? The code does not say.
///
/// ```text
/// 8000 DD 21 29 80  start: LD IX,8029h
/// 8004 DD 7E 00     note:  LD A,(IX+0)    n (0: the end of the table)
/// 8007 B7                  OR A
/// 8008 28 F6               JR Z,start
/// 800A DD 56 01            LD D,(IX+1)    cycles
/// 800D 3E 10        cyc:   LD A,10h       7
/// 800F D3 FE               OUT (FEh),A    11   site 1: high
/// 8011 DD 46 00            LD B,(IX+0)    19
/// 8014 10 FE        d1:    DJNZ d1        13n - 5
/// 8016 3E 00               LD A,0         7
/// 8018 D3 FE               OUT (FEh),A    11   site 2: low
/// 801A DD 4E 00            LD C,(IX+0)    19
/// 801D 0D           d2:    DEC C
/// 801E 20 FD               JR NZ,d2       16n - 5
/// 8020 15                  DEC D          4
/// 8021 20 EA               JR NZ,cyc      12
/// 8023 DD 23               INC IX
/// 8025 DD 23               INC IX
/// 8027 18 DB               JR note
/// 8029                     records (n, cycles) and a 0
/// ```
pub fn toggler_ambiguous() -> Fixture {
    #[rustfmt::skip]
    let code = [
        0xDD, 0x21, 0x29, 0x80, 0xDD, 0x7E, 0x00, 0xB7, 0x28, 0xF6, 0xDD, 0x56, 0x01,
        0x3E, 0x10, 0xD3, 0xFE, 0xDD, 0x46, 0x00, 0x10, 0xFE, 0x3E, 0x00, 0xD3, 0xFE,
        0xDD, 0x4E, 0x00, 0x0D, 0x20, 0xFD, 0x15, 0x20, 0xEA, 0xDD, 0x23, 0xDD, 0x23, 0x18, 0xDB,
    ];
    Fixture {
        name: "toggler_ambiguous",
        sna: toggler_sna(&code, 0x29),
        sites: vec![0x800F, 0x8018],
        kind: Kind::Square,
        truth: |peek, r| vec![40.0 + 14.5 * f64::from(peek(r.ix))],
    }
}

/// Two channels flipping one level byte (9000h) in unison, Unison style but balanced: each pass
/// D and E count down, and the one that reaches 0 is reloaded (from 9001h, 9002h: n1, n2) and
/// XORs 10h into the level and OUTs it; the other path pads to the same time (73 T). A pass is
/// 180 T, so channel c flips every `180 n_c` T. BC counts the note's passes. The table's notes
/// are a step apart (n2 = n1 + 1).
///
/// ```text
/// 8000 21 5F 80     start: LD HL,805Fh    the first record: n1, n2, passes (16 bits)
/// 8003 7E           note:  LD A,(HL)      n1 (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 32 01 90            LD (9001h),A
/// 800A 23                  INC HL
/// 800B 7E                  LD A,(HL)
/// 800C 32 02 90            LD (9002h),A   n2
/// 800F 23                  INC HL
/// 8010 4E                  LD C,(HL)
/// 8011 23                  INC HL
/// 8012 46                  LD B,(HL)      BC = passes
/// 8013 23                  INC HL
/// 8014 16 01               LD D,1         both flip on the first pass
/// 8016 1E 01               LD E,1
/// 8018 15           loop:  DEC D          4
/// 8019 28 0F               JR Z,r1        7 / 12
/// 801B 3A 01 90            LD A,(9001h)   13   padding: 54 T
/// 801E 3A 01 90            LD A,(9001h)   13
/// 8021 3A 01 90            LD A,(9001h)   13
/// 8024 3E 00               LD A,0         7
/// 8026 00                  NOP            4
/// 8027 00                  NOP            4
/// 8028 18 0E               JR s1          12
/// 802A 3A 01 90     r1:    LD A,(9001h)   13
/// 802D 57                  LD D,A         4    reload n1
/// 802E 3A 00 90            LD A,(9000h)   13
/// 8031 EE 10               XOR 10h        7
/// 8033 32 00 90            LD (9000h),A   13
/// 8036 D3 FE               OUT (FEh),A    11   channel 0's flip
/// 8038 1D           s1:    DEC E          4
/// 8039 28 0F               JR Z,r2        7 / 12
/// 803B 3A 02 90            LD A,(9002h)   13   padding: 54 T
/// 803E 3A 02 90            LD A,(9002h)   13
/// 8041 3A 02 90            LD A,(9002h)   13
/// 8044 3E 00               LD A,0         7
/// 8046 00                  NOP            4
/// 8047 00                  NOP            4
/// 8048 18 0E               JR s2          12
/// 804A 3A 02 90     r2:    LD A,(9002h)   13
/// 804D 5F                  LD E,A         4    reload n2
/// 804E 3A 00 90            LD A,(9000h)   13
/// 8051 EE 10               XOR 10h        7
/// 8053 32 00 90            LD (9000h),A   13
/// 8056 D3 FE               OUT (FEh),A    11   channel 1's flip
/// 8058 0B           s2:    DEC BC         6
/// 8059 78                  LD A,B         4
/// 805A B1                  OR C           4
/// 805B 20 BB               JR NZ,loop     12
/// 805D 18 A4               JR note
/// 805F                     records (n1, n2, passes) and a 0
/// ```
pub fn xor_unison() -> Fixture {
    #[rustfmt::skip]
    let mut code = vec![
        0x21, 0x5F, 0x80, 0x7E, 0xB7, 0x28, 0xF9, 0x32, 0x01, 0x90, 0x23, 0x7E, 0x32, 0x02, 0x90,
        0x23, 0x4E, 0x23, 0x46, 0x23, 0x16, 0x01, 0x1E, 0x01,
        // loop
        0x15, 0x28, 0x0F, 0x3A, 0x01, 0x90, 0x3A, 0x01, 0x90, 0x3A, 0x01, 0x90, 0x3E, 0x00, 0x00, 0x00,
        0x18, 0x0E, 0x3A, 0x01, 0x90, 0x57, 0x3A, 0x00, 0x90, 0xEE, 0x10, 0x32, 0x00, 0x90, 0xD3, 0xFE,
        // s1
        0x1D, 0x28, 0x0F, 0x3A, 0x02, 0x90, 0x3A, 0x02, 0x90, 0x3A, 0x02, 0x90, 0x3E, 0x00, 0x00, 0x00,
        0x18, 0x0E, 0x3A, 0x02, 0x90, 0x5F, 0x3A, 0x00, 0x90, 0xEE, 0x10, 0x32, 0x00, 0x90, 0xD3, 0xFE,
        // s2
        0x0B, 0x78, 0xB1, 0x20, 0xBB, 0x18, 0xA4,
    ];
    assert_eq!(code.len(), 0x5F);
    for (n1, n2) in [(10u8, 11u8), (12, 13), (9, 10), (15, 16)] {
        code.extend_from_slice(&[n1, n2, 0x88, 0x13]); // 5,000 passes
    }
    code.push(0);
    Fixture {
        name: "xor_unison",
        sna: sna(&code, &[]),
        sites: vec![0x8036, 0x8056],
        kind: Kind::Flips,
        truth: |peek, _| vec![180.0 * f64::from(peek(0x9001)), 180.0 * f64::from(peek(0x9002))],
    }
}

/// Two phase accumulators, a level per channel: each pass adds the channel's step (9004h,
/// 9006h) to its 16-bit phase (9000h, 9002h) and OUTs the phase's top bit as the speaker bit.
/// A pass is 218 T, each channel's `OUT` 97 T after the other's (pos 0, 97), so a channel's
/// half period is `32768 x 218 / step` T. The note's setup copies the steps from a table walked
/// by HL; BC counts the note's 8,192 passes.
///
/// ```text
/// 8000 21 4A 80     start: LD HL,804Ah    the first record: step 1, step 2 (16 bits each)
/// 8003 5E           note:  LD E,(HL)
/// 8004 23                  INC HL
/// 8005 56                  LD D,(HL)
/// 8006 23                  INC HL
/// 8007 7A                  LD A,D
/// 8008 B3                  OR E
/// 8009 28 F5               JR Z,start     step 1 = 0: the end of the table
/// 800B ED 53 04 90         LD (9004h),DE
/// 800F 5E                  LD E,(HL)
/// 8010 23                  INC HL
/// 8011 56                  LD D,(HL)
/// 8012 23                  INC HL
/// 8013 ED 53 06 90         LD (9006h),DE
/// 8017 E5                  PUSH HL
/// 8018 01 00 20            LD BC,2000h    passes
/// 801B 2A 00 90     pass:  LD HL,(9000h)  16
/// 801E ED 5B 04 90         LD DE,(9004h)  20
/// 8022 19                  ADD HL,DE      11
/// 8023 22 00 90            LD (9000h),HL  16
/// 8026 7C                  LD A,H         4
/// 8027 E6 80               AND 80h        7
/// 8029 0F                  RRCA           4
/// 802A 0F                  RRCA           4
/// 802B 0F                  RRCA           4    the phase's top bit to bit 4
/// 802C D3 FE               OUT (FEh),A    11   channel 0
/// 802E 2A 02 90            LD HL,(9002h)  16
/// 8031 ED 5B 06 90         LD DE,(9006h)  20
/// 8035 19                  ADD HL,DE      11
/// 8036 22 02 90            LD (9002h),HL  16
/// 8039 7C                  LD A,H         4
/// 803A E6 80               AND 80h        7
/// 803C 0F                  RRCA           4
/// 803D 0F                  RRCA           4
/// 803E 0F                  RRCA           4
/// 803F D3 FE               OUT (FEh),A    11   channel 1
/// 8041 0B                  DEC BC         6
/// 8042 78                  LD A,B         4
/// 8043 B1                  OR C           4
/// 8044 C2 1B 80            JP NZ,pass     10
/// 8047 E1                  POP HL
/// 8048 18 B9               JR note
/// 804A                     records (step 1, step 2) and a 0
/// ```
pub fn phase_slots() -> Fixture {
    #[rustfmt::skip]
    let mut code = vec![
        0x21, 0x4A, 0x80, 0x5E, 0x23, 0x56, 0x23, 0x7A, 0xB3, 0x28, 0xF5, 0xED, 0x53, 0x04, 0x90,
        0x5E, 0x23, 0x56, 0x23, 0xED, 0x53, 0x06, 0x90, 0xE5, 0x01, 0x00, 0x20,
        // pass
        0x2A, 0x00, 0x90, 0xED, 0x5B, 0x04, 0x90, 0x19, 0x22, 0x00, 0x90, 0x7C, 0xE6, 0x80,
        0x0F, 0x0F, 0x0F, 0xD3, 0xFE,
        0x2A, 0x02, 0x90, 0xED, 0x5B, 0x06, 0x90, 0x19, 0x22, 0x02, 0x90, 0x7C, 0xE6, 0x80,
        0x0F, 0x0F, 0x0F, 0xD3, 0xFE,
        0x0B, 0x78, 0xB1, 0xC2, 0x1B, 0x80, 0xE1, 0x18, 0xB9,
    ];
    assert_eq!(code.len(), 0x4A);
    for (s1, s2) in [(1796u16, 2262u16), (2016, 2540), (1600, 2400)] {
        code.extend_from_slice(&s1.to_le_bytes());
        code.extend_from_slice(&s2.to_le_bytes());
    }
    code.extend_from_slice(&[0, 0]);
    Fixture {
        name: "phase_slots",
        sna: sna(&code, &[]),
        sites: vec![0x802C, 0x803F],
        kind: Kind::Slots,
        truth: |peek, _| [0x9004, 0x9006].iter().map(|&a| 32768.0 * 218.0 / f64::from(peek16(peek, a))).collect(),
    }
}

/// A falling-pitch sweep, no table: the delay E drops by one every pass, from 200 to 1, then
/// again. Half period `13E + 33` T.
///
/// ```text
/// 8000 1E C8        start: LD E,200
/// 8002 AF                  XOR A
/// 8003 EE 10        tog:   XOR 10h        7
/// 8005 D3 FE               OUT (FEh),A    11
/// 8007 43                  LD B,E         4
/// 8008 10 FE        d:     DJNZ d         13E - 5
/// 800A 1D                  DEC E          4
/// 800B 20 F6               JR NZ,tog      12
/// 800D 18 F1               JR start
/// ```
pub fn sweep() -> Fixture {
    let code = [0x1E, 0xC8, 0xAF, 0xEE, 0x10, 0xD3, 0xFE, 0x43, 0x10, 0xFE, 0x1D, 0x20, 0xF6, 0x18, 0xF1];
    Fixture {
        name: "sweep",
        sna: sna(&code, &[]),
        sites: vec![0x8005],
        kind: Kind::Square,
        truth: |_, r| vec![13.0 * f64::from(r.de & 0xFF) + 33.0],
    }
}

/// A two-tone siren: 64 edges with the delay L = 50, then 64 with L = 80, and again. The port
/// is set in the loop (`OUT (C),A`). Half period `13L + 49` T.
///
/// ```text
/// 8000 1E 00        start: LD E,0         E = the level
/// 8002 2E 32        hi:    LD L,50
/// 8004 18 02               JR go
/// 8006 2E 50        lo:    LD L,80
/// 8008 26 40        go:    LD H,64        edges
/// 800A 7B           tog:   LD A,E         4
/// 800B EE 10               XOR 10h        7
/// 800D 5F                  LD E,A         4
/// 800E 0E FE               LD C,FEh       7
/// 8010 ED 79               OUT (C),A      12
/// 8012 45                  LD B,L         4
/// 8013 10 FE        d:     DJNZ d         13L - 5
/// 8015 25                  DEC H          4
/// 8016 20 F2               JR NZ,tog      12
/// 8018 7D                  LD A,L
/// 8019 FE 32               CP 50
/// 801B 28 E9               JR Z,lo
/// 801D 18 E3               JR hi
/// ```
pub fn siren() -> Fixture {
    #[rustfmt::skip]
    let code = [
        0x1E, 0x00, 0x2E, 0x32, 0x18, 0x02, 0x2E, 0x50, 0x26, 0x40,
        0x7B, 0xEE, 0x10, 0x5F, 0x0E, 0xFE, 0xED, 0x79, 0x45, 0x10, 0xFE, 0x25, 0x20, 0xF2,
        0x7D, 0xFE, 0x32, 0x28, 0xE9, 0x18, 0xE3,
    ];
    Fixture {
        name: "siren",
        sna: sna(&code, &[]),
        sites: vec![0x8010],
        kind: Kind::Square,
        truth: |_, r| vec![13.0 * f64::from(r.hl & 0xFF) + 49.0],
    }
}

/// A tune with a drum between its notes: a driver walks the table and calls the tone routine
/// (`square_one_counter`'s loop, returning at the note's end: half period `13n + 58` T) and then
/// a drum routine of its own, 200 `OUT`s of ROM bytes' bit 4 (noise, 44 T apart).
///
/// ```text
/// 8000 21 00 83     start: LD HL,8300h    the first record: n, K (edges, 16 bits)
/// 8003 7E           next:  LD A,(HL)      n (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 4F                  LD C,A         the delay
/// 8008 23                  INC HL
/// 8009 5E                  LD E,(HL)
/// 800A 23                  INC HL
/// 800B 56                  LD D,(HL)      DE = K
/// 800C 23                  INC HL
/// 800D E5                  PUSH HL
/// 800E CD 00 81            CALL tone
/// 8011 CD 00 82            CALL drum
/// 8014 E1                  POP HL
/// 8015 18 EC               JR next
///
/// 8100 08           tone:  EX AF,AF'      4    A' = the level
/// 8101 EE 10               XOR 10h        7
/// 8103 D3 FE               OUT (FEh),A    11   the edge
/// 8105 08                  EX AF,AF'      4
/// 8106 1B                  DEC DE         6
/// 8107 7A                  LD A,D         4
/// 8108 B3                  OR E           4
/// 8109 28 05               JR Z,done      7
/// 810B 41                  LD B,C         4
/// 810C 10 FE        d:     DJNZ d         13n - 5
/// 810E 18 F0               JR tone        12
/// 8110 C9           done:  RET
///
/// 8200 21 00 00     drum:  LD HL,0
/// 8203 06 C8               LD B,200
/// 8205 7E           noise: LD A,(HL)
/// 8206 23                  INC HL
/// 8207 E6 10               AND 10h
/// 8209 D3 FE               OUT (FEh),A    the drum's OUT
/// 820B 10 F8               DJNZ noise
/// 820D C9                  RET
/// 8300                     records (n, K) and a 0
/// ```
pub fn tune_with_drum() -> Fixture {
    let driver = [0x21, 0x00, 0x83, 0x7E, 0xB7, 0x28, 0xF9, 0x4F, 0x23, 0x5E, 0x23, 0x56, 0x23, 0xE5, 0xCD, 0x00, 0x81, 0xCD, 0x00, 0x82, 0xE1, 0x18, 0xEC];
    let tone = [0x08, 0xEE, 0x10, 0xD3, 0xFE, 0x08, 0x1B, 0x7A, 0xB3, 0x28, 0x05, 0x41, 0x10, 0xFE, 0x18, 0xF0, 0xC9];
    let drum = [0x21, 0x00, 0x00, 0x06, 0xC8, 0x7E, 0x23, 0xE6, 0x10, 0xD3, 0xFE, 0x10, 0xF8, 0xC9];
    let mut table = Vec::new();
    for n in [250u8, 222, 198, 187, 166, 148, 131, 124] {
        table.extend_from_slice(&[n, 0x58, 0x02]); // K = 600 edges
    }
    table.push(0);
    Fixture {
        name: "tune_with_drum",
        sna: sna(&driver, &[(0x8100, &tone), (0x8200, &drum), (0x8300, &table)]),
        sites: vec![0x8103, 0x8209],
        kind: Kind::Square,
        truth: |_, r| vec![13.0 * f64::from(r.bc & 0xFF) + 58.0],
    }
}

/// A fixed tone whose delay is reached through `JP (HL)`: half period 1,346 T (`XOR` 7, `OUT`
/// 11, `LD HL` 10, `JP (HL)` 4, `LD B` 7, the `DJNZ` loop 1,295, `JR` 12).
///
/// ```text
/// 8000 EE 10        tog:   XOR 10h
/// 8002 D3 FE               OUT (FEh),A
/// 8004 21 08 80            LD HL,8008h
/// 8007 E9                  JP (HL)
/// 8008 06 64               LD B,100
/// 800A 10 FE        d:     DJNZ d
/// 800C 18 F2               JR tog
/// ```
pub fn computed_jump() -> Fixture {
    let code = [0xEE, 0x10, 0xD3, 0xFE, 0x21, 0x08, 0x80, 0xE9, 0x06, 0x64, 0x10, 0xFE, 0x18, 0xF2];
    Fixture { name: "computed_jump", sna: sna(&code, &[]), sites: vec![0x8002], kind: Kind::Square, truth: |_, _| vec![1346.0] }
}


/// One channel flipped by an uneven reload: each pass `DJNZ` counts B down, and the pass where
/// it reaches 0 reloads it from C (the note's byte, from a table walked by HL), XORs the level in
/// A and OUTs it, 13 T longer than the other passes (42 T against 29). A pass is 45 T, so a flip
/// comes every `45n + 13` T. The note's passes are counted by E (`DEC E; JR NZ`) and, when E
/// wraps, by D on a path out of the loop and back (`DEC D; JR NZ`): that path costs 11 T more,
/// once every 256 passes, so a half period over it is `45n + 24` T.
///
/// ```text
/// 8000 21 26 80     start: LD HL,8026h    the first record: n, passes (low, high)
/// 8003 7E           note:  LD A,(HL)      n (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 4F                  LD C,A         the reload
/// 8008 23                  INC HL
/// 8009 5E                  LD E,(HL)
/// 800A 23                  INC HL
/// 800B 56                  LD D,(HL)      the note's passes: (D - 1) x 256 + E
/// 800C 23                  INC HL
/// 800D 3E 10               LD A,10h       the level
/// 800F 06 01               LD B,1         the first pass flips
/// 8011 10 07        loop:  DJNZ skip      8 / 13
/// 8013 41                  LD B,C         4    reload
/// 8014 EE 10               XOR 10h        7
/// 8016 D3 FE               OUT (FEh),A    11   the flip
/// 8018 18 04               JR next        12
/// 801A 00 00 00 00  skip:  NOP x 4        16
/// 801E 1D           next:  DEC E          4
/// 801F 20 F0               JR NZ,loop     12
/// 8021 15                  DEC D          4
/// 8022 20 ED               JR NZ,loop     12
/// 8024 18 DD               JR note
/// 8026                     records (n, low, high) and a 0
/// ```
pub fn flips_uneven() -> Fixture {
    #[rustfmt::skip]
    let mut code = vec![
        0x21, 0x26, 0x80, 0x7E, 0xB7, 0x28, 0xF9, 0x4F, 0x23, 0x5E, 0x23, 0x56, 0x23, 0x3E, 0x10, 0x06, 0x01,
        // loop
        0x10, 0x07, 0x41, 0xEE, 0x10, 0xD3, 0xFE, 0x18, 0x04, 0x00, 0x00, 0x00, 0x00,
        // next
        0x1D, 0x20, 0xF0, 0x15, 0x20, 0xED, 0x18, 0xDD,
    ];
    assert_eq!(code.len(), 0x26);
    for n in [40u8, 45, 50, 60, 36, 45] {
        code.extend_from_slice(&[n, 0, 0x20]);
    }
    code.push(0);
    Fixture {
        name: "flips_uneven",
        sna: sna(&code, &[]),
        sites: vec![0x8016],
        kind: Kind::Flips,
        // at the OUT, E has n decrements to go before the next flip: one of them wrapping it
        // (E from 1 to n) takes the 11 T longer way round
        truth: |_, r| {
            let n = f64::from(match (r.bc & 0xFF) as u8 { 0 => 256, c => u32::from(c) });
            let e = f64::from((r.de & 0xFF) as u8);
            vec![45.0 * n + 13.0 + if (1.0..=n).contains(&e) { 11.0 } else { 0.0 }]
        },
    }
}

/// One channel toggled from two sites by a routine called once a note (from a table walked by
/// HL), which saves the note's counts on the stack around each half period's nested delay: the
/// inner `DJNZ` counts B down, then runs 256 times for each further count of C (`DEC C; JR NZ`),
/// `13B + 3339C - 3333` T in all; `POP BC; PUSH BC` puts the counts back. From site 1 to site 2
/// the delay plus 39 T, back the delay plus 62 T: half period `13B + 3339C - 3282.5` T. DE counts
/// the note's cycles.
///
/// ```text
/// 8000 21 2F 80     start: LD HL,802Fh    the first record: B, C, cycles (16 bits)
/// 8003 7E           next:  LD A,(HL)      B (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 47                  LD B,A
/// 8008 23                  INC HL
/// 8009 4E                  LD C,(HL)
/// 800A 23                  INC HL
/// 800B 5E                  LD E,(HL)
/// 800C 23                  INC HL
/// 800D 56                  LD D,(HL)
/// 800E 23                  INC HL
/// 800F CD 14 80            CALL note
/// 8012 18 EF               JR next
/// 8014 C5           note:  PUSH BC        11
/// 8015 AF                  XOR A          4
/// 8016 D3 FE               OUT (FEh),A    11   site 1: low
/// 8018 10 FE        d1:    DJNZ d1        13B - 5, then 3,323 for each further C
/// 801A 0D                  DEC C          4
/// 801B 20 FB               JR NZ,d1       12 / 7
/// 801D C1                  POP BC         10
/// 801E C5                  PUSH BC        11
/// 801F 3E 10               LD A,10h       7
/// 8021 D3 FE               OUT (FEh),A    11   site 2: high
/// 8023 10 FE        d2:    DJNZ d2
/// 8025 0D                  DEC C
/// 8026 20 FB               JR NZ,d2
/// 8028 C1                  POP BC         10
/// 8029 1B                  DEC DE         6
/// 802A 7A                  LD A,D         4
/// 802B B3                  OR E           4
/// 802C 20 E6               JR NZ,note     12
/// 802E C9                  RET
/// 802F                     records (B, C, cycles) and a 0
/// ```
pub fn toggler_saved_nested() -> Fixture {
    #[rustfmt::skip]
    let mut code = vec![
        0x21, 0x2F, 0x80, 0x7E, 0xB7, 0x28, 0xF9, 0x47, 0x23, 0x4E, 0x23, 0x5E, 0x23, 0x56, 0x23,
        0xCD, 0x14, 0x80, 0x18, 0xEF,
        // note
        0xC5, 0xAF, 0xD3, 0xFE, 0x10, 0xFE, 0x0D, 0x20, 0xFB, 0xC1, 0xC5, 0x3E, 0x10, 0xD3, 0xFE,
        0x10, 0xFE, 0x0D, 0x20, 0xFB, 0xC1, 0x1B, 0x7A, 0xB3, 0x20, 0xE6, 0xC9,
    ];
    assert_eq!(code.len(), 0x2F);
    for (b, c) in [(200u8, 1u8), (150, 1), (250, 1), (100, 2), (60, 2), (120, 1)] {
        code.extend_from_slice(&[b, c, 150, 0]);
    }
    code.push(0);
    Fixture {
        name: "toggler_saved_nested",
        sna: sna(&code, &[]),
        sites: vec![0x8016, 0x8021],
        kind: Kind::Square,
        truth: |_, r| {
            let count = |x: u16| f64::from(match x & 0xFF { 0 => 256, v => v });
            vec![13.0 * count(r.bc >> 8) + 3339.0 * count(r.bc) - 3282.5]
        },
    }
}

/// As [`square_one_counter`], but the pass reloads the delay through the table pointer: B from
/// (HL), HL at the note's record. Half period `61 + 13n` T. The table lies after the code
/// (8022h) or, `contended`, at 6000h: there the pass's read of (HL) waits for the screen.
///
/// ```text
/// 8000 21 22 80     start: LD HL,8022h    (6000h contended) the first record: n, K (16 bits)
/// 8003 7E           note:  LD A,(HL)      n (0: the end of the table)
/// 8004 B7                  OR A
/// 8005 28 F9               JR Z,start
/// 8007 23                  INC HL
/// 8008 5E                  LD E,(HL)
/// 8009 23                  INC HL
/// 800A 56                  LD D,(HL)      DE = K
/// 800B 2B                  DEC HL
/// 800C 2B                  DEC HL         HL at n
/// 800D 08           tog:   EX AF,AF'      4    A' = the level
/// 800E EE 10               XOR 10h        7
/// 8010 D3 FE               OUT (FEh),A    11   the edge
/// 8012 08                  EX AF,AF'      4
/// 8013 1B                  DEC DE         6
/// 8014 7A                  LD A,D         4
/// 8015 B3                  OR E           4
/// 8016 28 05               JR Z,next      7    the note's last edge
/// 8018 46                  LD B,(HL)      7    the pointer read
/// 8019 10 FE        d:     DJNZ d         13n - 5
/// 801B 18 F0               JR tog         12
/// 801D 23           next:  INC HL
/// 801E 23                  INC HL
/// 801F 23                  INC HL
/// 8020 18 E1               JR note
/// 8022                     records (n, K) and a 0 (or at 6000h)
/// ```
pub fn square_pointer_pass(contended: bool) -> Fixture {
    let table_at: u16 = if contended { 0x6000 } else { 0x8022 };
    let [lo, hi] = table_at.to_le_bytes();
    #[rustfmt::skip]
    let mut code = vec![
        0x21, lo, hi, 0x7E, 0xB7, 0x28, 0xF9, 0x23, 0x5E, 0x23, 0x56, 0x2B, 0x2B,
        0x08, 0xEE, 0x10, 0xD3, 0xFE, 0x08, 0x1B, 0x7A, 0xB3, 0x28, 0x05, 0x46, 0x10, 0xFE,
        0x18, 0xF0, 0x23, 0x23, 0x23, 0x18, 0xE1,
    ];
    assert_eq!(code.len(), 0x22);
    let mut table = Vec::new();
    for n in [250u8, 222, 198, 187, 166, 148, 131, 124] {
        table.extend_from_slice(&[n, 0x2C, 0x01]); // K = 300 edges
    }
    table.push(0);
    let sna = if contended {
        sna(&code, &[(table_at, &table)])
    } else {
        code.extend_from_slice(&table);
        sna(&code, &[])
    };
    Fixture {
        name: if contended { "square_pointer_pass_contended" } else { "square_pointer_pass" },
        sna,
        sites: vec![0x8010],
        kind: Kind::Square,
        truth: |peek, r| vec![13.0 * f64::from(peek(r.hl)) + 61.0],
    }
}

/// The note-change tune's notes: B for each of its eight notes, all low (fewer than 8 rises a
/// frame), so a frame a note change falls in has few pairs to outvote it.
pub const NOTE_CHANGE_B: [u8; 8] = [240, 200, 170, 140, 110, 90, 60, 30];

/// A square wave whose notes last 16 half periods each, about one and a half frames, so a note
/// begins anywhere in a frame: the loop counts its half periods at 8200h and takes note
/// `(count >> 4) & 7` from the table at 8300h. At each `OUT`, B holds the note it begins. Half
/// period `13B + 4106` T.
///
/// ```text
/// 8000 3A 00 82  l: LD A,(8200h)  13   the count
/// 8003 3C           INC A          4
/// 8004 32 00 82     LD (8200h),A  13
/// 8007 0F 0F 0F 0F  RRCA x4       16
/// 800B E6 07        AND 7          7
/// 800D 6F           LD L,A         4
/// 800E 26 83        LD H,83h       7
/// 8010 46           LD B,(HL)      7   the note
/// 8011 79           LD A,C         4   C: the level
/// 8012 EE 10        XOR 10h        7
/// 8014 4F           LD C,A         4
/// 8015 D3 FE        OUT (FEh),A   11   the site
/// 8017 10 FE    d1: DJNZ d1       13B - 5
/// 8019 16 FA        LD D,250       7
/// 801B 15       d2: DEC D          4
/// 801C 20 FD        JR NZ,d2      16 x 250 - 5
/// 801E 18 E0        JR l          12
/// ```
pub fn note_change_tune() -> Fixture {
    #[rustfmt::skip]
    let code = [
        0x3A, 0x00, 0x82, 0x3C, 0x32, 0x00, 0x82, 0x0F, 0x0F, 0x0F, 0x0F, 0xE6, 0x07, 0x6F,
        0x26, 0x83, 0x46, 0x79, 0xEE, 0x10, 0x4F, 0xD3, 0xFE, 0x10, 0xFE, 0x16, 0xFA, 0x15,
        0x20, 0xFD, 0x18, 0xE0,
    ];
    Fixture {
        name: "note_change_tune",
        sna: sna(&code, &[(0x8300, &NOTE_CHANGE_B)]),
        sites: vec![0x8015],
        kind: Kind::Square,
        truth: |_, r| vec![13.0 * f64::from(if r.bc >> 8 == 0 { 256 } else { r.bc >> 8 }) + 4106.0],
    }
}

/// Every fixture.
pub fn all() -> Vec<Fixture> {
    vec![
        square_tune(),
        square_one_counter(),
        toggler_two_sites(),
        toggler_ambiguous(),
        xor_unison(),
        phase_slots(),
        sweep(),
        siren(),
        computed_jump(),
        flips_uneven(),
        toggler_saved_nested(),
        square_pointer_pass(false),
        square_pointer_pass(true),
    ]
}

/// One `OUT` to an even port.
#[derive(Clone, Debug)]
pub struct OutEv {
    /// T-states since loading, less all contention so far: the time the code alone makes.
    pub t: u64,
    pub pc: u16,
    /// The speaker bit (bit 4) it wrote.
    pub level: bool,
    /// `probe` of the memory and registers at the `OUT`.
    pub probe: Vec<f64>,
}

/// Runs `sna` for `frames` frames and lists its `OUT`s to even ports, each with `probe` of the
/// memory (a shadow copy kept from the run's writes) and the registers at it. `probe` runs in
/// the core's trace hook and must not panic.
pub fn trace_outs(sna: &[u8], frames: u32, probe: impl Fn(&dyn Fn(u16) -> u8, &zx_regs) -> Vec<f64> + 'static) -> Vec<OutEv> {
    let mut m = machine(sna);
    let shadow: Vec<u8> = (0..=0xFFFFu16).map(|a| m.peek(a)).collect();
    let outs = Rc::new(RefCell::new(Vec::new()));
    let frame = Rc::new(Cell::new(0u32));
    let (hook_outs, hook_frame) = (Rc::clone(&outs), Rc::clone(&frame));
    let shadow = RefCell::new(shadow);
    m.set_trace(
        Some(Box::new(move |ev: &TraceEv| {
            let Ok(mut mem) = shadow.try_borrow_mut() else { return };
            if u32::from(ev.kind) == ZX_TR_WRITE {
                if ev.addr >= RAM {
                    if let Some(b) = mem.get_mut(ev.addr as usize) {
                        *b = ev.val;
                    }
                }
                return;
            }
            if ev.addr & 1 != 0 {
                return;
            }
            let at = u64::from(hook_frame.get()) * u64::from(ZX_FRAME_TSTATES) + u64::from(ev.t);
            let peek = |a: u16| mem.get(a as usize).copied().unwrap_or(0);
            let p = probe(&peek, &ev.regs);
            if let Ok(mut o) = hook_outs.try_borrow_mut() {
                o.push(OutEv { t: at.wrapping_sub(u64::from(ev.cont)), pc: ev.pc, level: ev.val & 0x10 != 0, probe: p });
            }
        })),
        (1 << ZX_TR_OUT) | (1 << ZX_TR_WRITE),
    );
    for f in 0..frames {
        frame.set(f);
        m.run_frame();
    }
    m.set_trace(None, 0);
    let v = outs.borrow().clone();
    v
}
