// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The parts of a C64 a SID tune needs, and the capture of its SID registers.
//!
//! Memory: 64 KB of RAM; the I/O area at $D000-$DFFF and the KERNAL at $E000-$FFFF as the CPU
//! port ($00/$01) maps them (the BASIC and character ROMs are absent: RAM shows through). The
//! KERNAL is a stand-in: its interrupt entry ($FF48: save A, X, Y, then `JMP ($0314)`, or
//! `($0316)` for a BRK), its interrupt exits ($EA31 and $EA7E acknowledge CIA 1, $EA81 and
//! $FEBC restore A, X, Y and return), its NMI entry ($FE43: `JMP ($0318)`), the vectors, and
//! `RTS` at $FF81, $FF84, $FF87 and $FF8A; any other KERNAL address is a `JAM`, so a tune that
//! calls the KERNAL stops with a reason. I/O: the SID's registers ($D400-$D7FF, 32 mirrored;
//! writes captured, $D41B reads noise), VIC-II's raster counter, compare and interrupt registers
//! (PAL 312 lines of 63 cycles, NTSC 263 of 65), and both CIAs' timers A and B with their
//! interrupt control (CIA 1 drives the IRQ line, CIA 2 the NMI).
//!
//! # Playing
//!
//! A PSID with a play address: init is called (A = the subtune - 1, the CPU port as the address
//! needs: $37 below $A000, $36 below $D000, $34 in $D000-$DFFF, $35 above), then play once per
//! vertical blank (PAL 19,656 cycles, NTSC 17,095) or, for a CIA-timed song, once per CIA 1
//! timer A period (its latch + 1, read after every call: tunes change tempo with it). A call
//! returns by `RTS`, by an `RTI` with only the call's return address on the stack, or by
//! reaching a KERNAL interrupt exit. No interrupts are taken in this mode.
//!
//! A PSID with play address 0, and an RSID: the machine runs freely. CIA 1's timer A starts as
//! the KERNAL leaves it (60 Hz, its interrupt enabled), except a PSID timed by the vertical
//! blank, which gets VIC-II's raster interrupt instead; the tune's init reprograms what it
//! wants. Interrupts go through $FFFE (the stand-in KERNAL's $FF48 and the vector at $0314
//! while the KERNAL is mapped) and $FFFA. A PSID's init runs with interrupts off and must
//! return; an RSID's runs with them on and need not. After init the CPU idles until the next
//! interrupt. A row is captured at the end of each interrupt handler (an `RTI` back to the main
//! level) and timed at the interrupt, at most one per 1,229 cycles, or after 1.5 frames without
//! one (a tune playing from its main loop); rows start at the tune's first SID write.

use crate::cpu::{Bus, Cpu, Jam, I, U};
use crate::sidfile::SidFile;

/// The SID registers captured: $D400-$D418.
pub const SID_REGS: usize = 25;

/// The SID's state after one play call (or interrupt handler).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Cycles since the first row.
    pub t: u64,
    pub regs: [u8; SID_REGS],
    /// Bit v: voice v's gate went from off to on during the call (with the gate on before the
    /// call, a retrigger: off and on within it).
    pub rise: u8,
    /// Bit v: voice v was set to noise with its gate on during the call (from not so).
    pub noise_on: u8,
}

/// A tune's SID registers over time.
#[derive(Clone, Debug)]
pub struct Capture {
    pub clock_hz: u32,
    pub rows: Vec<Row>,
    /// Cycles since the first row at which the capture ends.
    pub end: u64,
}

/// The address an `RTS` from a called routine lands on; the CPU never runs code there.
const TRAP: u16 = 0x0001;
const PAL_LINE: u32 = 63;
const PAL_LINES: u32 = 312;
const NTSC_LINE: u32 = 65;
const NTSC_LINES: u32 = 263;
/// The KERNAL's CIA 1 timer A latch (60 Hz).
const KERNAL_TIMER_PAL: u16 = 0x4025;
const KERNAL_TIMER_NTSC: u16 = 0x4295;
/// The stand-in KERNAL's interrupt exits.
const IRQ_EXITS: [u16; 4] = [0xEA31, 0xEA7E, 0xEA81, 0xFEBC];

#[derive(Clone, Debug)]
struct Timer {
    count: u16,
    latch: u16,
    ctrl: u8,
}

