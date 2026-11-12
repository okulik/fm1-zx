// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Our two test programs, hand-assembled: `scroll.sna` changes every screen line every frame
//! (the LCD update path's worst case) and `border.sna` draws border stripes and a beeper tone.
//! `fmzx-pack testprogs <dir>` writes them; `games/test/` holds the committed copies.
//!
//! # Common layout
//!
//! Both run in uncontended RAM under IM 2 with their own vector table, so no ROM code or
//! system variable is involved and every frame takes the same time:
//!
//! | Address         | Contents                                                    |
//! |-----------------|-------------------------------------------------------------|
//! | `8000h`         | main loop (listings below)                                  |
//! | `FCFEh`-`FCFFh` | the stack (SP `FD00h`); the `.sna` keeps PC there           |
//! | `FDFDh`         | interrupt handler: `RET` (`C9`, 10 T)                       |
//! | `FE00h`-`FF00h` | 257 bytes of `FDh`: whatever the bus holds, the vector is `FDFDh` |
//!
//! I = `FEh`, IM 2, interrupts off at load. The main loop ends with `EI; HALT`; the interrupt
//! at T 0 costs 19 T (IM 2 response) plus the handler's `RET`, which returns to the
//! instruction after the `HALT` (address [`RESUME`]) with interrupts off again. So the work
//! starts at about T 32 of every frame and can never be interrupted.
//!
//! # `scroll.sna`
//!
//! A 32-pixel band (columns 14-17) on each of the 192 lines holds an 8-pixel stripe whose
//! position depends on the line (a diagonal). Every frame the band of every line rotates one
//! pixel right, so every line changes every frame. The border stays 1 (blue). Lines are visited
//! in memory order: bitmap rows are 32 bytes apart, and each 32-byte row is one screen line.
//!
//! ```text
//! 8000 FB        frame: EI                4
//! 8001 76               HALT              4n   IM 2 + RET return to 8002, interrupts off
//! 8002 21 11 40         LD HL,4011h       10   row 0, column 17 (band = columns 14-17)
//! 8005 11 20 00         LD DE,0020h       10   one bitmap row
//! 8008 06 C0            LD B,192          7
//! 800A 7E        line:  LD A,(HL)         7    column 17
//! 800B 1F               RRA               4    CF = its bit 0, wraps into column 14
//! 800C 2D               DEC L             4    DEC and INC keep CF
//! 800D 2D               DEC L             4
//! 800E 2D               DEC L             4    column 14
//! 800F CB 1E            RR (HL)           15
//! 8011 2C               INC L             4
//! 8012 CB 1E            RR (HL)           15
//! 8014 2C               INC L             4
//! 8015 CB 1E            RR (HL)           15
//! 8017 2C               INC L             4
//! 8018 CB 1E            RR (HL)           15   column 17
//! 801A 19               ADD HL,DE         11   next row
//! 801B 10 ED            DJNZ line         13/8
//! 801D 18 E1            JR frame          12
//! ```
//!
//! A line takes 119 T plus contention (9 screen accesses, at most 6 T each); the work ends at
//! T 23,919 (the test `scroll_fits_well_inside_a_frame` measures it).
//!
//! # `border.sna`
//!
//! The paper shows eight static colour bars (colours 0-7, left to right).
//!
//! One `OUT (FEh)` per 224-T line, 312 per frame: the border colour cycles 0-7 line by line
//! and the beeper (EAR, bit 4) flips every 8 lines, a square wave of
//! 3,500,000 / (2 x 8 x 224) = 976.5625 Hz. The 312 lines fill the frame exactly, so the
//! last line's `OUT` is followed directly by `EI; HALT` and the interrupt's timing takes the
//! place of that line's delay loop: the tone runs on across frames without a seam.
//!
//! `OUT (FEh)` is I/O-contended on paper lines unless its I/O cycle falls in the 96 T of
//! border and retrace at the end of each line. The first `OUT` starts at about T 146 of the
//! frame (the pre-delay `D` = 4 places it), and with exactly 224 T per line every `OUT`
//! starts at T 146 + 224k, whose I/O cycle (T 153 + 224k) is never contended. So the loop
//! keeps exact time, and the border changes 153 T into each line. The last `OUT` starts at
//! about T 69,810 and the `HALT` at about T 69,858, before the next interrupt.
//!
//! The output byte lives in E: colour in bits 0-2, EAR in bit 4, MIC (bit 3) 0. Setting bit 3
//! before `INC A` makes the carry out of the colour bits flip bit 4; `AND 17h` then clears
//! bit 3 and bits 5-7 (so the port's high byte stays below 40h, out of the contended range).
//!
//! ```text
//! 8000 FB        frame: EI                4
//! 8001 76               HALT              4n   IM 2 + RET return to 8002, interrupts off
//! 8002 01 38 01         LD BC,312         10   lines this frame
//! 8005 16 04            LD D,4            7    pre-delay: 7 + 4 x 16 - 5 = 66 T
//! 8007 15        pre:   DEC D             4
//! 8008 20 FD            JR NZ,pre         12/7
//! 800A 18 07            JR out            12   the first line skips its delay
//! 800C 16 09     line:  LD D,9            7    delay: 7 + 9 x 16 - 5 = 146 T
//! 800E 15        wait:  DEC D             4
//! 800F 20 FD            JR NZ,wait        12/7
//! 8011 00               NOP               4
//! 8012 29               ADD HL,HL         11   padding (HL unused)
//! 8013 7B        out:   LD A,E            4
//! 8014 F6 08            OR 08h            7
//! 8016 3C               INC A             4    colour + 1; every 8th time EAR flips
//! 8017 E6 17            AND 17h           7
//! 8019 5F               LD E,A            4
//! 801A D3 FE            OUT (FEh),A       11   I/O cycle at T +7
//! 801C 0B               DEC BC            6
//! 801D 78               LD A,B            4
//! 801E B1               OR C              4
//! 801F 20 EB            JR NZ,line        12/7 line total: 161 + 63 = 224 T
//! 8021 18 DD            JR frame          12
//! ```
//!
//! Measured on the host core (tests in `tests/testprogs.rs`): [`BORDER_EDGES_PER_FRAME`] beeper
//! edges in every frame, 1,792 T apart; the `HALT` reached at T 69,858 (scroll: T 23,919);
//! and a tone of 976.06 Hz ([`BORDER_TONE_HZ`]) from the zero crossings of one second
//! (50 frames) of `zx_beeper` output at 750000/17 Hz. The device's tone test compares against
//! that value (±1%).

