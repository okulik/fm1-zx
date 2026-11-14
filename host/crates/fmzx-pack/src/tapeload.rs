// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Games in, ready for the pack: snapshots as they are, standard tapes as tapes once a trial
//! load through the ROM trap succeeds, and every other tape loaded on the host through the tape
//! signal and stored as a 48K `.z80`.
//!
//! # Load to snapshot ([`load_signal`])
//!
//! The tape is inserted as a signal, and the machine autostarts (`LOAD ""`) with the trap off
//! (no `.tap` inserted). Frames run until the player reaches the end of the tape. A stop (TZX
//! 0x20 pause 0) with the tape not yet at its end is played on after [`RESUME_FRAMES`], as a
//! person presses play again. The tape is then taken out, so it no longer drives port FEh, and
//! the machine runs a settle time of [`SETTLE_FRAMES`]; the capture is the machine at the end of
//! that frame. A tape still playing after [`LIMIT_FRAMES`] (15 minutes) is a failure.
//!
//! # The trap trial ([`trap_trial`])
//!
//! A standard tape is inserted as a `.tap` and autostarted with the LD-BYTES trap on. It
//! succeeds when every block has been taken by LD-BYTES and the settle time passes the success
//! test. It fails early when, for [`TRAP_IDLE_FRAMES`] frames running, no block is taken and
//! the program polls port FEh as a loader does (at least [`POLL_READS`] reads a frame): the
//! program reads the tape without LD-BYTES, which the trap cannot serve.
//!
//! # The success test
//!
//! Over the settle time the machine is sampled at each frame end (100 samples). It fails:
//!
//! - when it is halted with interrupts off (in any sample: nothing wakes it);
//! - "still loading" (a heuristic), when in at least half of the samples the frame polled port
//!   FEh at least [`POLL_READS`] times with PC within 256 bytes of where it was while the tape
//!   gave edges (the last 50 frame ends inside a pulse) or in the ROM's LD-BYTES (0556h-0604h):
//!   the loader is still waiting for the tape. A title loop that polls port FEh as hard near
//!   those places would be taken for a loader;
//! - at the copyright screen (a reset), when in at least half of the samples PC is in the ROM,
//!   the BASIC program is empty (PROG, 5C53h, equals VARS, 5C4Bh) and the system variable ERR_NR
//!   (5C3Ah) is 0 or FFh (the copyright screen leaves 0);
//! - at the ROM's error report, when in at least half of the samples PC is in the ROM and ERR_NR
//!   is not FFh otherwise (the ROM's error restart sets it to the report code less one; FFh is
//!   "OK"), named by that report;
//! - at the BASIC editor (the program ended with "0 OK"), when in at least half of the samples
//!   PC is in the ROM's key wait (WAIT-KEY's loop through INPUT-AD, CALL-SUB and CALL-JUMP into
//!   KEY-INPUT: 10A8h-10B4h, 15DEh-15FFh, 162Ch) and the system variable FLAGX (5C71h) has bit 5
//!   clear (with it set, the editor is serving a BASIC `INPUT`: the program is still running).
//!
//! A program that runs in the ROM passes: a BASIC part playing `BEEP`, a menu printed through
//! `RST 10h`, a `PAUSE 0`. (The design asked for code in RAM in half of the samples; that refuses
//! such games, so the test names the one ROM state that means the program has ended.)
//!
//! # The snapshot ([`save_z80`])
//!
//! A `.z80` version 3 (54-byte extended header), 48K, with pages 8, 4 and 5 compressed with the
//! format's `ED ED nn bb` run-length coding, or stored when that is no shorter. The core's loads
//! start at frame T-state 0, where the frame interrupt is raised: so the capture is at a frame
//! end, and the T-state counter is written as frame T-state 0. A CPU halted with interrupts on
//! is saved with PC after the `HALT` (the interrupt at T 0 then returns there, as on the
//! running machine); halted with interrupts off, with PC at the `HALT`.

use std::collections::VecDeque;
use std::path::Path;