impl Timer {
    fn new() -> Self {
        Timer { count: 0xFFFF, latch: 0xFFFF, ctrl: 0 }
    }

    fn running(&self) -> bool {
        self.ctrl & 1 != 0
    }

    /// Counts `n` pulses down; returns the underflows.
    fn count_down(&mut self, n: u32) -> u32 {
        let mut left = n;
        let mut under = 0;
        while left > 0 && self.running() {
            let c = u32::from(self.count);
            if left <= c {
                self.count = (c - left) as u16;
                break;
            }
            left -= c + 1;
            self.count = self.latch;
            under += 1;
            if self.ctrl & 8 != 0 {
                self.ctrl &= !1;
            }
        }
        under
    }
}

#[derive(Clone, Debug)]
struct Cia {
    a: Timer,
    b: Timer,
    mask: u8,
    flags: u8,
    regs: [u8; 16],
}

impl Cia {
    fn new() -> Self {
        Cia { a: Timer::new(), b: Timer::new(), mask: 0, flags: 0, regs: [0; 16] }
    }

    fn irq(&self) -> bool {
        self.flags & self.mask & 0x1F != 0
    }

    fn step(&mut self, n: u32) {
        let ua = if self.a.ctrl & 0x20 == 0 { self.a.count_down(n) } else { 0 };
        if ua > 0 {
            self.flags |= 1;
        }
        let ub = match self.b.ctrl & 0x60 {
            0x00 => self.b.count_down(n),
            0x40 | 0x60 => self.b.count_down(ua),
            _ => 0,
        };
        if ub > 0 {
            self.flags |= 2;
        }
    }

    /// Cycles until an enabled timer interrupt (at most `cap`).
    fn until_irq(&self, cap: u32) -> u32 {
        let mut n = cap;
        if self.mask & 1 != 0 && self.a.running() && self.a.ctrl & 0x20 == 0 {
            n = n.min(u32::from(self.a.count) + 1);
        }
        if self.mask & 2 != 0 && self.b.running() {
            let b = if self.b.ctrl & 0x60 == 0 { u32::from(self.b.count) + 1 } else { u32::from(self.a.count) + 1 };
            n = n.min(b);
        }
        n
    }

    fn read(&mut self, r: u16) -> u8 {
        match r {
            0 | 1 => 0xFF,
            4 => self.a.count as u8,
            5 => (self.a.count >> 8) as u8,
            6 => self.b.count as u8,
            7 => (self.b.count >> 8) as u8,
            0xD => {
                let v = self.flags | if self.irq() { 0x80 } else { 0 };
                self.flags = 0;
                v
            }
            0xE => self.a.ctrl,
            0xF => self.b.ctrl,
            _ => self.regs[usize::from(r)],
        }
    }

    fn write(&mut self, r: u16, v: u8) {
        let set_latch = |t: &mut Timer, hi: bool| {
            t.latch = if hi { (t.latch & 0x00FF) | u16::from(v) << 8 } else { (t.latch & 0xFF00) | u16::from(v) };
            if hi && !t.running() {
                t.count = t.latch;
            }
        };
        let set_ctrl = |t: &mut Timer| {
            if v & 0x10 != 0 {
                t.count = t.latch;
            }
            t.ctrl = v & !0x10;
        };
        match r {
            4 => set_latch(&mut self.a, false),
            5 => set_latch(&mut self.a, true),
            6 => set_latch(&mut self.b, false),
            7 => set_latch(&mut self.b, true),
            0xD => {
                if v & 0x80 != 0 {
                    self.mask |= v & 0x1F;
                } else {
                    self.mask &= !(v & 0x1F);
                }
            }
            0xE => set_ctrl(&mut self.a),
            0xF => set_ctrl(&mut self.b),
            _ => self.regs[usize::from(r)] = v,
        }
    }
}

#[derive(Clone, Debug)]
struct Vic {
    cmp: u16,
    ctrl1: u8,
    flags: u8,
    mask: u8,
    regs: [u8; 64],
    line_cycles: u32,
    lines: u32,
}

impl Vic {
    fn frame(&self) -> u64 {
        u64::from(self.line_cycles * self.lines)
    }

    fn line(&self, cycles: u64) -> u16 {
        ((cycles / u64::from(self.line_cycles)) % u64::from(self.lines)) as u16
    }

