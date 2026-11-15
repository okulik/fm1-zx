// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A Z80 instruction decoder for the learner: for the instruction at an address it gives the
//! length, the T-states (no contention), where control goes, the registers read and written,
//! the memory and port the instruction touches, and a disassembly.
//!
//! Registers are the numbers of [`crate::prog`]: 0 A, 1 F, 2 B, 3 C, 4 D, 5 E, 6 H, 7 L, 8 BC,
//! 9 DE, 10 HL, 11 IX, 12 IY, 13 SP, 14 A' (stands for AF'), 15 BC', 16 DE', 17 HL'. An 8-bit
//! register and the pair that holds it alias: touching B also marks BC, touching BC also marks B
//! and C. The halves of IX and IY (undocumented) mark IX or IY. Bit 1 (F) is a flag use as data
//! (ADC, RLA, PUSH AF); bit [`COND`] is the flags read as a condition (JR NZ, RET C).

/// The flags as a condition, in [`Insn::reads`].
pub const COND: u32 = 1 << 20;

/// Where control goes after the instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Next,
    Jump(u16),
    CondJump(u16),
    Djnz(u16),
    Call(u16),
    CondCall(u16),
    Ret,
    CondRet,
    JumpIndirect,
    Rst(u16),
    Halt,
}

/// A memory operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemRef {
    /// A fixed address.
    Abs(u16),
    /// The address in register 8-10 (BC, DE, HL).
    Reg16(u8),
    /// Register 11 or 12 (IX, IY) plus a displacement.
    Indexed(u8, i8),
    /// The stack top.
    Sp,
}

/// The port an OUT writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortRef {
    /// OUT (n),A: the low byte; A is the high byte.
    Imm(u8),
    /// OUT (C),r and the block outputs: BC.
    RegC,
}

/// One decoded instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    pub pc: u16,
    pub len: u8,
    /// T-states of the not-taken or only path.
    pub t: u8,
    /// T-states of the taken path (the repeating form of a block instruction).
    pub t_taken: u8,
    pub flow: Flow,
    pub reads: u32,
    pub writes: u32,
    pub mem_read: Option<MemRef>,
    pub mem_write: Option<MemRef>,
    pub port_out: Option<PortRef>,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Idx {
    X,
    Y,
}

impl Idx {
    fn reg(self) -> u8 {
        match self {
            Idx::X => 11,
            Idx::Y => 12,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Idx::X => "IX",
            Idx::Y => "IY",
        }
    }
}

/// The register bits of register `r`, with the aliases between a pair and its halves.
fn rb(r: u8) -> u32 {
    match r {
        2..=7 => (1 << r) | (1 << (8 + (r - 2) / 2)),
        8..=10 => (1 << r) | (3 << (2 + 2 * (r - 8))),
        _ => 1 << r,
    }
}

const AF: u32 = 3;
const F: u32 = 1 << 1;
const SP: u32 = 1 << 13;

const R8: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
const R8N: [u8; 8] = [2, 3, 4, 5, 6, 7, 255, 0];
const RP: [&str; 4] = ["BC", "DE", "HL", "SP"];
const RP2: [&str; 4] = ["BC", "DE", "HL", "AF"];
const CC: [&str; 8] = ["NZ", "Z", "NC", "C", "PO", "PE", "P", "M"];
const ALU: [&str; 8] = ["ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP "];
const ROT: [&str; 8] = ["RLC", "RRC", "RL", "RR", "SLA", "SRA", "SLL", "SRL"];
const BLOCK: [[&str; 4]; 4] = [
    ["LDI", "LDD", "LDIR", "LDDR"],
    ["CPI", "CPD", "CPIR", "CPDR"],
    ["INI", "IND", "INIR", "INDR"],
    ["OUTI", "OUTD", "OTIR", "OTDR"],
];

fn h8(v: u8) -> String {
    format!("{v:02X}h")
}
fn h16(v: u16) -> String {
    format!("{v:04X}h")
}