use crate::tzx;
use zx_sys::Machine;

/// Frames of the settle time after the tape: 2 s.
pub const SETTLE_FRAMES: u32 = 100;
/// The longest a load may run: 15 minutes of emulated time.
pub const LIMIT_FRAMES: u32 = 45_000;
/// Frames a stop holds the tape before play goes on: 1 s.
pub const RESUME_FRAMES: u32 = 50;
/// Port FEh reads in a frame from which the program counts as polling the tape. A loader samples
/// the EAR bit over a thousand times a frame; a keyboard scan reads the port 8 times or so.
pub const POLL_READS: u32 = 200;
/// Frames of polling with no block taken after which the trap trial gives up: 5 s.
pub const TRAP_IDLE_FRAMES: u32 = 250;
/// The largest file taken in.
pub const MAX_FILE: usize = 4 << 20;

/// The frame ends of the loader's last frames that the "still loading" test compares with.
const LOADER_PCS: usize = 50;
/// The ROM's LD-BYTES.
const ROM_LD_BYTES: std::ops::RangeInclusive<u16> = 0x0556..=0x0604;
/// The ROM's key wait at the editor (see the module docs).
const ROM_KEY_WAIT: [std::ops::RangeInclusive<u16>; 3] = [0x10A8..=0x10B4, 0x15DE..=0x15FF, 0x162C..=0x162C];
const ERR_NR: u16 = 0x5C3A;
const FLAGX: u16 = 0x5C71;
const VARS: u16 = 0x5C4B;
const PROG: u16 = 0x5C53;

/// How a game goes into the pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A 48K `.sna` or `.z80`.
    Snapshot,
    /// A `.tap`, loaded on the device through the ROM trap.
    Tape,
}

/// A game ready for the pack.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub kind: Kind,
    /// The file's bytes: the snapshot (as given, or the `.z80` of the loaded machine), or the
    /// `.tap` (as given, or converted from a standard `.tzx`).
    pub data: Vec<u8>,
    /// Emulated frames the trial or the load ran, settle time included (0 for a snapshot taken
    /// as it is).
    pub frames: u32,
    /// How it went in: "as is", "tape", "loaded (own loader)" or "loaded".
    pub how: &'static str,
}

