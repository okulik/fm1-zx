// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Z80 decoder against the real core: every opcode form's length, T-states and branch
//! targets are checked by running it once in `zx-sys`'s `Machine`.

use soundlearn::z80::{decode, Flow, PortRef};
use zx_sys::{zx_regs, Machine};

const PC: u16 = 0x8000;
const PTR: u16 = 0xC000;
/// A frame T-state past the contended screen period.
const FRAME_T: u32 = 60000;

/// Every instruction form the core can run alone: the unprefixed, CB, ED, DD, FD and DDCB/FDCB
/// opcode sequences. HALT is left out (it never leaves its own address).
fn sequences() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for op in 0..=255u8 {
        if op == 0x76 || matches!(op, 0xCB | 0xDD | 0xED | 0xFD) {
            continue;
        }
        v.push(vec![op]);
    }
    for op in 0..=255u8 {
        v.push(vec![0xCB, op]);
        v.push(vec![0xED, op]);
    }
    for pre in [0xDD, 0xFD] {
        for op in 0..=255u8 {
            if op == 0x76 || op == 0xCB {
                continue;
            }
            v.push(vec![pre, op]);
        }
        for op in 0..=255u8 {
            v.push(vec![pre, 0xCB, 0x10, op]);
        }
    }
    v
}

fn machine_with(seq: &[u8], f: u16, bc: u16) -> Machine {
    let mut m = Machine::new();
    // The tail bytes make nn = 1010h and every displacement and offset +10h.
    for i in 0..8 {
        m.poke(PC + i, seq.get(i as usize).copied().unwrap_or(0x10));
    }
    for a in 0..16 {
        m.poke(PTR + a, 0x55);
    }
    m.set_regs(&zx_regs {
        af: f,
        bc,
        de: PTR,
        hl: PTR,
        af_: 0,
        bc_: 0,
        de_: 0,
        hl_: 0,
        ix: PTR,
        iy: PTR,
        sp: 0xF000,
        pc: PC,
        i: 0,
        r: 0,
        iff1: 0,
        iff2: 0,
        im: 0,
    });
    m.test_set_tstate(FRAME_T);
    m
}

/// Flags all clear and all set, and the three counts that make every loop end or repeat.
fn configs() -> Vec<(u16, u16)> {
    let mut v = Vec::new();
    for f in [0x0000, 0x00FF] {
        for bc in [PTR, 0x0001, 0x0100] {
            v.push((f, bc));
        }
    }
    v
}

#[test]
fn timing_matches_core() {
    let mut forms = 0;
    for seq in sequences() {
        // A lone prefix is folded into the next instruction by the core's step; those forms
        // are checked separately.
        if matches!(seq.get(1), Some(0xDD | 0xED | 0xFD)) && matches!(seq[0], 0xDD | 0xFD) {
            continue;
        }
        forms += 1;
        let mut taken_seen = false;
        let mut skipped_seen = false;
        let mut has_branch = false;
        for (f, bc) in configs() {
            let mut m = machine_with(&seq, f, bc);
            let ins = decode(|a| m.peek(a), PC);
            let t = m.test_step();
            let after = m.regs().pc;
            let next = PC.wrapping_add(ins.len as u16);
            let took = after != next;
            let want = if took { ins.t_taken } else { ins.t };
            assert_eq!(t, want as u32, "T-states of {seq:02X?} ({}) f={f:04X} bc={bc:04X}", ins.text);
            if ins.t != ins.t_taken {
                has_branch = true;
                if took { taken_seen = true } else { skipped_seen = true }
            }
            if took {
                match ins.flow {
                    Flow::Jump(a) | Flow::CondJump(a) | Flow::Djnz(a) | Flow::Call(a) | Flow::CondCall(a) | Flow::Rst(a) => {
                        assert_eq!(after, a, "target of {seq:02X?} ({})", ins.text)
                    }
                    Flow::JumpIndirect => assert_eq!(after, PTR, "{}", ins.text),
                    Flow::Ret | Flow::CondRet => assert_eq!(after, 0, "{}", ins.text),
                    // A repeating block instruction stays on itself.
                    Flow::Next => assert_eq!(after, PC, "{}", ins.text),
                    Flow::Halt => unreachable!(),
                }
            }
        }
        if has_branch {
            assert!(taken_seen && skipped_seen, "{seq:02X?} did not run both ways");
        }
    }
    // The generated set is the whole instruction space (minus HALT and the lone prefixes).
    assert!(forms > 1500, "{forms} forms");
}

