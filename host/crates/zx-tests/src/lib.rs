// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared fixtures for the machine tests.

pub use zx_sys::{cmd, ev, view, Menu, Ui, UI_COLS, UI_ROWS, km_default, km_basic, MENU_BASIC, KeyMapper, KmInput, KM_INPUTS, KM_NONE, KM_PULSE_FRAMES, lcd_budget_bytes, zx_edge, zx_regs, Beeper, Machine, PixFormat, Sched};

/// Pokes `bytes` into the machine starting at `addr`.
pub fn load(m: &mut Machine, addr: u16, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        m.poke(addr.wrapping_add(i as u16), *b);
    }
}

pub use zx_sys::contention;

fn run_one(m: &mut Machine, pc: u16, a: u8, t: u32) -> u32 {
    let mut r = m.regs();
    r.pc = pc;
    r.af = (r.af & 0x00FF) | ((a as u16) << 8);
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.test_set_tstate(t);
    m.test_step()
}

/// T-states of a NOP at `addr` fetched at frame T-state `t`.
pub fn cycles_of_nop_at(addr: u16, t: u32) -> u32 {
    let mut m = Machine::new();
    load(&mut m, addr, &[0x00]);
    run_one(&mut m, addr, 0, t)
}

/// T-states of `IN A,(n)` at 0x8000 (A = `a_high`) whose I/O cycle starts at
/// T-state 14335.
pub fn cycles_of_in_at(a_high: u8, n: u8) -> u32 {
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0xDB, n]);
    run_one(&mut m, 0x8000, a_high, 14335 - 7)
}

/// Runs `frames` frames of a program that idles with interrupts enabled
/// (or disabled, when `di`) under IM 2; the handler increments a RAM byte.
/// Returns the byte.
pub fn count_interrupts(frames: u32, di: bool) -> u8 {
    let mut m = Machine::new();
    // vector table: I = 0x80, bus value 0xFF -> word at 0x80FF
    load(&mut m, 0x80FF, &[0x00, 0x90]);
    // handler: PUSH AF; LD A,(A000h); INC A; LD (A000h),A; POP AF; EI; RETI
    load(
        &mut m,
        0x9000,
        &[0xF5, 0x3A, 0x00, 0xA0, 0x3C, 0x32, 0x00, 0xA0, 0xF1, 0xFB, 0xED, 0x4D],
    );
    // main: EI (or DI); JR $
    load(&mut m, 0x8000, &[if di { 0xF3 } else { 0xFB }, 0x18, 0xFE]);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFFF0;
    r.i = 0x80;
    r.im = 2;
    m.set_regs(&r);
    for _ in 0..frames {
        m.run_frame();
    }
    m.peek(0xA000)
}

/// Executes the instruction bytes at 0x8000 with `b`, `c`, `a` loaded and
/// interrupts off, starting at frame T-state `t`. Returns the machine.
pub fn run_at(m: &mut Machine, code: &[u8], a: u8, b: u8, c: u8, t: u32) {
    load(m, 0x8000, code);
    let mut r = m.regs();
    r.pc = 0x8000;
    r.af = (r.af & 0x00FF) | ((a as u16) << 8);
    r.bc = ((b as u16) << 8) | c as u16;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.test_set_tstate(t);
    m.test_step();
}

/// A as left by the last instruction.
pub fn reg_a(m: &Machine) -> u8 {
    (m.regs().af >> 8) as u8
}

/// Text of character row `row` (0-23): each 8x8 cell is matched against the ROM
/// font (0x3D00, chars 0x20-0x7F, 0x7F is the copyright sign) by bitmap alone;
/// `?` when nothing matches. Trailing spaces are trimmed.
pub fn screen_text(m: &Machine, row: usize) -> String {
    let lines: Vec<[u8; 256]> = (0..8).map(|l| m.screen_line(row * 8 + l)).collect();
    let mut text = String::new();
    for col in 0..32 {
        // The cell's two colours are unknown (the ROM prints black on white),
        // so accept the bitmap with either colour as the set bits.
        let min = lines.iter().flat_map(|l| l[col * 8..col * 8 + 8].iter().copied()).min().unwrap_or(0);
        let pattern = |set_is_min: bool| -> Vec<u8> {
            lines
                .iter()
                .map(|l| {
                    let px = &l[col * 8..col * 8 + 8];
                    px.iter().fold(0u8, |b, &p| (b << 1) | u8::from((p == min) == set_is_min))
                })
                .collect()
        };
        let (a, b) = (pattern(false), pattern(true));
        let found = (0x20u16..=0x7F).find(|&c| {
            let base = 0x3D00 + (c - 0x20) * 8;
            (0..8).all(|i| m.peek(base + i) == a[i as usize])
                || (0..8).all(|i| m.peek(base + i) == b[i as usize])
        });
        text.push(match found {
            Some(0x7F) => '\u{a9}',
            Some(c) => c as u8 as char,
            None => '?',
        });
    }
    text.trim_end().to_string()
}

// ---- snapshot fixtures -------------------------------------------------