    /// The first cycle after `from` that starts the compare line, if the line exists.
    fn next_cmp(&self, from: u64) -> Option<u64> {
        if u32::from(self.cmp) >= self.lines {
            return None;
        }
        let at = u64::from(self.cmp) * u64::from(self.line_cycles);
        let mut t = from / self.frame() * self.frame() + at;
        if t <= from {
            t += self.frame();
        }
        Some(t)
    }

    fn irq(&self) -> bool {
        self.flags & self.mask & 0x0F != 0
    }
}

/// The CPU's memory and I/O.
struct Mem {
    ram: Vec<u8>,
    kernal: Vec<u8>,
    sid: [u8; 32],
    rise: u8,
    noise_on: u8,
    sid_writes: u64,
    cia1: Cia,
    cia2: Cia,
    vic: Vic,
    cycles: u64,
    noise: u32,
    nmi_line: bool,
    nmi_pending: bool,
}

/// The stand-in KERNAL (see the module docs).
fn kernal() -> Vec<u8> {
    let mut k = vec![0x02u8; 0x2000];
    let mut put = |at: u16, code: &[u8]| {
        let o = usize::from(at - 0xE000);
        k[o..o + code.len()].copy_from_slice(code);
    };
    put(0xFF48, &[0x48, 0x8A, 0x48, 0x98, 0x48, 0xBA, 0xBD, 0x04, 0x01, 0x29, 0x10, 0xF0, 0x03, 0x6C, 0x16, 0x03, 0x6C, 0x14, 0x03]);
    put(0xEA31, &[0x4C, 0x7E, 0xEA]);
    put(0xEA7E, &[0xAD, 0x0D, 0xDC, 0x68, 0xA8, 0x68, 0xAA, 0x68, 0x40]);
    put(0xFEBC, &[0x68, 0xA8, 0x68, 0xAA, 0x68, 0x40]);
    put(0xFE43, &[0x78, 0x6C, 0x18, 0x03]);
    put(0xFE47, &[0x40]);
    for a in [0xFF81, 0xFF84, 0xFF87, 0xFF8A] {
        put(a, &[0x60]);
    }
    put(0xFFFA, &[0x43, 0xFE, 0xE2, 0xFC, 0x48, 0xFF]);
    k
}

impl Mem {
    fn port(&self) -> u8 {
        (self.ram[1] | !self.ram[0]) & 7
    }
    fn io(&self) -> bool {
        let p = self.port();
        p & 3 != 0 && p & 4 != 0
    }
    fn hiram(&self) -> bool {
        self.port() & 2 != 0
    }

    fn io_read(&mut self, a: u16) -> u8 {
        match a {
            0xD000..=0xD3FF => {
                let r = a & 0x3F;
                let line = self.vic.line(self.cycles);
                match r {
                    0x11 => (self.vic.ctrl1 & 0x7F) | ((line >> 1) as u8 & 0x80),
                    0x12 => line as u8,
                    0x19 => self.vic.flags | 0x70 | if self.vic.irq() { 0x80 } else { 0 },
                    0x1A => self.vic.mask | 0xF0,
                    _ => self.vic.regs[usize::from(r)],
                }
            }
            0xD400..=0xD7FF => match a & 0x1F {
                0x19 | 0x1A => 0xFF,
                0x1B => {
                    // a 23-bit LFSR, as the SID's noise
                    let n = self.noise;
                    self.noise = ((n << 1) | (((n >> 22) ^ (n >> 17)) & 1)) & 0x7F_FFFF;
                    (self.noise >> 15) as u8
                }
                _ => 0,
            },
            0xDC00..=0xDCFF => self.cia1.read(a & 0xF),
            0xDD00..=0xDDFF => self.cia2.read(a & 0xF),
            0xDE00..=0xDFFF => 0,
            _ => self.ram[usize::from(a)],
        }
    }

