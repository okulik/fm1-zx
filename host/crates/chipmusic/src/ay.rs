// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! An `.ay` song played in the Spectrum machine core (`zx-sys`), and the capture of its AY
//! registers.
//!
//! # Memory and start
//!
//! As the ZXAYEMUL format lays it out: $0000-$00FF `RET` ($C9) with `EI` ($FB) at $0038 (an
//! IM 1 interrupt returns enabled), $0100-$3FFF $FF, $4000-$FFFF $00, the player at $0000, then
//! the song's blocks. With an interrupt address the player is `DI; CALL init; loop: IM 1; EI;
//! HALT; CALL interrupt; JR loop`, without one `DI; CALL init; loop: IM 2; EI; HALT; JR loop`
//! (the init plays by itself, or installs an IM 2 handler of its own). An init address of 0 is
//! the first block's address. The registers start as `HiReg`/`LoReg` give them (A, B, D, H and
//! the alternate set's and IX's and IY's high bytes `HiReg`, F, C, E, L and the low bytes
//! `LoReg`), I = 3, SP the song's stack, PC 0, interrupts off in mode 0; the AY's registers 0.
//!
//! The machine is a 48K's: $0000-$3FFF is ROM (the image above), so the song's writes there are
//! lost, as they are on a real Spectrum (a player ripped from a Spectrum program never needs
//! them); the frame is the 48K's 69,888 T at 3.5 MHz with its interrupt at T 0 (a 128K's is
//! 70,908 T, 50.02 Hz against 50.08: a tune plays 0.1% fast); reads of the AY's ports read the
//! floating bus, not the AY; an IM 2 interrupt reads $FF from the bus.
//!
//! # Capture
//!
//! `OUT`s are decoded as a 128K decodes them: port & $C002 = $C000 selects a register (the value
//! & 15), port & $C002 = $8000 writes the selected one (registers 0-13; the I/O ports 14 and 15
//! are ignored); values are kept to their registers' bits. A row is the registers at the end of
//! each frame, timed at the frame's interrupt; rows start at the frame of the song's first
//! register write. The song plays until `seconds` of rows are captured, it halts with
//! interrupts off, or it writes nothing to the AY for 10 s (its end, or a runaway: the rows
//! end a second after its last write); a song that writes nothing to the AY within 30 s cannot be played
//! (the reason says so, or, by its `OUT`s, that it is a beeper tune (port $FE), an Amstrad CPC
//! song (ports $F4xx and $F6xx) or a SAM Coupe song (ports $00FF and $01FF)).

use crate::ayfile::AyFile;
use std::cell::RefCell;
use std::rc::Rc;
use zx_sys::{zx_regs, Machine, TraceEv, ZX_FRAME_TSTATES, ZX_ROM_SIZE, ZX_TR_OUT};

/// The AY registers captured: tone periods, noise period, mixer, levels, envelope.
pub const AY_REGS: usize = 14;
/// The 128K's AY clock in Hz.
pub const AY_HZ: u32 = 1_773_400;
/// The bits each register keeps.
const MASK: [u8; AY_REGS] = [0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0x1F, 0xFF, 0x1F, 0x1F, 0x1F, 0xFF, 0xFF, 0x0F];
/// How long a song may take to write to the AY (seconds).
const WAIT_S: f64 = 30.0;
/// A song that stops writing to the AY for this long has ended (or run away) (seconds).
const IDLE_S: f64 = 10.0;

/// The AY's state at the end of a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AyRow {
    /// T-states (3.5 MHz) since the first row.
    pub t: u64,
    pub regs: [u8; AY_REGS],
    /// Register 13 was written during the frame (which restarts the envelope, even with its
    /// value unchanged).
    pub env_restart: bool,
    /// Bit c: channel c's level (register 8 + c), above 0 at the frame's start, went to 0 and
    /// up again during the frame (a retrigger the frame's end does not show).
    pub rise: u8,
}

/// A song's AY registers over time.
#[derive(Clone, Debug)]
pub struct AyCapture {
    pub rows: Vec<AyRow>,
    /// T-states since the first row at which the capture ends.
    pub end: u64,
}

/// The 64 KB the song starts with (see the module docs). `song` is 1-based and valid.
pub fn memory(file: &AyFile, song: u16) -> Vec<u8> {
    let s = &file.songs[usize::from(song) - 1];
    let mut mem = vec![0u8; 0x1_0000];
    mem[..0x100].fill(0xC9);
    mem[0x100..0x4000].fill(0xFF);
    mem[0x38] = 0xFB;
    let init = if s.init == 0 { s.blocks.first().map_or(0, |b| b.addr) } else { s.init };
    let [il, ih] = init.to_le_bytes();
    let player: Vec<u8> = if s.interrupt == 0 {
        vec![0xF3, 0xCD, il, ih, 0xED, 0x5E, 0xFB, 0x76, 0x18, 0xFA]
    } else {
        let [jl, jh] = s.interrupt.to_le_bytes();
        vec![0xF3, 0xCD, il, ih, 0xED, 0x56, 0xFB, 0x76, 0xCD, jl, jh, 0x18, 0xF7]
    };
    mem[..player.len()].copy_from_slice(&player);
    for b in &s.blocks {
        let a = usize::from(b.addr);
        let n = b.data.len().min(0x1_0000 - a);
        mem[a..a + n].copy_from_slice(&b.data[..n]);
    }
    mem
}

