// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A 6502 (NMOS 6510) CPU: every documented opcode, decimal mode, and the stable undocumented
//! opcodes tunes use (`LAX`, `SAX`, `DCP`, `ISC`, `SLO`, `RLA`, `SRE`, `RRA`, `ANC`, `ALR`, `ARR`,
//! `SBX`, the `NOP`s; `ANE`, `LXA`, `LAS`, `SHA`, `SHX`, `SHY` and `TAS` as their common forms).
//! A `JAM` opcode stops the CPU ([`Jam`]). Cycle counts per instruction include the page-cross
//! and branch penalties; the bus is not cycle-exact within an instruction.

/// The memory a [`Cpu`] reads and writes.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);
}

pub const C: u8 = 0x01;
pub const Z: u8 = 0x02;
pub const I: u8 = 0x04;
pub const D: u8 = 0x08;
pub const B: u8 = 0x10;
pub const U: u8 = 0x20;
pub const V: u8 = 0x40;
pub const N: u8 = 0x80;

/// A `JAM` opcode executed at this address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jam(pub u16);

#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub pc: u16,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu { a: 0, x: 0, y: 0, sp: 0xFD, p: U | I, pc: 0 }
    }
}

/// Base cycles per opcode (the NMOS 6502's).
const CYCLES: [u8; 256] = [
    7, 6, 0, 8, 3, 3, 5, 5, 3, 2, 2, 2, 4, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
    6, 6, 0, 8, 3, 3, 5, 5, 4, 2, 2, 2, 4, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
    6, 6, 0, 8, 3, 3, 5, 5, 3, 2, 2, 2, 3, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
    6, 6, 0, 8, 3, 3, 5, 5, 4, 2, 2, 2, 5, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4, //
    2, 6, 0, 6, 4, 4, 4, 4, 2, 5, 2, 5, 5, 5, 5, 5, //
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4, //
    2, 5, 0, 5, 4, 4, 4, 4, 2, 4, 2, 4, 4, 4, 4, 4, //
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6, //
    2, 5, 0, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, //
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Imm,
    Zp,
    Zpx,
    Zpy,
    Abs,
    Absx,
    Absy,
    Indx,
    Indy,
}
use Mode::*;

impl Cpu {
    fn fetch(&mut self, b: &mut impl Bus) -> u8 {
        let v = b.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self, b: &mut impl Bus) -> u16 {
        let lo = self.fetch(b);
        u16::from_le_bytes([lo, self.fetch(b)])
    }

    pub fn push(&mut self, b: &mut impl Bus, v: u8) {
        b.write(0x100 | u16::from(self.sp), v);
        self.sp = self.sp.wrapping_sub(1);
    }

    pub fn pull(&mut self, b: &mut impl Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        b.read(0x100 | u16::from(self.sp))
    }

    fn zp16(b: &mut impl Bus, z: u8) -> u16 {
        u16::from_le_bytes([b.read(u16::from(z)), b.read(u16::from(z.wrapping_add(1)))])
    }

    /// The effective address of `mode`, and whether indexing crossed a page.
    fn ea(&mut self, b: &mut impl Bus, mode: Mode) -> (u16, bool) {
        match mode {
            Imm => {
                let a = self.pc;
                self.pc = self.pc.wrapping_add(1);
                (a, false)
            }
            Zp => (u16::from(self.fetch(b)), false),
            Zpx => (u16::from(self.fetch(b).wrapping_add(self.x)), false),
            Zpy => (u16::from(self.fetch(b).wrapping_add(self.y)), false),
            Abs => (self.fetch16(b), false),
            Absx | Absy => {
                let base = self.fetch16(b);
                let a = base.wrapping_add(u16::from(if mode == Absx { self.x } else { self.y }));
                (a, a & 0xFF00 != base & 0xFF00)
            }
            Indx => {
                let z = self.fetch(b).wrapping_add(self.x);
                (Self::zp16(b, z), false)
            }
            Indy => {
                let z = self.fetch(b);
                let base = Self::zp16(b, z);
                let a = base.wrapping_add(u16::from(self.y));
                (a, a & 0xFF00 != base & 0xFF00)
            }
        }
    }