fn disp(d: i8) -> String {
    format!("{}{:02X}h", if d < 0 { '-' } else { '+' }, d.unsigned_abs())
}

struct Cur<'a> {
    peek: &'a dyn Fn(u16) -> u8,
    pc: u16,
    n: u16,
}

impl Cur<'_> {
    fn byte(&mut self) -> u8 {
        let b = (self.peek)(self.pc.wrapping_add(self.n));
        self.n += 1;
        b
    }
    fn word(&mut self) -> u16 {
        let lo = self.byte() as u16;
        lo | (self.byte() as u16) << 8
    }
    fn rel(&mut self) -> u16 {
        let d = self.byte() as i8;
        self.pc.wrapping_add(self.n).wrapping_add(d as u16)
    }
}

/// An 8-bit operand: its name, register (None for memory) and memory reference.
struct Opd {
    text: String,
    reg: Option<u8>,
    mem: Option<MemRef>,
}

/// Operand `n` (0-7, the r field). Memory is (HL) or, with an index prefix, (IX+d) (reads the
/// displacement); the H and L of an index prefix are IXH and IXL unless `plain`.
fn opd(c: &mut Cur, n: u8, idx: Option<Idx>, plain: bool) -> Opd {
    match (n, idx) {
        (6, None) => Opd { text: "(HL)".into(), reg: None, mem: Some(MemRef::Reg16(10)) },
        (6, Some(x)) => {
            let d = c.byte() as i8;
            Opd { text: format!("({}{})", x.name(), disp(d)), reg: None, mem: Some(MemRef::Indexed(x.reg(), d)) }
        }
        (4 | 5, Some(x)) if !plain => {
            let h = if n == 4 { "H" } else { "L" };
            Opd { text: format!("{}{h}", x.name()), reg: Some(x.reg()), mem: None }
        }
        _ => Opd { text: R8[n as usize].into(), reg: Some(R8N[n as usize]), mem: None },
    }
}

fn mem_base(m: MemRef) -> u32 {
    match m {
        MemRef::Reg16(r) | MemRef::Indexed(r, _) => rb(r),
        MemRef::Abs(_) => 0,
        MemRef::Sp => SP,
    }
}

impl Insn {
    fn rd(&mut self, o: &Opd) {
        if let Some(r) = o.reg {
            self.reads |= rb(r);
        }
        if let Some(m) = o.mem {
            self.mem_read = Some(m);
            self.reads |= mem_base(m);
        }
    }
    fn wr(&mut self, o: &Opd) {
        if let Some(r) = o.reg {
            self.writes |= rb(r);
        }
        if let Some(m) = o.mem {
            self.mem_write = Some(m);
            self.reads |= mem_base(m);
        }
    }
    fn rw(&mut self, o: &Opd) {
        self.rd(o);
        self.wr(o);
    }
    fn times(&mut self, t: u8, taken: u8) {
        self.t = t;
        self.t_taken = taken;
    }
    fn time(&mut self, t: u8) {
        self.times(t, t);
    }
    fn read_mem(&mut self, m: MemRef) {
        self.mem_read = Some(m);
        self.reads |= mem_base(m);
    }
    fn write_mem(&mut self, m: MemRef) {
        self.mem_write = Some(m);
        self.reads |= mem_base(m);
    }
    fn push_effect(&mut self) {
        self.mem_write = Some(MemRef::Sp);
        self.reads |= SP;
        self.writes |= SP;
    }
    fn pop_effect(&mut self) {
        self.mem_read = Some(MemRef::Sp);
        self.reads |= SP;
        self.writes |= SP;
    }
}