/// Registers used by the snapshot tests: every field distinct, IM 2, IFFs set,
/// R with bit 7 set, PC nonzero.
pub fn sample_regs() -> zx_sys::zx_regs {
    zx_sys::zx_regs {
        af: 0x1A2B,
        bc: 0x3C4D,
        de: 0x5E6F,
        hl: 0x7081,
        af_: 0x9293,
        bc_: 0xA4B5,
        de_: 0xC6D7,
        hl_: 0xE8F9,
        ix: 0x0A1B,
        iy: 0x2C3D,
        sp: 0xFF00,
        pc: 0x8123,
        i: 0x3F,
        r: 0xA5,
        iff1: 1,
        iff2: 1,
        im: 2,
    }
}

/// 48 KB of RAM with runs, ED ED pairs and lone EDs, so the compressor has work.
pub fn sample_ram() -> Vec<u8> {
    let mut ram = vec![0u8; 49152];
    for (i, b) in ram.iter_mut().enumerate() {
        *b = ((i * 7) ^ (i >> 5)) as u8;
    }
    ram[100..1100].fill(0); // long zero run
    ram[2000..2300].fill(0x55); // run longer than 255
    ram[3000..3010].fill(0xED); // run of EDs
    ram[3100] = 0xED; // lone ED then a would-be run
    ram[3101..3110].fill(0x77);
    ram[4000] = 0xED; // lone ED at a chunk edge
    ram[4001] = 0xED;
    ram[4002] = 0x01;
    ram[10000..20000].fill(0x20);
    ram[0x1800..0x1B00].fill(0x38); // attributes
    ram[49151] = 0x42;
    ram
}