    fn nz(&mut self, v: u8) -> u8 {
        self.p = (self.p & !(N | Z)) | (v & N) | if v == 0 { Z } else { 0 };
        v
    }

    fn flag(&mut self, f: u8, on: bool) {
        if on {
            self.p |= f;
        } else {
            self.p &= !f;
        }
    }

    fn adc(&mut self, v: u8) {
        let (a, c) = (u16::from(self.a), u16::from(self.p & C));
        let v16 = u16::from(v);
        if self.p & D != 0 {
            let bin = (a + v16 + c) as u8;
            let mut lo = (a & 0x0F) + (v16 & 0x0F) + c;
            let mut hi = (a & 0xF0) + (v16 & 0xF0);
            if lo > 9 {
                lo += 6;
            }
            if lo > 0x0F {
                hi += 0x10;
            }
            self.flag(Z, bin == 0);
            self.flag(N, hi & 0x80 != 0);
            self.flag(V, !(a ^ v16) & (a ^ hi) & 0x80 != 0);
            if hi > 0x90 {
                hi += 0x60;
            }
            self.flag(C, hi > 0xFF);
            self.a = ((hi & 0xF0) | (lo & 0x0F)) as u8;
        } else {
            let s = a + v16 + c;
            self.flag(C, s > 0xFF);
            self.flag(V, !(a ^ v16) & (a ^ s) & 0x80 != 0);
            self.a = self.nz(s as u8);
        }
    }

    fn sbc(&mut self, v: u8) {
        let (a, borrow) = (i16::from(self.a), i16::from(1 - (self.p & C)));
        let v16 = i16::from(v);
        let s = a - v16 - borrow;
        self.flag(C, s >= 0);
        self.flag(V, (a ^ v16) & (a ^ s) & 0x80 != 0);
        self.nz(s as u8);
        if self.p & D != 0 {
            let mut lo = (a & 0x0F) - (v16 & 0x0F) - borrow;
            let mut hi = (a >> 4) - (v16 >> 4);
            if lo & 0x10 != 0 {
                lo -= 6;
                hi -= 1;
            }
            if hi & 0x10 != 0 {
                hi -= 6;
            }
            self.a = (((hi << 4) & 0xF0) | (lo & 0x0F)) as u8;
        } else {
            self.a = s as u8;
        }
    }

    fn cmp(&mut self, r: u8, v: u8) {
        self.flag(C, r >= v);
        self.nz(r.wrapping_sub(v));
    }

    fn asl(&mut self, v: u8) -> u8 {
        self.flag(C, v & 0x80 != 0);
        self.nz(v << 1)
    }
    fn lsr(&mut self, v: u8) -> u8 {
        self.flag(C, v & 1 != 0);
        self.nz(v >> 1)
    }
    fn rol(&mut self, v: u8) -> u8 {
        let c = self.p & C;
        self.flag(C, v & 0x80 != 0);
        self.nz((v << 1) | c)
    }
    fn ror(&mut self, v: u8) -> u8 {
        let c = self.p & C;
        self.flag(C, v & 1 != 0);
        self.nz((v >> 1) | (c << 7))
    }

    /// Pushes PC and P and jumps through `vector` (an interrupt; `brk` sets B in the pushed P).
    fn interrupt(&mut self, b: &mut impl Bus, vector: u16, brk: bool) {
        let [lo, hi] = self.pc.to_le_bytes();
        self.push(b, hi);
        self.push(b, lo);
        let p = (self.p | U) & !B | if brk { B } else { 0 };
        self.push(b, p);
        self.p |= I;
        self.pc = u16::from_le_bytes([b.read(vector), b.read(vector.wrapping_add(1))]);
    }