/// Decodes the instruction at `pc`, reading its bytes with `peek`.
pub fn decode(peek: impl Fn(u16) -> u8, pc: u16) -> Insn {
    let mut c = Cur { peek: &peek, pc, n: 0 };
    let mut i = Insn {
        pc,
        len: 0,
        t: 4,
        t_taken: 4,
        flow: Flow::Next,
        reads: 0,
        writes: 0,
        mem_read: None,
        mem_write: None,
        port_out: None,
        text: String::new(),
    };
    let op = c.byte();
    match op {
        0xCB => cb(&mut c, &mut i, None),
        0xED => ed(&mut c, &mut i),
        0xDD | 0xFD => {
            let x = if op == 0xDD { Idx::X } else { Idx::Y };
            let next = (peek)(pc.wrapping_add(1));
            if matches!(next, 0xDD | 0xED | 0xFD) {
                i.text = "NOP".into(); // the prefix alone, 4 T-states
            } else {
                let o = c.byte();
                if o == 0xCB {
                    cb(&mut c, &mut i, Some(x));
                } else {
                    main(&mut c, &mut i, o, Some(x));
                    i.t += 4;
                    i.t_taken += 4;
                }
            }
        }
        _ => main(&mut c, &mut i, op, None),
    }
    i.len = c.n as u8;
    i
}