fn le16(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

/// Whether a file is for a 128K machine: a `.sna` of a 128K length, a `.z80` version 2 or 3
/// whose hardware mode is a 128K machine, or a `.tzx` whose hardware type block says it does not
/// run on a 48K (or that it uses a 128K's hardware and names no 48K it runs on).
pub fn is_128k(data: &[u8], ext: &str) -> bool {
    match ext.to_ascii_lowercase().as_str() {
        "sna" => data.len() == 131_103 || data.len() == 147_487,
        "z80" => {
            // version 1 (PC in the header not 0) is 48K only
            if data.len() < 35 || le16(data, 6) != 0 {
                return false;
            }
            let hw = data[34];
            match le16(data, 30) {
                23 => matches!(hw, 3 | 4 | 7..=10 | 12 | 13),
                54 | 55 => matches!(hw, 4..=10 | 12 | 13),
                _ => false,
            }
        }
        "tzx" => {
            let computers: Vec<(u8, u8)> =
                tzx::hardware(data).into_iter().filter(|e| e.0 == 0).map(|e| (e.1, e.2)).collect();
            let is_48 = |id: u8| id == 1 || id == 2;
            let runs_48 = computers.iter().any(|&(id, info)| is_48(id) && info != 3);
            let refused_48 = computers.iter().any(|&(id, info)| is_48(id) && info == 3);
            let uses_128 = computers.iter().any(|&(id, info)| matches!(id, 3 | 4 | 5 | 8 | 0x0E) && info == 1);
            !runs_48 && (refused_48 || uses_128)
        }
        _ => false,
    }
}

/// The ROM's report for an ERR_NR value: its code (0-9, A-R) and message.
fn report_name(err_nr: u8) -> String {
    const NAMES: [&str; 28] = [
        "OK",
        "NEXT without FOR",
        "Variable not found",
        "Subscript wrong",
        "Out of memory",
        "Out of screen",
        "Number too big",
        "RETURN without GOSUB",
        "End of file",
        "STOP statement",
        "Invalid argument",
        "Integer out of range",
        "Nonsense in BASIC",
        "BREAK - CONT repeats",
        "Out of DATA",
        "Invalid file name",
        "No room for line",
        "STOP in INPUT",
        "FOR without NEXT",
        "Invalid I/O device",
        "Invalid colour",
        "BREAK into program",
        "RAMTOP no good",
        "Statement lost",
        "Invalid stream",
        "FN without DEF",
        "Parameter error",
        "Tape loading error",
    ];
    let k = err_nr.wrapping_add(1) as usize;
    match NAMES.get(k) {
        Some(name) => {
            let code = if k < 10 { (b'0' + k as u8) as char } else { (b'A' + (k - 10) as u8) as char };
            format!("\"{code} {name}\"")
        }
        None => format!("with code {err_nr:02X}h"),
    }
}

/// Runs the settle time and applies the success test (see the module docs). `loader`: where PC
/// was while the tape gave edges.
fn settle(m: &mut Machine, loader: &[u16]) -> Result<(), String> {
    let half = SETTLE_FRAMES / 2;
    let near_loader = |pc: u16| ROM_LD_BYTES.contains(&pc) || loader.iter().any(|&p| pc.abs_diff(p) < 0x100);
    let peek16 = |m: &Machine, a: u16| u16::from_le_bytes([m.peek(a), m.peek(a + 1)]);
    m.port_fe_probe(true);
    m.take_port_fe_reads();
    let (mut editor, mut polling, mut report, mut reset) = (0u32, 0u32, 0u32, 0u32);
    let mut halted_di = false;
    let mut err_nr = 0xFF;
    for _ in 0..SETTLE_FRAMES {
        m.run_frame();
        let reads = m.take_port_fe_reads();
        let r = m.regs();
        halted_di |= m.halted() && r.iff1 == 0;
        if ROM_KEY_WAIT.iter().any(|k| k.contains(&r.pc)) && m.peek(FLAGX) & 0x20 == 0 {
            editor += 1;
        }
        if reads >= POLL_READS && near_loader(r.pc) {
            polling += 1;
        }
        if r.pc < 0x4000 {
            // the copyright screen leaves ERR_NR 0 (or FFh); a report with no program (a failed
            // LOAD "") has another code
            let e = m.peek(ERR_NR);
            if peek16(m, PROG) == peek16(m, VARS) && (e == 0 || e == 0xFF) {
                reset += 1;
            } else if e != 0xFF {
                report += 1;
                err_nr = e;
            }
        }
    }
    m.port_fe_probe(false);
    if halted_di {
        Err("halted with interrupts off".into())
    } else if polling >= half {
        Err(format!(
            "still loading: after the tape, {polling} of {SETTLE_FRAMES} frames read port FEh {POLL_READS} times or more \
             near where the loader ran (the still-loading heuristic)"
        ))
    } else if report >= half {
        Err(format!("stopped at the ROM's error report {}", report_name(err_nr)))
    } else if reset >= half {
        Err("back at the copyright screen (the machine reset)".into())
    } else if editor >= half {
        Err("the program ended: the machine waits at the BASIC editor".into())
    } else {
        Ok(())
    }
}

/// The trap trial; the frames it ran.
fn trap_run(tap: &[u8]) -> Result<u32, String> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i < tap.len() {
        if tap.len() - i < 2 {
            return Err("bad .tap: a block runs past the end of the file".into());
        }
        starts.push(i);
        i += 2 + le16(tap, i) as usize;
    }
    if starts.is_empty() || i != tap.len() {
        return Err(if starts.is_empty() { "empty tape" } else { "bad .tap: a block runs past the end of the file" }.into());
    }
    let mut m = Machine::new();
    m.tape_insert(tap).map_err(|_| "bad .tap: a block runs past the end of the file".to_string())?;
    m.tape_autostart();
    m.port_fe_probe(true);
    let (mut frames, mut idle, mut pos) = (0u32, 0u32, 0u32);
    while m.tape_pos() as usize != tap.len() {
        let taken = starts.iter().filter(|&&s| s < m.tape_pos() as usize).count();
        if frames == LIMIT_FRAMES {
            return Err(format!("LD-BYTES took {taken} of the {} blocks in 15 minutes", starts.len()));
        }
        if idle == TRAP_IDLE_FRAMES {
            return Err(format!(
                "LD-BYTES took {taken} of the {} blocks; the program reads the rest itself",
                starts.len()
            ));
        }
        m.run_frame();
        frames += 1;
        let reads = m.take_port_fe_reads();
        if m.tape_pos() != pos {
            pos = m.tape_pos();
            idle = 0;
        } else if reads >= POLL_READS {
            idle += 1;
        } else {
            idle = 0;
        }
    }
    settle(&mut m, &[])?;
    Ok(frames + SETTLE_FRAMES)
}