use std::path::{Path, PathBuf};

/// Beeper edges per frame of `border.sna`: 312 lines / 8.
pub const BORDER_EDGES_PER_FRAME: usize = 39;
/// Tone of `border.sna` measured from the host beeper renderer: 1,949 zero crossings in the
/// 50 frames (0.9984 s) after frame 10 (the design value is 976.5625 Hz).
pub const BORDER_TONE_HZ: f64 = 976.06;
/// Address of the `HALT` in both programs.
pub const HALT: u16 = 0x8001;
/// Where the interrupt handler returns to in both programs.
pub const RESUME: u16 = 0x8002;
/// Builds one program's `.sna`.
pub type Generator = fn() -> Vec<u8>;
/// The programs `fmzx-pack testprogs` writes: file name and generator.
pub const PROGRAMS: [(&str, Generator); 2] = [("scroll.sna", scroll_sna), ("border.sna", border_sna)];

const CODE: u16 = 0x8000;
const STACK: u16 = 0xFD00;
const HANDLER: u16 = 0xFDFD;
const VECTORS: u16 = 0xFE00;
const RAM: u16 = 0x4000;

#[rustfmt::skip]
const SCROLL_CODE: [u8; 31] = [
    0xFB,             // frame: EI
    0x76,             //        HALT
    0x21, 0x11, 0x40, //        LD HL,4011h
    0x11, 0x20, 0x00, //        LD DE,0020h
    0x06, 0xC0,       //        LD B,192
    0x7E,             // line:  LD A,(HL)
    0x1F,             //        RRA
    0x2D,             //        DEC L
    0x2D,             //        DEC L
    0x2D,             //        DEC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x19,             //        ADD HL,DE
    0x10, 0xED,       //        DJNZ line
    0x18, 0xE1,       //        JR frame
];