/// The unprefixed instruction set; `idx` is the DD or FD prefix in front of it.
fn main(c: &mut Cur, i: &mut Insn, op: u8, idx: Option<Idx>) {
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let (p, q) = (y >> 1, y & 1);
    let hl = idx.map_or(10, Idx::reg);
    let hln = idx.map_or("HL", Idx::name);
    // The HL slot of a register-pair field: HL, IX or IY.
    let rpn = |k: u8| if k == 2 { hln } else { RP[k as usize] };
    let rpr = |k: u8| if k == 2 { hl } else { [8, 9, 10, 13][k as usize] };
    let rp2n = |k: u8| if k == 2 { hln } else { RP2[k as usize] };
    // Extra T-states of an (IX+d) operand over (HL).
    let ix_extra = if idx.is_some() { 8 } else { 0 };
    match x {
        0 => match z {
            0 => match y {
                0 => i.text = "NOP".into(),
                1 => {
                    i.text = "EX AF,AF'".into();
                    i.reads = AF | 1 << 14;
                    i.writes = AF | 1 << 14;
                }
                2 => {
                    let t = c.rel();
                    i.flow = Flow::Djnz(t);
                    i.times(8, 13);
                    i.reads = rb(2);
                    i.writes = rb(2);
                    i.text = format!("DJNZ {}", h16(t));
                }
                3 => {
                    let t = c.rel();
                    i.flow = Flow::Jump(t);
                    i.time(12);
                    i.text = format!("JR {}", h16(t));
                }
                _ => {
                    let t = c.rel();
                    i.flow = Flow::CondJump(t);
                    i.times(7, 12);
                    i.reads = COND;
                    i.text = format!("JR {},{}", CC[(y - 4) as usize], h16(t));
                }
            },
            1 => {
                if q == 0 {
                    let nn = c.word();
                    i.time(10);
                    i.writes = rb(rpr(p));
                    i.text = format!("LD {},{}", rpn(p), h16(nn));
                } else {
                    i.time(11);
                    i.reads = rb(hl) | rb(rpr(p));
                    i.writes = rb(hl) | F;
                    i.text = format!("ADD {hln},{}", rpn(p));
                }
            }
            2 => {
                i.time([7, 7, 16, 13][p as usize]);
                let st = q == 0;
                let (mem, text, reg_bits): (MemRef, String, u32) = match p {
                    0 => (MemRef::Reg16(8), "(BC)".into(), 1),
                    1 => (MemRef::Reg16(9), "(DE)".into(), 1),
                    _ => {
                        let nn = c.word();
                        (MemRef::Abs(nn), format!("({})", h16(nn)), if p == 2 { rb(hl) } else { 1 })
                    }
                };
                let rname = if p == 2 { hln } else { "A" };
                if st {
                    i.write_mem(mem);
                    i.reads |= reg_bits;
                    i.text = format!("LD {text},{rname}");
                } else {
                    i.read_mem(mem);
                    i.writes |= reg_bits;
                    i.text = format!("LD {rname},{text}");
                }
            }
            3 => {
                i.time(6);
                i.reads = rb(rpr(p));
                i.writes = rb(rpr(p));
                i.text = format!("{} {}", if q == 0 { "INC" } else { "DEC" }, rpn(p));
            }
            4 | 5 => {
                let o = opd(c, y, idx, false);
                i.rw(&o);
                i.writes |= F;
                i.time(if o.mem.is_some() { 11 + ix_extra } else { 4 });
                i.text = format!("{} {}", if z == 4 { "INC" } else { "DEC" }, o.text);
            }
            6 => {
                let o = opd(c, y, idx, false);
                let n = c.byte();
                i.wr(&o);
                i.time(if o.mem.is_some() { 10 + ix_extra.saturating_sub(3) } else { 7 });
                i.text = format!("LD {},{}", o.text, h8(n));
            }
            _ => {
                i.time(4);
                let name = ["RLCA", "RRCA", "RLA", "RRA", "DAA", "CPL", "SCF", "CCF"][y as usize];
                i.text = name.into();
                match y {
                    0 | 1 => (i.reads, i.writes) = (1, 1 | F),
                    2..=4 => (i.reads, i.writes) = (1 | F, 1 | F),
                    5 => (i.reads, i.writes) = (1, 1 | F),
                    6 => i.writes = F,
                    _ => (i.reads, i.writes) = (F, F),
                }
            }
        },
        1 => {
            if op == 0x76 {
                i.flow = Flow::Halt;
                i.text = "HALT".into();
                return;
            }
            // With an index prefix, (IX+d) takes the other operand's plain H or L.
            let dst = opd(c, y, idx, z == 6);
            let src = opd(c, z, idx, y == 6);
            i.wr(&dst);
            i.rd(&src);
            i.time(if dst.mem.is_some() || src.mem.is_some() { 7 + ix_extra } else { 4 });
            i.text = format!("LD {},{}", dst.text, src.text);
        }
        2 => {
            let o = opd(c, z, idx, false);
            i.rd(&o);
            alu(i, y);
            i.time(if o.mem.is_some() { 7 + ix_extra } else { 4 });
            i.text = format!("{}{}", ALU[y as usize], o.text);
        }
        _ => match z {
            0 => {
                i.flow = Flow::CondRet;
                i.times(5, 11);
                i.reads = COND;
                i.pop_effect();
                i.text = format!("RET {}", CC[y as usize]);
            }
            1 => match (q, p) {
                (0, _) => {
                    i.time(10);
                    i.pop_effect();
                    i.writes |= if p == 3 { AF } else { rb([8, 9, hl][p as usize]) };
                    i.text = format!("POP {}", rp2n(p));
                }
                (_, 0) => {
                    i.flow = Flow::Ret;
                    i.time(10);
                    i.pop_effect();
                    i.text = "RET".into();
                }
                (_, 1) => {
                    i.text = "EXX".into();
                    let s = rb(8) | rb(9) | rb(10) | rb(15) | rb(16) | rb(17);
                    i.reads = s;
                    i.writes = s;
                }
                (_, 2) => {
                    i.flow = Flow::JumpIndirect;
                    i.time(4);
                    i.reads = rb(hl);
                    i.text = format!("JP ({hln})");
                }
                _ => {
                    i.time(6);
                    i.reads = rb(hl);
                    i.writes = SP;
                    i.text = format!("LD SP,{hln}");
                }
            },
            2 => {
                let nn = c.word();
                i.flow = Flow::CondJump(nn);
                i.time(10);
                i.reads = COND;
                i.text = format!("JP {},{}", CC[y as usize], h16(nn));
            }
            3 => match y {
                0 => {
                    let nn = c.word();
                    i.flow = Flow::Jump(nn);
                    i.time(10);
                    i.text = format!("JP {}", h16(nn));
                }
                2 => {
                    let n = c.byte();
                    i.time(11);
                    i.reads = 1;
                    i.port_out = Some(PortRef::Imm(n));
                    i.text = format!("OUT ({}),A", h8(n));
                }
                3 => {
                    let n = c.byte();
                    i.time(11);
                    i.reads = 1;
                    i.writes = 1;
                    i.text = format!("IN A,({})", h8(n));
                }
                4 => {
                    i.time(19);
                    i.pop_effect();
                    i.mem_write = Some(MemRef::Sp);
                    i.reads |= rb(hl);
                    i.writes |= rb(hl);
                    i.text = format!("EX (SP),{hln}");
                }
                5 => {
                    // EX DE,HL ignores an index prefix.
                    i.reads = rb(9) | rb(10);
                    i.writes = rb(9) | rb(10);
                    i.text = "EX DE,HL".into();
                }
                6 => i.text = "DI".into(),
                _ => i.text = "EI".into(),
            },
            4 => {
                let nn = c.word();
                i.flow = Flow::CondCall(nn);
                i.times(10, 17);
                i.reads = COND;
                i.push_effect();
                i.text = format!("CALL {},{}", CC[y as usize], h16(nn));
            }
            5 => {
                if q == 0 {
                    i.time(11);
                    i.push_effect();
                    i.reads |= if p == 3 { AF } else { rb([8, 9, hl][p as usize]) };
                    i.text = format!("PUSH {}", rp2n(p));
                } else {
                    let nn = c.word();
                    i.flow = Flow::Call(nn);
                    i.time(17);
                    i.push_effect();
                    i.text = format!("CALL {}", h16(nn));
                }
            }
            6 => {
                let n = c.byte();
                i.reads = 1;
                alu(i, y);
                i.time(7);
                i.text = format!("{}{}", ALU[y as usize], h8(n));
            }
            _ => {
                i.flow = Flow::Rst(y as u16 * 8);
                i.time(11);
                i.push_effect();
                i.text = format!("RST {}", h8(y * 8));
            }
        },
    }
}