/// Loads a standard tape (`.tap`) through the ROM trap, as the device will, and applies the
/// success test (see the module docs).
pub fn trap_trial(tap: &[u8]) -> Result<(), String> {
    trap_run(tap).map(|_| ())
}

/// Loads a `.tzx` or `.tap` through the tape signal to the point after the tape and the settle
/// time, and applies the success test (see the module docs).
pub fn load_signal(data: &[u8]) -> Result<Machine, String> {
    if data.len() > MAX_FILE {
        return Err("file too big for a tape".into());
    }
    let mut m = Machine::new();
    m.tape_signal_insert(data)?;
    m.tape_autostart();
    let mut loader = VecDeque::with_capacity(LOADER_PCS);
    let (mut frames, mut held) = (0u32, 0u32);
    while !m.tape_signal_done() {
        if frames == LIMIT_FRAMES {
            return Err("still loading after 15 minutes: the tape never finished".into());
        }
        m.run_frame();
        frames += 1;
        if m.tape_signal_in_pulse() {
            if loader.len() == LOADER_PCS {
                loader.pop_front();
            }
            loader.push_back(m.regs().pc);
        }
        if m.tape_signal_stopped() {
            held += 1;
            if held >= RESUME_FRAMES {
                m.tape_signal_play();
                held = 0;
            }
        } else {
            held = 0;
        }
    }
    // done: take the tape out, so it no longer drives port FEh bit 6
    m.tape_signal_eject();
    settle(&mut m, loader.make_contiguous())?;
    Ok(m)
}