#[test]
fn lengths_match_core() {
    for seq in sequences() {
        if matches!(seq.get(1), Some(0xDD | 0xED | 0xFD)) && matches!(seq[0], 0xDD | 0xFD) {
            continue;
        }
        let mut m = machine_with(&seq, 0, 0x0001);
        let ins = decode(|a| m.peek(a), PC);
        assert_eq!(ins.pc, PC);
        assert!(ins.len >= 1 && ins.len <= 4, "{seq:02X?}");
        m.test_step();
        if ins.flow == Flow::Next {
            // A block instruction may repeat, which leaves the PC on itself.
            let after = m.regs().pc;
            let repeat = ins.t != ins.t_taken && after == PC;
            assert!(after == PC + ins.len as u16 || repeat, "length of {seq:02X?} ({})", ins.text);
        }
    }
}

/// A prefix before DD, ED or FD does nothing and is one byte; the core folds it into the next
/// instruction, so the pair's time is the prefix's 4 T-states plus the next one's.
#[test]
fn lone_prefix() {
    for pre in [0xDDu8, 0xFD] {
        for op in 0..=255u8 {
            let seq = [pre, 0xED, op];
            let mut m = machine_with(&seq, 0, 0x0001);
            let first = decode(|a| m.peek(a), PC);
            assert_eq!((first.len, first.t, first.t_taken, first.flow), (1, 4, 4, Flow::Next));
            let second = decode(|a| m.peek(a), PC + 1);
            if second.t == second.t_taken {
                assert_eq!(m.test_step(), 4 + second.t as u32, "{pre:02X} ED {op:02X}");
            }
        }
        let m = machine_with(&[pre, pre, 0x00], 0, 0);
        assert_eq!(decode(|a| m.peek(a), PC).len, 1);
    }
}

#[test]
fn out_ports() {
    let ins = decode(|a| [0xD3, 0xFE][(a - PC) as usize], PC);
    assert_eq!(ins.port_out, Some(PortRef::Imm(0xFE)));
    assert_eq!((ins.len, ins.t), (2, 11));
    assert_eq!(ins.text, "OUT (FEh),A");
    let ins = decode(|a| [0xED, 0x79][(a - PC) as usize], PC);
    assert_eq!(ins.port_out, Some(PortRef::RegC));
    assert_eq!(ins.text, "OUT (C),A");
    // Other instructions write no port.
    let ins = decode(|_| 0x00, PC);
    assert_eq!(ins.port_out, None);
}

#[test]
fn text_and_registers() {
    let d = |bytes: &[u8]| {
        let b = bytes.to_vec();
        decode(move |a| b.get(a.wrapping_sub(PC) as usize).copied().unwrap_or(0), PC)
    };
    assert_eq!(d(&[0xDD, 0x46, 0x02]).text, "LD B,(IX+02h)");
    assert_eq!(d(&[0x10, 0x1F]).text, "DJNZ 8021h");
    assert_eq!(d(&[0x10, 0xFE]).flow, Flow::Djnz(PC));
    assert_eq!(d(&[0xDD, 0x7C]).text, "LD A,IXH");
    assert_eq!(d(&[0xCB, 0x30]).text, "SLL B");
    // INC B reads and writes B (2), and so BC (8), and the flags are written.
    let inc = d(&[0x04]);
    assert!(inc.reads & (1 << 2) != 0 && inc.writes & (1 << 2) != 0 && inc.writes & (1 << 8) != 0);
    assert!(inc.writes & (1 << 1) != 0);
    // JR NZ reads the flags as a condition.
    let jr = d(&[0x20, 0x05]);
    assert_eq!(jr.flow, Flow::CondJump(PC + 7));
    assert!(jr.reads & (1 << 20) != 0);
    assert_eq!(d(&[0xC7]).flow, Flow::Rst(0));
    assert_eq!(d(&[0xCD, 0x34, 0x12]).flow, Flow::Call(0x1234));
    assert_eq!(d(&[0x76]).flow, Flow::Halt);
}