    fn io_write(&mut self, a: u16, v: u8) {
        match a {
            0xD000..=0xD3FF => {
                let r = a & 0x3F;
                match r {
                    0x11 => {
                        self.vic.ctrl1 = v;
                        self.vic.cmp = (self.vic.cmp & 0xFF) | (u16::from(v & 0x80) << 1);
                    }
                    0x12 => self.vic.cmp = (self.vic.cmp & 0x100) | u16::from(v),
                    0x19 => self.vic.flags &= !(v & 0x0F),
                    0x1A => self.vic.mask = v & 0x0F,
                    _ => self.vic.regs[usize::from(r)] = v,
                }
            }
            0xD400..=0xD7FF => {
                let r = usize::from(a & 0x1F);
                if matches!(r, 4 | 11 | 18) {
                    let (bit, old) = (1u8 << (r / 7), self.sid[r]);
                    if v & 1 != 0 && old & 1 == 0 {
                        self.rise |= bit;
                    }
                    if v & 0x81 == 0x81 && old & 0x81 != 0x81 {
                        self.noise_on |= bit;
                    }
                }
                self.sid[r] = v;
                self.sid_writes += 1;
            }
            0xDC00..=0xDCFF => self.cia1.write(a & 0xF, v),
            0xDD00..=0xDDFF => self.cia2.write(a & 0xF, v),
            0xD800..=0xDBFF => self.ram[usize::from(a)] = v,
            _ => {}
        }
    }

    /// Advances the clock, the timers and the raster by `n` cycles.
    fn advance(&mut self, n: u32) {
        let from = self.cycles;
        self.cycles += u64::from(n);
        self.cia1.step(n);
        self.cia2.step(n);
        if self.vic.next_cmp(from).is_some_and(|t| t <= self.cycles) {
            self.vic.flags |= 1;
        }
        let nmi = self.cia2.irq();
        if nmi && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = nmi;
    }

    fn irq_line(&self) -> bool {
        self.cia1.irq() || self.vic.irq()
    }

    fn row(&mut self, t: u64) -> Row {
        let mut regs = [0; SID_REGS];
        regs.copy_from_slice(&self.sid[..SID_REGS]);
        let r = Row { t, regs, rise: self.rise, noise_on: self.noise_on };
        self.rise = 0;
        self.noise_on = 0;
        r
    }
}

impl Bus for Mem {
    fn read(&mut self, a: u16) -> u8 {
        match a {
            0xD000..=0xDFFF if self.io() => self.io_read(a),
            0xE000..=0xFFFF if self.hiram() => self.kernal[usize::from(a - 0xE000)],
            _ => self.ram[usize::from(a)],
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        if (0xD000..=0xDFFF).contains(&a) && self.io() {
            self.io_write(a, v);
        } else {
            self.ram[usize::from(a)] = v;
        }
    }
}

/// The CPU port value a PSID's routine at `addr` is called with.
fn iomap(addr: u16) -> u8 {
    match addr {
        0..=0x9FFF => 0x37,
        0xA000..=0xCFFF => 0x36,
        0xD000..=0xDFFF => 0x34,
        _ => 0x35,
    }
}

struct C64 {
    cpu: Cpu,
    mem: Mem,
    ntsc: bool,
}

impl C64 {
    fn new(file: &SidFile) -> C64 {
        let ntsc = file.ntsc();
        let mut ram = vec![0u8; 0x1_0000];
        ram[0] = 0x2F;
        ram[1] = 0x37;
        ram[0x02A6] = u8::from(!ntsc);
        ram[0x0314..0x031A].copy_from_slice(&[0x31, 0xEA, 0x66, 0xFE, 0x47, 0xFE]);
        let at = usize::from(file.load);
        ram[at..at + file.data.len()].copy_from_slice(&file.data);
        let (line_cycles, lines) = if ntsc { (NTSC_LINE, NTSC_LINES) } else { (PAL_LINE, PAL_LINES) };
        let mut cia1 = Cia::new();
        cia1.a.latch = if ntsc { KERNAL_TIMER_NTSC } else { KERNAL_TIMER_PAL };
        cia1.a.count = cia1.a.latch;
        cia1.a.ctrl = 0x01;
        cia1.mask = 0x01;
        let mem = Mem {
            ram,
            kernal: kernal(),
            sid: [0; 32],
            rise: 0,
            noise_on: 0,
            sid_writes: 0,
            cia1,
            cia2: Cia::new(),
            vic: Vic { cmp: 0, ctrl1: 0x1B, flags: 0, mask: 0, regs: [0; 64], line_cycles, lines },
            cycles: 0,
            noise: 0x7F_FFF8,
            nmi_line: false,
            nmi_pending: false,
        };
        C64 { cpu: Cpu::default(), mem, ntsc }
    }