/// `ED ED nn bb` run-length coding of a `.z80` page: runs of 5 or more equal bytes, and of 2 or
/// more EDh, become blocks of at most 255; the byte after a lone EDh is never the start of one.
fn z80_compress(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() {
        let b = d[i];
        let run = d[i..].iter().take(255).take_while(|&&x| x == b).count();
        if run >= 5 || (b == 0xED && run >= 2) {
            out.extend_from_slice(&[0xED, 0xED, run as u8, b]);
            i += run;
        } else if b == 0xED {
            out.push(0xED);
            i += 1;
            if i < d.len() {
                out.push(d[i]);
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// The machine as a 48K `.z80` version 3 (see the module docs).
pub fn save_z80(m: &Machine) -> Vec<u8> {
    let r = m.regs();
    let rr = m.r_register();
    let pc = if m.halted() && r.iff1 == 0 { r.pc.wrapping_sub(1) } else { r.pc };
    let mut h = Vec::with_capacity(0xC000 + 86);
    h.extend_from_slice(&[(r.af >> 8) as u8, r.af as u8]);
    h.extend_from_slice(&r.bc.to_le_bytes());
    h.extend_from_slice(&r.hl.to_le_bytes());
    h.extend_from_slice(&[0, 0]); // PC 0: version 2 or 3
    h.extend_from_slice(&r.sp.to_le_bytes());
    h.extend_from_slice(&[r.i, rr & 0x7F, (rr >> 7) | (m.border() & 7) << 1]);
    for w in [r.de, r.bc_, r.de_, r.hl_] {
        h.extend_from_slice(&w.to_le_bytes());
    }
    h.extend_from_slice(&[(r.af_ >> 8) as u8, r.af_ as u8]);
    h.extend_from_slice(&r.iy.to_le_bytes());
    h.extend_from_slice(&r.ix.to_le_bytes());
    h.extend_from_slice(&[(r.iff1 != 0) as u8, (r.iff2 != 0) as u8, r.im & 3]);
    debug_assert_eq!(h.len(), 30);
    let mut ext = [0u8; 54];
    ext[0..2].copy_from_slice(&pc.to_le_bytes());
    // ext[2] = 0: a 48K; the T-state counter at frame T-state 0: the high counter (quarter
    // frames, counting up modulo 4) is 3 just after the interrupt, and the low counter counts
    // down from 17471 in each quarter
    ext[23..25].copy_from_slice(&(17_471u16).to_le_bytes());
    ext[25] = 3;
    h.extend_from_slice(&54u16.to_le_bytes());
    h.extend_from_slice(&ext);
    for (page, base) in [(4u8, 0x8000u16), (5, 0xC000), (8, 0x4000)] {
        let raw: Vec<u8> = (0..0x4000u16).map(|i| m.peek(base + i)).collect();
        let packed = z80_compress(&raw);
        if packed.len() < raw.len() {
            h.extend_from_slice(&(packed.len() as u16).to_le_bytes());
            h.push(page);
            h.extend_from_slice(&packed);
        } else {
            h.extend_from_slice(&[0xFF, 0xFF, page]);
            h.extend_from_slice(&raw);
        }
    }
    h
}

/// A game file ready for the pack (the routing of the design's goal 4):
///
/// - `.sna` and 48K `.z80`: as they are ("as is");
/// - `.tap`, and `.tzx` with only standard blocks: as a tape ("tape") when the trap trial
///   succeeds, else loaded through the signal and stored as its `.z80` ("loaded (own loader)");
/// - any other `.tzx`: loaded through the signal ("loaded");
/// - 128K files: `Err("128K, left out")`.
pub fn prepare(path: &Path) -> Result<Loaded, String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    if !["sna", "z80", "tap", "tzx"].contains(&ext.as_str()) {
        return Err("not a game file (.sna, .z80, .tap or .tzx)".into());
    }
    let size = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_FILE as u64 {
        return Err("file too big".into());
    }
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if is_128k(&data, &ext) {
        return Err("128K, left out".into());
    }
    let loaded = |m: Machine, how| Loaded { kind: Kind::Snapshot, data: save_z80(&m), frames: m.frame_count(), how };
    match ext.as_str() {
        "sna" | "z80" => {
            let mut m = Machine::new();
            let r = if ext == "sna" { m.load_sna(&data) } else { m.load_z80(&data) };
            r.map_err(|_| format!("not a 48K .{ext} snapshot (malformed or cut short)"))?;
            Ok(Loaded { kind: Kind::Snapshot, data, frames: 0, how: "as is" })
        }
        _ => {
            let tap = if ext == "tap" { Some(data.clone()) } else { tzx::to_tap(&data).ok() };
            let Some(tap) = tap else {
                return Ok(loaded(load_signal(&data)?, "loaded"));
            };
            match trap_run(&tap) {
                Ok(frames) => Ok(Loaded { kind: Kind::Tape, data: tap, frames, how: "tape" }),
                Err(trap) => match load_signal(&data) {
                    Ok(m) => Ok(loaded(m, "loaded (own loader)")),
                    Err(e) if e == trap => Err(e),
                    Err(e) => Err(format!("{e} (through the ROM trap: {trap})")),
                },
            }
        }
    }
}