/// The registers the song starts with (see the module docs).
fn start_regs(file: &AyFile, song: u16) -> zx_regs {
    let s = &file.songs[usize::from(song) - 1];
    let w = u16::from(s.hi_reg) << 8 | u16::from(s.lo_reg);
    zx_regs {
        af: w,
        bc: w,
        de: w,
        hl: w,
        af_: w,
        bc_: w,
        de_: w,
        hl_: w,
        ix: w,
        iy: w,
        sp: s.stack,
        pc: 0,
        i: 3,
        r: 0,
        iff1: 0,
        iff2: 0,
        im: 0,
    }
}

/// Plays `song` (1-based) of `file` for `seconds` and captures the AY registers once per frame
/// (see the module docs). Err: why it cannot be played.
pub fn capture(file: &AyFile, song: u16, seconds: f64) -> Result<AyCapture, String> {
    if let Some(r) = file.refusal(song) {
        return Err(r);
    }
    let mem = memory(file, song);
    let mut m = Machine::with_rom(&mem[..ZX_ROM_SIZE as usize]);
    for (a, &v) in mem.iter().enumerate().skip(ZX_ROM_SIZE as usize) {
        m.poke(a as u16, v);
    }
    m.set_regs(&start_regs(file, song));
    let outs: Rc<RefCell<Vec<(u16, u8)>>> = Rc::default();
    let log = Rc::clone(&outs);
    m.set_trace(
        Some(Box::new(move |ev: &TraceEv| {
            if ev.kind == ZX_TR_OUT as u8 {
                log.borrow_mut().push((ev.addr, ev.val));
            }
        })),
        1 << ZX_TR_OUT,
    );
    let frame = u64::from(ZX_FRAME_TSTATES);
    let hz = 3_500_000.0 / frame as f64;
    let want = (seconds.max(0.0) * hz).ceil() as usize;
    let wait = (WAIT_S * hz) as u64;
    let idle = (IDLE_S * hz) as usize;
    let mut last_write = 0usize;
    let mut regs = [0u8; AY_REGS];
    let mut sel = 0usize;
    let mut rows: Vec<AyRow> = Vec::new();
    let (mut writes, mut cpc, mut sam, mut beeper) = (0u64, 0u64, 0u64, 0u64);
    let mut frames = 0u64;
    let mut halted = false;
    while rows.len() < want {
        m.run_frame();
        frames += 1;
        let (mut env_restart, mut rise) = (false, 0u8);
        let at_start = regs;
        let before = writes;
        for (port, val) in outs.borrow_mut().drain(..) {
            match port & 0xC002 {
                0xC000 => sel = usize::from(val & 0x0F),
                0x8000 if sel < AY_REGS => {
                    let v = val & MASK[sel];
                    let old = regs[sel];
                    match sel {
                        8..=10 if old == 0 && v != 0 && at_start[sel] != 0 => rise |= 1 << (sel - 8),
                        13 => env_restart = true,
                        _ => {}
                    }
                    regs[sel] = v;
                    writes += 1;
                }
                _ => {}
            }
            if matches!(port >> 8, 0xF4 | 0xF6) {
                cpc += 1;
            }
            if matches!(port, 0x00FF | 0x01FF) {
                sam += 1;
            }
            if port & 1 == 0 {
                beeper += 1;
            }
        }
        if writes > before {
            last_write = rows.len();
        }
        if writes > 0 {
            rows.push(AyRow { t: rows.len() as u64 * frame, regs, env_restart, rise });
            if rows.len() - last_write > idle {
                rows.truncate(last_write + 1 + hz as usize);
                break;
            }
        } else if frames > wait {
            break;
        }
        if m.halted() && m.regs().iff1 == 0 {
            halted = true;
            break;
        }
    }
    if rows.is_empty() {
        if halted {
            return Err("the song halted with interrupts off before writing to the AY".into());
        }
        if cpc > 0 {
            return Err("an Amstrad CPC song (it writes to the CPC's ports, not a Spectrum 128's AY)".into());
        }
        if sam > 0 {
            return Err("a SAM Coupe song (it writes to the SAA 1099's ports, not a Spectrum 128's AY)".into());
        }
        if beeper > 0 {
            return Err(format!("a beeper tune: in {WAIT_S:.0} s it plays the speaker ({beeper} OUTs to port $FE), not the AY"));
        }
        return Err(format!("the song wrote nothing to the AY (nor the speaker) in {WAIT_S:.0} s"));
    }
    let end = rows.len() as u64 * frame;
    Ok(AyCapture { rows, end })
}