    fn frame_cycles(&self) -> u64 {
        self.mem.vic.frame()
    }

    /// Runs one instruction.
    fn exec(&mut self) -> Result<u32, String> {
        match self.cpu.step(&mut self.mem) {
            Ok(n) => {
                self.mem.advance(n);
                Ok(n)
            }
            Err(Jam(at)) => Err(match at {
                0xFE66 => "the tune ran into a BRK (code it does not have)".to_string(),
                0xFCE2 => "the tune reset the C64".to_string(),
                0xE000..=0xFFFF if self.mem.hiram() => {
                    format!("the tune calls the KERNAL at ${at:04X}, which is not provided")
                }
                _ => format!("the tune ran into a JAM opcode at ${at:04X}"),
            }),
        }
    }

    /// Calls the routine at `addr` with A = `a` and runs it until it returns (see the module
    /// docs), interrupts off, for at most `max` cycles.
    fn call(&mut self, addr: u16, a: u8, max: u64, what: &str) -> Result<(), String> {
        self.cpu.sp = 0xFF;
        let [lo, hi] = TRAP.wrapping_sub(1).to_le_bytes();
        self.cpu.push(&mut self.mem, hi);
        self.cpu.push(&mut self.mem, lo);
        let frame_sp = self.cpu.sp;
        self.cpu.pc = addr;
        self.cpu.a = a;
        self.cpu.x = 0;
        self.cpu.y = 0;
        self.cpu.p = U | I;
        let start = self.mem.cycles;
        loop {
            let pc = self.cpu.pc;
            if pc == TRAP || (self.mem.hiram() && IRQ_EXITS.contains(&pc)) {
                return Ok(());
            }
            if self.cpu.sp == frame_sp && self.mem.read(pc) == 0x40 {
                return Ok(()); // an RTI with only the call's frame on the stack
            }
            self.exec()?;
            if self.mem.cycles - start > max {
                let secs = max as f64 / self.clock() as f64;
                return Err(format!("the {what} routine at ${addr:04X} does not return within {secs:.1} s"));
            }
        }
    }

