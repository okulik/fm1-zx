// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Which code drives the speaker: every speaker edge carries the address of its `OUT`, and each
//! frame lists the distinct `OUT`s that made edges (`zx_out_sites`) with their registers.

use music::{capture, castaway, test_sna, RATE};
use zx_sys::{Beeper, Machine, ZX_OUT_SITES};

fn is_out(m: &Machine, pc: u16) -> bool {
    let op = m.peek(pc);
    op == 0xD3 || (op == 0xED && m.peek(pc.wrapping_add(1)) & 0xC7 == 0x41)
}

fn check_edges(name: &str, mut m: Machine, skip: usize, frames: usize) {
    let mut n = 0;
    for f in 0..skip + frames {
        m.run_frame();
        if f < skip {
            continue;
        }
        for e in m.edges() {
            assert!(is_out(&m, e.pc), "{name}: frame {f}: edge at T {} has pc {:04X}, not an OUT", e.tstate, e.pc);
            n += 1;
        }
    }
    assert!(n > 100, "{name}: {n} edges checked");
}

#[test]
fn edge_pc_is_an_out() {
    check_edges("tune", test_sna("tune.sna"), 0, 300);
    check_edges("border", test_sna("border.sna"), 0, 300);
    check_edges("castaway", castaway(), 400, 300);
}

#[test]
fn tune_one_site() {
    let mut m = test_sna("tune.sna");
    let mut with_edges = 0;
    for f in 0..300 {
        m.run_frame();
        let (n, sites) = (m.edges().len(), m.out_sites());
        if n == 0 {
            assert!(sites.is_empty(), "frame {f}: sites without edges");
            continue;
        }
        with_edges += 1;
        assert_eq!(sites.len(), 1, "frame {f}: {sites:?}");
        assert_eq!(sites[0].count as usize, n, "frame {f}");
        assert_eq!(sites[0].regs.pc, sites[0].pc, "frame {f}");
        assert!(m.edges().iter().all(|e| e.pc == sites[0].pc), "frame {f}");
    }
    assert!(with_edges > 150, "{with_edges} frames with edges");
}

#[test]
fn site_regs_at_the_out() {
    let mut m = test_sna("tune.sna");
    let mut checked = 0;
    for f in 0..300 {
        m.run_frame();
        let Some(last) = m.edges().last().copied() else { continue };
        let s = m.out_sites()[0];
        let a = (s.regs.af >> 8) as u8;
        assert_eq!((a >> 3) & 3, last.level, "frame {f}: A is the value the last edge's OUT wrote");
        assert_eq!(s.regs.sp, m.regs().sp, "frame {f}: the melody loop keeps SP");
        assert!(is_out(&m, s.pc));
        checked += 1;
    }
    assert!(checked > 150);
}

#[test]
fn sites_capped() {
    let mut m = Machine::new();
    // DI; then 6 x (LD A,n; OUT (0xFE),A) alternating speaker levels 1 and 2; JR $.
    let mut code = vec![0xF3u8];
    let mut outs = Vec::new();
    for i in 0..6u16 {
        code.extend([0x3E, if i % 2 == 0 { 0x08 } else { 0x10 }, 0xD3, 0xFE]);
        outs.push(0x8000 + 1 + 4 * i + 2);
    }
    code.extend([0x18, 0xFE]);
    for (i, b) in code.iter().enumerate() {
        m.poke(0x8000 + i as u16, *b);
    }
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m.run_frame();
    let edges = m.edges();
    assert_eq!(edges.len(), 6);
    assert_eq!(edges.iter().map(|e| e.pc).collect::<Vec<_>>(), outs);
    let sites = m.out_sites();
    assert_eq!(sites.len(), ZX_OUT_SITES as usize);
    assert_eq!(sites.iter().map(|s| s.pc).collect::<Vec<_>>(), outs[..4]);
    assert!(sites.iter().all(|s| s.count == 1 && s.regs.pc == s.pc));
    // The registers are those at each OUT, not at the frame's end: A alternates 0x08 / 0x10.
    for (i, s) in sites.iter().enumerate() {
        assert_eq!((s.regs.af >> 8) as u8, if i % 2 == 0 { 0x08 } else { 0x10 }, "site {i}");
    }
    assert_eq!((m.regs().af >> 8) as u8, 0x10, "the frame ends with A = 0x10, so a frame-end snapshot would fail");
}

fn run_loop(code: &[u8]) -> Machine {
    let mut m = Machine::new();
    for (i, b) in code.iter().enumerate() {
        m.poke(0x8000 + i as u16, *b);
    }
    let mut r = m.regs();
    r.pc = 0x8000;
    r.sp = 0xFF00;
    r.iff1 = 0;
    r.iff2 = 0;
    m.set_regs(&r);
    m
}

#[test]
fn pending_edge_carries_pc_and_regs() {
    // Loops of 4+4 T of OUTs plus n NOPs: some phase puts the frame end inside an OUT.
    let mut carried = 0;
    for n in 0..24u8 {
        let mut code = vec![0x3E, 0x08, 0xD3, 0xFE, 0x3E, 0x10, 0xD3, 0xFE];
        code.extend(std::iter::repeat(0u8).take(n as usize));
        code.extend([0x18, (-(10 + n as i32)) as u8]);
        carried += check_pending(run_loop(&code));
    }
    assert!(carried > 0, "some frame ends inside an OUT");
}

fn check_pending(mut m: Machine) -> usize {
    let mut carried = 0;
    for f in 0..200 {
        m.run_frame();
        let overshoot = m.frame_tstate();
        m.run_frame();
        let first = m.edges()[0];
        if first.tstate >= overshoot {
            continue;
        }
        carried += 1;
        let sites = m.out_sites();
        assert_eq!(sites[0].pc, first.pc, "frame {f}: the carried edge's OUT is the first site");
        assert!(sites[0].count >= 1 && sites[0].regs.pc == first.pc);
        let want_a = if first.pc == 0x8002 { 0x08 } else { 0x10 };
        assert_eq!((sites[0].regs.af >> 8) as u8, want_a, "frame {f}: registers at the OUT");
        assert_eq!(sites.iter().map(|s| s.count as usize).sum::<usize>(), m.edges().len());
    }
    carried
}

#[test]
fn snapshot_load_clears_sites() {
    let mut m = test_sna("tune.sna");
    for _ in 0..100 {
        m.run_frame();
    }
    assert!(!m.out_sites().is_empty());
    let data = std::fs::read(music::repo().join("games/test/border.sna")).unwrap();
    m.load_sna(&data).unwrap();
    assert!(m.edges().is_empty() && m.out_sites().is_empty());
}

fn fnv(frames: &[music::Frame]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for s in frames.iter().flat_map(|f| &f.beeper) {
        for b in s.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[test]
fn beeper_unchanged() {
    // Hashes computed at the commit before this change.
    let c = capture(&mut castaway(), &mut Beeper::new(RATE.0, RATE.1), 600);
    assert_eq!(fnv(&c), 0x16a3e06b31137849, "castaway title");
    let b = capture(&mut test_sna("border.sna"), &mut Beeper::new(RATE.0, RATE.1), 300);
    assert_eq!(fnv(&b), 0x63cb9bcd25fb19ff, "border.sna");
}