    /// Takes an IRQ (whatever the I flag: the caller checks it). 7 cycles.
    pub fn irq(&mut self, b: &mut impl Bus) -> u32 {
        self.interrupt(b, 0xFFFE, false);
        7
    }

    /// Takes an NMI. 7 cycles.
    pub fn nmi(&mut self, b: &mut impl Bus) -> u32 {
        self.interrupt(b, 0xFFFA, false);
        7
    }

    /// The mode of a read, write or read-modify-write opcode of the `cc = 01/11` and `cc = 10`
    /// columns.
    fn mode(op: u8) -> Mode {
        let bbb = (op >> 2) & 7;
        let y_index = matches!(op, 0x96 | 0x97 | 0xB6 | 0xB7 | 0x9E | 0x9F | 0xBE | 0xBF);
        match (op & 3, bbb) {
            (1 | 3, 0) => Indx,
            (_, 1) => Zp,
            (_, 0 | 2) => Imm,
            (_, 3) => Abs,
            (_, 4) => Indy,
            (_, 5) if y_index => Zpy,
            (_, 5) => Zpx,
            (_, 6) => Absy,
            (_, _) if y_index => Absy,
            _ => Absx,
        }
    }

    /// Runs one instruction. Returns its cycles, or the JAM it hit (the PC left on it).
    pub fn step(&mut self, b: &mut impl Bus) -> Result<u32, Jam> {
        let at = self.pc;
        let op = self.fetch(b);
        let mut cyc = u32::from(CYCLES[usize::from(op)]);
        let (aaa, bbb, cc) = (op >> 5, (op >> 2) & 7, op & 3);
        // reads add a cycle when indexing crosses a page
        macro_rules! rd {
            () => {{
                let (a, x) = self.ea(b, Self::mode(op));
                if x {
                    cyc += 1;
                }
                b.read(a)
            }};
        }
        macro_rules! rmw {
            ($f:expr) => {{
                let (a, _) = self.ea(b, Self::mode(op));
                let v = b.read(a);
                b.write(a, v);
                let r = $f(self, v);
                b.write(a, r);
                r
            }};
        }
        match (cc, bbb, aaa) {
            // the JAMs
            (2, 0, 0..=3) | (2, 4, _) => {
                self.pc = at;
                return Err(Jam(at));
            }
            (1, _, _) if op == 0x89 => {
                self.pc = self.pc.wrapping_add(1); // NOP #imm
            }
            (1, _, 4) => {
                let (a, _) = self.ea(b, Self::mode(op));
                b.write(a, self.a);
            }
            (1, _, _) => {
                let v = rd!();
                match aaa {
                    0 => self.a = self.nz(self.a | v),
                    1 => self.a = self.nz(self.a & v),
                    2 => self.a = self.nz(self.a ^ v),
                    3 => self.adc(v),
                    5 => self.a = self.nz(v),
                    6 => self.cmp(self.a, v),
                    _ => self.sbc(v),
                }
            }
            (2, 2, 0..=3) => {
                self.a = match aaa {
                    0 => self.asl(self.a),
                    1 => self.rol(self.a),
                    2 => self.lsr(self.a),
                    _ => self.ror(self.a),
                }
            }
            (2, 2, 4) => self.a = self.nz(self.x),
            (2, 2, 5) => self.x = self.nz(self.a),
            (2, 2, 6) => self.x = self.nz(self.x.wrapping_sub(1)),
            (2, 2, 7) | (2, 6, 0..=3 | 6 | 7) => {} // NOP
            (2, 6, 4) => self.sp = self.x,
            (2, 6, 5) => self.x = self.nz(self.sp),
            (2, 0, 4 | 6 | 7) => self.pc = self.pc.wrapping_add(1), // NOP #imm
            (2, 7, 4) => {
                // SHX abs,y
                let (a, _) = self.ea(b, Absy);
                b.write(a, self.x & ((a >> 8) as u8).wrapping_add(1));
            }
            (2, _, 4) => {
                let (a, _) = self.ea(b, Self::mode(op));
                b.write(a, self.x);
            }
            (2, _, 5) => {
                let v = rd!();
                self.x = self.nz(v);
            }
            (2, _, _) => {
                rmw!(|c: &mut Cpu, v: u8| match aaa {
                    0 => c.asl(v),
                    1 => c.rol(v),
                    2 => c.lsr(v),
                    3 => c.ror(v),
                    6 => c.nz(v.wrapping_sub(1)),
                    _ => c.nz(v.wrapping_add(1)),
                });
            }
            (3, 2, _) => {
                // ANC, ALR, ARR (binary form), ANE, LXA, SBX, SBC
                let v = self.fetch(b);
                match aaa {
                    0 | 1 => {
                        self.a = self.nz(self.a & v);
                        self.flag(C, self.a & 0x80 != 0);
                    }
                    2 => self.a = self.lsr(self.a & v),
                    3 => {
                        let t = self.a & v;
                        self.a = self.nz((t >> 1) | ((self.p & C) << 7));
                        self.flag(C, self.a & 0x40 != 0);
                        self.flag(V, ((self.a >> 6) ^ (self.a >> 5)) & 1 != 0);
                    }
                    4 => self.a = self.nz((self.a | 0xEE) & self.x & v),
                    5 => {
                        self.a = self.nz((self.a | 0xEE) & v);
                        self.x = self.a;
                    }
                    6 => {
                        let t = self.a & self.x;
                        self.flag(C, t >= v);
                        self.x = self.nz(t.wrapping_sub(v));
                    }
                    _ => self.sbc(v),
                }
            }
            (3, 4 | 7, 4) => {
                // SHA (zp),y / abs,y
                let (a, _) = self.ea(b, Self::mode(op));
                b.write(a, self.a & self.x & ((a >> 8) as u8).wrapping_add(1));
            }
            (3, 6, 4) => {
                // TAS abs,y
                let (a, _) = self.ea(b, Absy);
                self.sp = self.a & self.x;
                b.write(a, self.sp & ((a >> 8) as u8).wrapping_add(1));
            }
            (3, 6, 5) => {
                // LAS abs,y
                let v = rd!() & self.sp;
                self.sp = v;
                self.x = v;
                self.a = self.nz(v);
            }
            (3, _, 4) => {
                let (a, _) = self.ea(b, Self::mode(op));
                b.write(a, self.a & self.x);
            }
            (3, _, 5) => {
                let v = rd!();
                self.x = v;
                self.a = self.nz(v);
            }
            (3, _, _) => {
                rmw!(|c: &mut Cpu, v: u8| match aaa {
                    0 => {
                        let r = c.asl(v);
                        c.a = c.nz(c.a | r);
                        r
                    }
                    1 => {
                        let r = c.rol(v);
                        c.a = c.nz(c.a & r);
                        r
                    }
                    2 => {
                        let r = c.lsr(v);
                        c.a = c.nz(c.a ^ r);
                        r
                    }
                    3 => {
                        let r = c.ror(v);
                        c.adc(r);
                        r
                    }
                    6 => {
                        let r = v.wrapping_sub(1);
                        c.cmp(c.a, r);
                        r
                    }
                    _ => {
                        let r = v.wrapping_add(1);
                        c.sbc(r);
                        r
                    }
                });
            }
            // cc = 00
            (0, 0, 0) => {
                self.pc = self.pc.wrapping_add(1);
                self.interrupt(b, 0xFFFE, true);
            }
            (0, 0, 1) => {
                let target = self.fetch16(b);
                let [lo, hi] = self.pc.wrapping_sub(1).to_le_bytes();
                self.push(b, hi);
                self.push(b, lo);
                self.pc = target;
            }
            (0, 0, 2) => {
                self.p = (self.pull(b) | U) & !B;
                let lo = self.pull(b);
                self.pc = u16::from_le_bytes([lo, self.pull(b)]);
            }
            (0, 0, 3) => {
                let lo = self.pull(b);
                self.pc = u16::from_le_bytes([lo, self.pull(b)]).wrapping_add(1);
            }
            (0, 0, 4) => self.pc = self.pc.wrapping_add(1),
            (0, 1 | 3 | 5 | 7, 0 | 2 | 3) | (0, 5 | 7, 6 | 7) | (0, 5 | 7, 1) if !(bbb == 3 && aaa >= 2) => {
                // the NOPs that read (zp, zp,x, abs, abs,x)
                let mode = match bbb {
                    1 => Zp,
                    3 => Abs,
                    5 => Zpx,
                    _ => Absx,
                };
                let (a, x) = self.ea(b, mode);
                if x {
                    cyc += 1;
                }
                b.read(a);
            }
            (0, 2, _) => match aaa {
                0 => {
                    let p = self.p | B | U;
                    self.push(b, p);
                }
                1 => self.p = (self.pull(b) | U) & !B,
                2 => {
                    let a = self.a;
                    self.push(b, a);
                }
                3 => {
                    let v = self.pull(b);
                    self.a = self.nz(v);
                }
                4 => self.y = self.nz(self.y.wrapping_sub(1)),
                5 => self.y = self.nz(self.a),
                6 => self.y = self.nz(self.y.wrapping_add(1)),
                _ => self.x = self.nz(self.x.wrapping_add(1)),
            },
            (0, 3, 2) => self.pc = self.fetch16(b),
            (0, 3, 3) => {
                // JMP (ind), with the page wrap of the NMOS part
                let p = self.fetch16(b);
                let hi_at = (p & 0xFF00) | (p.wrapping_add(1) & 0x00FF);
                self.pc = u16::from_le_bytes([b.read(p), b.read(hi_at)]);
            }
            (0, 4, _) => {
                let off = self.fetch(b) as i8;
                let take = match aaa >> 1 {
                    0 => self.p & N != 0,
                    1 => self.p & V != 0,
                    2 => self.p & C != 0,
                    _ => self.p & Z != 0,
                } == (aaa & 1 != 0);
                if take {
                    let to = self.pc.wrapping_add(off as u16);
                    cyc += if to & 0xFF00 != self.pc & 0xFF00 { 2 } else { 1 };
                    self.pc = to;
                }
            }
            (0, 6, _) => match aaa {
                0 => self.p &= !C,
                1 => self.p |= C,
                2 => self.p &= !I,
                3 => self.p |= I,
                4 => self.a = self.nz(self.y),
                5 => self.p &= !V,
                6 => self.p &= !D,
                _ => self.p |= D,
            },
            (0, 7, 4) => {
                // SHY abs,x
                let (a, _) = self.ea(b, Absx);
                b.write(a, self.y & ((a >> 8) as u8).wrapping_add(1));
            }
            (0, _, 1) => {
                let v = rd!();
                self.flag(Z, self.a & v == 0);
                self.p = (self.p & !(N | V)) | (v & (N | V));
            }
            (0, _, 4) => {
                let (a, _) = self.ea(b, Self::mode(op));
                b.write(a, self.y);
            }
            (0, _, 5) => {
                let v = rd!();
                self.y = self.nz(v);
            }
            (0, _, 6) => {
                let v = rd!();
                self.cmp(self.y, v);
            }
            (0, _, _) => {
                let v = rd!();
                self.cmp(self.x, v);
            }
            _ => unreachable!("opcode {op:02X}"),
        }
        Ok(cyc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ram(Vec<u8>);
    impl Bus for Ram {
        fn read(&mut self, a: u16) -> u8 {
            self.0[usize::from(a)]
        }
        fn write(&mut self, a: u16, v: u8) {
            self.0[usize::from(a)] = v;
        }
    }

    /// Runs `code` at $0200 until it reaches a JAM; returns the CPU and memory.
    fn run(code: &[u8]) -> (Cpu, Ram) {
        let mut m = Ram(vec![0; 65536]);
        m.0[0x200..0x200 + code.len()].copy_from_slice(code);
        m.0[0x200 + code.len()] = 0x02;
        let mut c = Cpu { pc: 0x200, ..Cpu::default() };
        for _ in 0..10_000 {
            if c.step(&mut m).is_err() {
                return (c, m);
            }
        }
        panic!("no JAM");
    }

    #[test]
    fn arithmetic_and_flags() {
        // CLC LDA #$50 ADC #$50: $A0, V and N set, C clear
        let (c, _) = run(&[0x18, 0xA9, 0x50, 0x69, 0x50]);
        assert_eq!((c.a, c.p & (N | V | C | Z)), (0xA0, N | V));
        // SEC LDA #$00 SBC #$01: $FF, borrow (C clear), N
        let (c, _) = run(&[0x38, 0xA9, 0x00, 0xE9, 0x01]);
        assert_eq!((c.a, c.p & (N | C | Z)), (0xFF, N));
        // decimal: SED CLC LDA #$19 ADC #$28 -> $47; SEC LDA #$40 SBC #$01 -> $39
        let (c, _) = run(&[0xF8, 0x18, 0xA9, 0x19, 0x69, 0x28]);
        assert_eq!(c.a, 0x47);
        let (c, _) = run(&[0xF8, 0x38, 0xA9, 0x40, 0xE9, 0x01]);
        assert_eq!(c.a, 0x39);
        let (c, _) = run(&[0xF8, 0x18, 0xA9, 0x99, 0x69, 0x01]);
        assert_eq!((c.a, c.p & C), (0x00, C));
    }

    #[test]
    fn loops_stack_and_indirection() {
        // LDX #5; loop: DEX; BNE loop; LDY #3; STY $10; LDA #$00 STA $FB; LDA #$03 STA $FC;
        // LDY #1; LDA #$AA; STA ($FB),Y; JSR sub; JAM. sub: INC $0301; RTS
        let code = [
            0xA2, 0x05, 0xCA, 0xD0, 0xFD, 0xA0, 0x03, 0x84, 0x10, 0xA9, 0x00, 0x85, 0xFB, 0xA9, 0x03, 0x85, 0xFC, 0xA0,
            0x01, 0xA9, 0xAA, 0x91, 0xFB, 0x20, 0x1C, 0x02, 0x02, 0x00, 0xEE, 0x01, 0x03, 0x60,
        ];
        let (c, m) = run(&code);
        assert_eq!((c.x, m.0[0x10], m.0[0x301], c.sp), (0, 3, 0xAB, 0xFD));
    }

    #[test]
    fn undocumented_opcodes() {
        // LDA #$0F; STA $20; LAX $20 -> A = X = $0F; DCP $20 -> $0E, compares A ($0F) with it: C set
        let (c, m) = run(&[0xA9, 0x0F, 0x85, 0x20, 0xA7, 0x20, 0xC7, 0x20]);
        assert_eq!((c.a, c.x, m.0[0x20], c.p & C), (0x0F, 0x0F, 0x0E, C));
        // LDA #$F0; LDX #$3C; SAX $21 -> $30
        let (_, m) = run(&[0xA9, 0xF0, 0xA2, 0x3C, 0x87, 0x21]);
        assert_eq!(m.0[0x21], 0x30);
    }

    #[test]
    fn interrupts_and_rti() {
        let mut m = Ram(vec![0; 65536]);
        m.0[0xFFFE] = 0x00;
        m.0[0xFFFF] = 0x03; // handler at $0300: INC $40; RTI
        m.0[0x300..0x304].copy_from_slice(&[0xE6, 0x40, 0x40, 0x02]);
        let mut c = Cpu { pc: 0x1234, p: U, ..Cpu::default() };
        c.irq(&mut m);
        assert_eq!((c.pc, c.p & I), (0x300, I));
        c.step(&mut m).unwrap();
        c.step(&mut m).unwrap();
        assert_eq!((c.pc, c.p & I, m.0[0x40]), (0x1234, 0, 1));
    }
}