    fn clock(&self) -> u32 {
        if self.ntsc {
            crate::sidfile::NTSC_HZ
        } else {
            crate::sidfile::PAL_HZ
        }
    }
}

/// Plays `song` (1-based) of `file` for `seconds` and captures the SID registers once per play
/// call (see the module docs). Err: why it cannot be played.
pub fn capture(file: &SidFile, song: u16, seconds: f64) -> Result<Capture, String> {
    if let Some(r) = file.refusal() {
        return Err(r);
    }
    if song == 0 || song > file.songs {
        return Err(format!("no subtune {song}: the file has {} (1-{})", file.songs, file.songs));
    }
    let mut m = C64::new(file);
    let clock = u64::from(m.clock());
    let span = (seconds.max(0.0) * clock as f64) as u64;
    let a = (song - 1) as u8;
    if !file.rsid && file.play != 0 {
        direct(&mut m, file, song, a, span, clock)
    } else {
        free_running(&mut m, file, song, a, span, clock)
    }
}

fn direct(m: &mut C64, file: &SidFile, song: u16, a: u8, span: u64, clock: u64) -> Result<Capture, String> {
    m.mem.ram[1] = iomap(file.init);
    m.call(file.init, a, clock * 10, "init")?;
    let cia = file.cia_timed(song);
    let base = m.mem.cycles;
    let mut rows = Vec::new();
    let mut t = 0;
    while t < span {
        m.mem.cycles = m.mem.cycles.max(base + t);
        m.mem.ram[1] = iomap(file.play);
        m.mem.rise = 0;
        m.mem.noise_on = 0;
        m.call(file.play, 0, clock / 2, "play")?;
        rows.push(m.mem.row(t));
        // the play rate: the vertical blank, or CIA 1 timer A's period (at most about 500 Hz)
        t += if cia { (u64::from(m.mem.cia1.a.latch) + 1).max(2000) } else { m.frame_cycles() };
    }
    if m.mem.sid_writes == 0 {
        return Err(format!("the tune wrote nothing to the SID in {:.0} s", span as f64 / clock as f64));
    }
    Ok(Capture { clock_hz: clock as u32, rows, end: span })
}

fn free_running(m: &mut C64, file: &SidFile, song: u16, a: u8, span: u64, clock: u64) -> Result<Capture, String> {
    if !file.rsid {
        if file.cia_timed(song) {
            m.mem.vic.mask = 0;
        } else {
            m.mem.cia1.mask = 0;
            m.mem.vic.mask = 1;
        }
        m.mem.ram[1] = iomap(file.init);
    }
    let [lo, hi] = TRAP.wrapping_sub(1).to_le_bytes();
    m.cpu.sp = 0xFF;
    m.cpu.push(&mut m.mem, hi);
    m.cpu.push(&mut m.mem, lo);
    m.cpu.pc = file.init;
    m.cpu.a = a;
    m.cpu.p = if file.rsid { U } else { U | I };
    let frame = m.frame_cycles();
    let (min_gap, forced) = (frame / 16, frame * 3 / 2);
    let init_limit = clock * 10;
    let mut init_done = file.rsid;
    // the interrupts being served: true an IRQ, false an NMI
    let mut nest: Vec<bool> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut t0 = None::<u64>;
    let mut last = 0u64;
    // when the interrupt being served (at the main level) was taken
    let mut entry = 0u64;
    let give_up = span + clock * 30;
    loop {
        let now = m.mem.cycles;
        if let Some(t0) = t0 {
            if now >= t0 + span {
                break;
            }
            // (not within a handler, unless it has run for as long: one that never returns)
            if now - last >= forced && (nest.is_empty() || now - entry >= forced) {
                last = now;
                rows.push(m.mem.row(now - t0));
            }
        } else if now > give_up {
            break;
        }
        if m.mem.nmi_pending {
            m.mem.nmi_pending = false;
            let n = m.cpu.nmi(&mut m.mem);
            m.mem.advance(n);
            nest.push(false);
            continue;
        }
        if init_done && m.mem.irq_line() && m.cpu.p & I == 0 {
            if !nest.contains(&true) {
                entry = now;
            }
            let n = m.cpu.irq(&mut m.mem);
            m.mem.advance(n);
            nest.push(true);
            continue;
        }
        let pc = m.cpu.pc;
        let op = m.mem.read(pc);
        let spin = op == 0x4C && m.mem.read(pc.wrapping_add(1)) as u16 | (m.mem.read(pc.wrapping_add(2)) as u16) << 8 == pc;
        if pc == TRAP || spin {
            if pc == TRAP && !init_done {
                init_done = true;
                m.cpu.p &= !I;
                continue;
            }
            // idle until the next interrupt, row or the end
            let mut n = 20_000u64;
            n = n.min(u64::from(m.mem.cia1.until_irq(20_000))).min(u64::from(m.mem.cia2.until_irq(20_000)));
            if m.mem.vic.mask & 1 != 0 {
                if let Some(t) = m.mem.vic.next_cmp(now) {
                    n = n.min(t - now);
                }
            }
            if let Some(t0) = t0 {
                n = n.min((last + forced).saturating_sub(now)).min((t0 + span).saturating_sub(now));
            }
            m.mem.advance(n.max(1) as u32);
            continue;
        }
        let rti = op == 0x40;
        m.exec()?;
        if !init_done && m.mem.cycles > init_limit {
            return Err(format!("the init routine at ${:04X} does not return within 10 s", file.init));
        }
        if rti && nest.pop() == Some(true) && !nest.contains(&true) && m.mem.sid_writes > 0 {
            match t0 {
                None => {
                    t0 = Some(entry);
                    last = entry;
                    rows.push(m.mem.row(0));
                }
                Some(t0) if entry >= last + min_gap => {
                    last = entry;
                    rows.push(m.mem.row(entry - t0));
                }
                _ => {}
            }
        }
        if t0.is_none() && m.mem.sid_writes > 0 && init_done && nest.is_empty() {
            // a tune playing from its main loop: rows start at its first write
            t0 = Some(m.mem.cycles);
            last = m.mem.cycles;
            rows.push(m.mem.row(0));
        }
    }
    if rows.is_empty() || m.mem.sid_writes == 0 {
        return Err(format!("the tune wrote nothing to the SID in {:.0} s", give_up as f64 / clock as f64));
    }
    Ok(Capture { clock_hz: clock as u32, rows, end: span })
}