/// An ALU operation `y` on A: the flags are written, A is written except by CP, and ADC and SBC
/// read the carry. The operand's own reads are already in `i`.
fn alu(i: &mut Insn, y: u8) {
    i.reads |= 1;
    i.writes |= F;
    if y != 7 {
        i.writes |= 1;
    }
    if y == 1 || y == 3 {
        i.reads |= F;
    }
}

/// CB-prefixed, or DD CB d op when `idx` is set (the displacement comes before the opcode).
fn cb(c: &mut Cur, i: &mut Insn, idx: Option<Idx>) {
    let d = idx.map(|_| c.byte() as i8);
    let op = c.byte();
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let o = match (idx, d) {
        (Some(ix), Some(d)) => Opd {
            text: format!("({}{})", ix.name(), disp(d)),
            reg: None,
            mem: Some(MemRef::Indexed(ix.reg(), d)),
        },
        _ => opd(c, z, None, true),
    };
    let mem = o.mem.is_some();
    if x == 1 {
        i.rd(&o);
        i.writes |= F;
        i.time(match (idx.is_some(), mem) {
            (true, _) => 20,
            (_, true) => 12,
            _ => 8,
        });
        i.text = format!("BIT {y},{}", o.text);
        return;
    }
    i.rw(&o);
    i.time(match (idx.is_some(), mem) {
        (true, _) => 23,
        (_, true) => 15,
        _ => 8,
    });
    if x == 0 {
        i.writes |= F;
        if y == 2 || y == 3 {
            i.reads |= F;
        }
        i.text = format!("{} {}", ROT[y as usize], o.text);
    } else {
        i.text = format!("{} {y},{}", if x == 2 { "RES" } else { "SET" }, o.text);
    }
    // The undocumented forms of DD CB d op: the result is also copied to register z.
    if idx.is_some() && z != 6 {
        let r = Opd { text: R8[z as usize].into(), reg: Some(R8N[z as usize]), mem: None };
        i.wr(&r);
        i.text = format!("{},{}", i.text, r.text);
    }
}