/// ED ED nn bb run-length encoding of `data` (never ends with a lone ED).
pub fn z80_compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        let mut run = 1;
        while i + run < data.len() && data[i + run] == b && run < 255 {
            run += 1;
        }
        if run >= 5 || (b == 0xED && run >= 2) {
            out.extend_from_slice(&[0xED, 0xED, run as u8, b]);
            i += run;
        } else if b == 0xED {
            // a byte after a lone ED is never part of a run
            out.push(0xED);
            out.push(data[i + 1]);
            i += 2;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// A 48K `.sna`: PC is pushed on the stack (SP - 2), border in the header.
pub fn make_sna(regs: &zx_sys::zx_regs, ram: &[u8], border: u8) -> Vec<u8> {
    let mut ram = ram.to_vec();
    let sp = regs.sp.wrapping_sub(2);
    for (k, b) in regs.pc.to_le_bytes().iter().enumerate() {
        let a = sp.wrapping_add(k as u16);
        if a >= 0x4000 {
            ram[a as usize - 0x4000] = *b;
        }
    }
    let mut h = vec![regs.i];
    for w in [regs.hl_, regs.de_, regs.bc_, regs.af_, regs.hl, regs.de, regs.bc, regs.iy, regs.ix] {
        h.extend_from_slice(&w.to_le_bytes());
    }
    h.push(regs.iff2 << 2);
    h.push(regs.r);
    h.extend_from_slice(&regs.af.to_le_bytes());
    h.extend_from_slice(&sp.to_le_bytes());
    h.push(regs.im);
    h.push(border);
    assert_eq!(h.len(), 27);
    h.extend_from_slice(&ram);
    h
}

fn z80_header(regs: &zx_sys::zx_regs, border: u8, pc: u16, compressed: bool) -> Vec<u8> {
    let mut h = vec![(regs.af >> 8) as u8, regs.af as u8];
    for w in [regs.bc, regs.hl, pc, regs.sp] {
        h.extend_from_slice(&w.to_le_bytes());
    }
    h.push(regs.i);
    h.push(regs.r & 0x7F);
    h.push((regs.r >> 7) | (border << 1) | ((compressed as u8) << 5));
    for w in [regs.de, regs.bc_, regs.de_, regs.hl_] {
        h.extend_from_slice(&w.to_le_bytes());
    }
    h.push((regs.af_ >> 8) as u8);
    h.push(regs.af_ as u8);
    h.extend_from_slice(&regs.iy.to_le_bytes());
    h.extend_from_slice(&regs.ix.to_le_bytes());
    h.push(regs.iff1);
    h.push(regs.iff2);
    h.push(regs.im);
    assert_eq!(h.len(), 30);
    h
}

/// A version 1 `.z80`, border 3 (header byte 12 carries it).
pub fn make_z80_v1(regs: &zx_sys::zx_regs, ram: &[u8], compressed: bool) -> Vec<u8> {
    assert_ne!(regs.pc, 0);
    let mut f = z80_header(regs, 3, regs.pc, compressed);
    if compressed {
        f.extend(z80_compress(ram));
        f.extend_from_slice(&[0x00, 0xED, 0xED, 0x00]);
    } else {
        f.extend_from_slice(ram);
    }
    f
}

/// A version 2 (`version == 2`, header 23) or 3 (54) `.z80` with pages 8, 4, 5;
/// page 4 is stored compressed, the others plain. Border 3.
pub fn make_z80_vn(regs: &zx_sys::zx_regs, ram: &[u8], version: u8, hw_mode: u8) -> Vec<u8> {
    let mut f = z80_header(regs, 3, 0, false);
    let ext: u16 = if version == 2 { 23 } else { 54 };
    f.extend_from_slice(&ext.to_le_bytes());
    let mut e = vec![0u8; ext as usize];
    e[0..2].copy_from_slice(&regs.pc.to_le_bytes());
    e[2] = hw_mode;
    f.extend(e);
    for (page, base) in [(8u8, 0usize), (4, 0x4000), (5, 0x8000)] {
        let data = &ram[base..base + 0x4000];
        if page == 4 {
            let c = z80_compress(data);
            f.extend_from_slice(&(c.len() as u16).to_le_bytes());
            f.push(page);
            f.extend(c);
        } else {
            f.extend_from_slice(&[0xFF, 0xFF, page]);
            f.extend_from_slice(data);
        }
    }
    f
}

pub fn make_z80_v3(regs: &zx_sys::zx_regs, ram: &[u8], hw_mode: u8) -> Vec<u8> {
    make_z80_vn(regs, ram, 3, hw_mode)
}

// ---- tape fixtures -----------------------------------------------------

/// A `.tap` block: 2-byte length, flag, data, XOR checksum.
pub fn tap_block(flag: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![flag];
    b.extend_from_slice(data);
    b.push(b.iter().fold(0u8, |x, &y| x ^ y));
    let mut out = (b.len() as u16).to_le_bytes().to_vec();
    out.extend(b);
    out
}

/// A header block (flag 0): kind 0 program, 3 code; `name` is padded to 10.
pub fn tap_header(kind: u8, name: &str, len: u16, p1: u16, p2: u16) -> Vec<u8> {
    let mut d = vec![kind];
    let mut n = name.as_bytes().to_vec();
    n.resize(10, b' ');
    d.extend(n);
    for w in [len, p1, p2] {
        d.extend_from_slice(&w.to_le_bytes());
    }
    tap_block(0x00, &d)
}

/// A tokenised BASIC line: number (big endian), length, `tokens`, ENTER.
pub fn basic_line(n: u16, tokens: &[u8]) -> Vec<u8> {
    let mut out = n.to_be_bytes().to_vec();
    out.extend_from_slice(&((tokens.len() + 1) as u16).to_le_bytes());
    out.extend_from_slice(tokens);
    out.push(0x0D);
    out
}

// ---- pack and golden-hash fixtures --------------------------------------

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// A valid pack of stored games named `names`, each with the default key map; built without the
/// packer crate.
pub fn build_pack(names: &[&str]) -> Vec<u8> {
    let mut p = vec![0u8; 32 + 160 * names.len()];
    p[..8].copy_from_slice(b"FMZXPACK");
    p[8..10].copy_from_slice(&2u16.to_le_bytes());
    p[10..12].copy_from_slice(&(names.len() as u16).to_le_bytes());
    let keys = km_default();
    for (i, name) in names.iter().enumerate() {
        let e = 32 + 160 * i;
        let raw = [i as u8; 16];
        let off = p.len();
        p.extend_from_slice(&raw);
        p[e..e + name.len()].copy_from_slice(name.as_bytes());
        p[e + 29] = zx_sys::PACK_SNA;
        p[e + 30] = zx_sys::PACK_STORED;
        p[e + 31] = 8;
        put32(&mut p, e + 32, off as u32);
        put32(&mut p, e + 36, raw.len() as u32);
        put32(&mut p, e + 40, raw.len() as u32);
        put32(&mut p, e + 44, zx_sys::pack_crc32(&raw));
        for (k, a) in keys.iter().enumerate() {
            p[e + 48 + 2 * k] = a[0];
            p[e + 49 + 2 * k] = a[1];
        }
    }
    let total = p.len() as u32;
    put32(&mut p, 12, total);
    let crc = zx_sys::pack_crc32(&p[32..]);
    put32(&mut p, 16, crc);
    p
}

/// FNV-1a (32 bit) of `data`.
pub fn fnv1a(data: &[u8]) -> u32 {
    data.iter().fold(0x811C_9DC5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// Compares `hex` with the entry `name` of `tests/golden/hashes.txt` (`name = hex` lines), or
/// rewrites the entry when `FMZX_GOLDEN_UPDATE=1`.
pub fn golden(name: &str, hex: &str) {
    use std::fmt::Write as _;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/hashes.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| l.split_once(" = "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if std::env::var("FMZX_GOLDEN_UPDATE").is_ok_and(|v| v == "1") {
        match lines.iter_mut().find(|(k, _)| k == name) {
            Some(l) => l.1 = hex.to_string(),
            None => lines.push((name.to_string(), hex.to_string())),
        }
        lines.sort();
        let mut out = String::new();
        for (k, v) in &lines {
            writeln!(out, "{k} = {v}").unwrap();
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, out).unwrap();
        return;
    }
    let want = lines.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    assert_eq!(want, Some(hex), "golden hash {name} (FMZX_GOLDEN_UPDATE=1 rewrites it)");
}
