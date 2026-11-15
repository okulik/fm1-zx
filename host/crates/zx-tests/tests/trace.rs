// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The host-only trace hook: instruction, memory and `OUT` events with the running contention
//! total, seen from a game's code without changing what it does.

use std::cell::RefCell;
use std::rc::Rc;

use music::test_sna;
use zx_sys::{TraceEv, ZX_TR_OUT, ZX_TR_READ};

#[test]
fn trace_sees_out_and_reads() {
    let mut m = test_sna("tune.sna");
    let seen: Rc<RefCell<Vec<TraceEv>>> = Rc::default();
    let sink = seen.clone();
    m.set_trace(Some(Box::new(move |e| sink.borrow_mut().push(*e))), 1 << ZX_TR_OUT | 1 << ZX_TR_READ);
    let mut edges = 0;
    for _ in 0..10 {
        m.run_frame();
        edges += m.edges().len();
    }
    let seen = seen.borrow();
    let outs: Vec<_> = seen.iter().filter(|e| e.kind == ZX_TR_OUT as u8).collect();
    assert!(outs.len() > 100, "{} OUTs", outs.len());
    assert!(outs.iter().all(|e| e.pc == 0x8014 && e.addr & 0xFF == 0xFE), "OUT outside 8014h");
    assert_eq!(outs.len(), edges, "OUT events vs speaker edges");
    let ix2: Vec<_> = seen.iter().filter(|e| e.kind == ZX_TR_READ as u8 && e.pc == 0x801B).collect();
    assert!(!ix2.is_empty());
    // The operand fetches and the data read are all READs of this instruction; the data read
    // is the last one, at IX+2 (IX is read from the registers at the previous OUT).
    let out_ix = outs.last().unwrap().regs.ix;
    assert!(ix2.iter().any(|e| e.addr == out_ix.wrapping_add(2)), "no read at IX+2");
}

#[test]
fn trace_off_changes_nothing() {
    let run = |hook: bool| {
        let mut m = test_sna("tune.sna");
        if hook {
            m.set_trace(Some(Box::new(|_| {})), 0xFF);
        }
        let mut edges = Vec::new();
        for _ in 0..200 {
            m.run_frame();
            edges.extend(m.edges().iter().map(|e| (e.tstate, e.level, e.pc)));
        }
        let ram: Vec<u8> = (0x4000..=0xFFFFu32).map(|a| m.peek(a as u16)).collect();
        (edges, ram)
    };
    let a = run(false);
    let b = run(true);
    assert!(a.0.len() > 1000);
    assert!(a == b, "a hook changed the run");
}