fn ed(c: &mut Cur, i: &mut Insn) {
    let op = c.byte();
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let (p, q) = (y >> 1, y & 1);
    i.time(8);
    i.text = "NOP".into();
    if x == 2 && z < 4 && y >= 4 {
        block(i, z, y - 4);
        return;
    }
    if x != 1 {
        return;
    }
    let rp = [8u8, 9, 10, 13][p as usize];
    match z {
        0 => {
            i.time(12);
            i.reads = rb(8);
            i.writes = F;
            if y == 6 {
                i.text = "IN F,(C)".into();
            } else {
                i.writes |= rb(R8N[y as usize]);
                i.text = format!("IN {},(C)", R8[y as usize]);
            }
        }
        1 => {
            i.time(12);
            i.reads = rb(8);
            i.port_out = Some(PortRef::RegC);
            if y == 6 {
                i.text = "OUT (C),0".into();
            } else {
                i.reads |= rb(R8N[y as usize]);
                i.text = format!("OUT (C),{}", R8[y as usize]);
            }
        }
        2 => {
            i.time(15);
            i.reads = rb(10) | rb(rp) | F;
            i.writes = rb(10) | F;
            i.text = format!("{} HL,{}", if q == 0 { "SBC" } else { "ADC" }, RP[p as usize]);
        }
        3 => {
            i.time(20);
            let nn = c.word();
            if q == 0 {
                i.write_mem(MemRef::Abs(nn));
                i.reads |= rb(rp);
                i.text = format!("LD ({}),{}", h16(nn), RP[p as usize]);
            } else {
                i.read_mem(MemRef::Abs(nn));
                i.writes |= rb(rp);
                i.text = format!("LD {},({})", RP[p as usize], h16(nn));
            }
        }
        4 => {
            i.reads = 1;
            i.writes = 1 | F;
            i.text = "NEG".into();
        }
        5 => {
            i.flow = Flow::Ret;
            i.time(14);
            i.pop_effect();
            i.text = if y == 1 { "RETI" } else { "RETN" }.into();
        }
        6 => i.text = format!("IM {}", [0, 0, 1, 2, 0, 0, 1, 2][y as usize]),
        _ => match y {
            0 => {
                i.time(9);
                i.reads = 1;
                i.text = "LD I,A".into();
            }
            1 => {
                i.time(9);
                i.reads = 1;
                i.text = "LD R,A".into();
            }
            2 | 3 => {
                i.time(9);
                i.writes = 1 | F;
                i.text = if y == 2 { "LD A,I" } else { "LD A,R" }.into();
            }
            4 | 5 => {
                i.time(18);
                i.read_mem(MemRef::Reg16(10));
                i.write_mem(MemRef::Reg16(10));
                i.reads |= 1;
                i.writes |= 1 | F;
                i.text = if y == 4 { "RRD" } else { "RLD" }.into();
            }
            _ => {}
        },
    }
}

/// The block instructions: `kind` 0 LD, 1 CP, 2 IN, 3 OUT; `col` 0 increasing, 1 decreasing,
/// 2 increasing and repeating, 3 decreasing and repeating (the ED A0..BB layout).
fn block(i: &mut Insn, kind: u8, col: u8) {
    let hl = rb(10);
    i.times(16, if col >= 2 { 21 } else { 16 });
    i.text = BLOCK[kind as usize][col as usize].into();
    match kind {
        0 => {
            i.read_mem(MemRef::Reg16(10));
            i.write_mem(MemRef::Reg16(9));
            i.reads |= rb(8) | rb(9);
            i.writes = hl | rb(9) | rb(8) | F;
        }
        1 => {
            i.read_mem(MemRef::Reg16(10));
            i.reads |= 1 | rb(8);
            i.writes = hl | rb(8) | F;
        }
        2 => {
            i.write_mem(MemRef::Reg16(10));
            i.reads |= rb(8);
            i.writes = hl | rb(2) | F;
        }
        _ => {
            i.read_mem(MemRef::Reg16(10));
            i.port_out = Some(PortRef::RegC);
            i.reads |= rb(8);
            i.writes = hl | rb(2) | F;
        }
    }
}