#[rustfmt::skip]
const BORDER_CODE: [u8; 35] = [
    0xFB,             // frame: EI
    0x76,             //        HALT
    0x01, 0x38, 0x01, //        LD BC,312
    0x16, 0x04,       //        LD D,4
    0x15,             // pre:   DEC D
    0x20, 0xFD,       //        JR NZ,pre
    0x18, 0x07,       //        JR out
    0x16, 0x09,       // line:  LD D,9
    0x15,             // wait:  DEC D
    0x20, 0xFD,       //        JR NZ,wait
    0x00,             //        NOP
    0x29,             //        ADD HL,HL
    0x7B,             // out:   LD A,E
    0xF6, 0x08,       //        OR 08h
    0x3C,             //        INC A
    0xE6, 0x17,       //        AND 17h
    0x5F,             //        LD E,A
    0xD3, 0xFE,       //        OUT (FEh),A
    0x0B,             //        DEC BC
    0x78,             //        LD A,B
    0xB1,             //        OR C
    0x20, 0xEB,       //        JR NZ,line
    0x18, 0xDD,       //        JR frame
];

/// 48K of RAM (from 4000h) with `code` at 8000h, the IM 2 handler and vector table, and a
/// screen whose attributes are all `attr`.
fn ram_with(code: &[u8], attr: u8) -> Vec<u8> {
    let mut ram = vec![0u8; 0xC000];
    let at = |a: u16| (a - RAM) as usize;
    ram[at(CODE)..at(CODE) + code.len()].copy_from_slice(code);
    ram[at(HANDLER)] = 0xC9; // RET
    ram[at(VECTORS)..=at(VECTORS) + 256].fill((HANDLER >> 8) as u8);
    ram[0x1800..0x1B00].fill(attr);
    ram
}

/// Offset of screen line `y`'s byte `col` in the bitmap.
fn bitmap_offset(y: usize, col: usize) -> usize {
    ((y & 0xC0) << 5) | ((y & 7) << 8) | ((y & 0x38) << 2) | col
}

/// A 48K `.sna` that starts at 8000h with SP FD00h, I FEh, IM 2, interrupts off, every other
/// register 0, and the given border.
fn sna(mut ram: Vec<u8>, border: u8) -> Vec<u8> {
    let sp = STACK - 2; // PC is popped from the stack
    let s = (sp - RAM) as usize;
    ram[s..s + 2].copy_from_slice(&CODE.to_le_bytes());
    let mut h = vec![0xFEu8]; // I
    h.extend_from_slice(&[0; 18]); // HL' DE' BC' AF' HL DE BC IY IX
    h.push(0); // IFF2 (bit 2) clear
    h.push(0); // R
    h.extend_from_slice(&[0, 0]); // AF
    h.extend_from_slice(&sp.to_le_bytes());
    h.push(2); // IM
    h.push(border);
    debug_assert_eq!(h.len(), 27);
    h.extend_from_slice(&ram);
    h
}

/// The scroller: border 1, white paper, a yellow-on-blue band whose stripe on line `y`
/// starts at pixel `y % 32` of the band.
pub fn scroll_sna() -> Vec<u8> {
    let mut ram = ram_with(&SCROLL_CODE, 0x38);
    for y in 0..192 {
        let band = 0xFF00_0000u32.rotate_right(y as u32 % 32).to_be_bytes();
        for (i, b) in band.iter().enumerate() {
            ram[bitmap_offset(y, 14 + i)] = *b;
        }
    }
    for row in 0..24 {
        ram[0x1800 + row * 32 + 14..0x1800 + row * 32 + 18].fill(0x0E); // ink 6, paper 1
    }
    sna(ram, 1)
}

/// The border stripes and 976.5625 Hz tone; the paper shows eight static bars of the colours
/// 0-7, 4 columns each, for comparison with the border.
pub fn border_sna() -> Vec<u8> {
    let mut ram = ram_with(&BORDER_CODE, 0x38);
    for (i, a) in ram[0x1800..0x1B00].iter_mut().enumerate() {
        *a = ((i % 32 / 4) as u8) << 3; // paper = colour, ink 0
    }
    sna(ram, 0)
}

/// Writes every program into `dir` (created if needed); returns the paths written.
pub fn write_all(dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    PROGRAMS
        .iter()
        .map(|(name, make)| {
            let path = dir.join(name);
            std::fs::write(&path, make()).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        })
        .collect()
}
