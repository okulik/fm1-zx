// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use emu_gate::{build_dir, fnv1a, profile_bench, run_bench};
use zx_sys::Machine;

#[test]
#[ignore = "needs the fm1 build: set FMZX_FM1_BUILD and run with --ignored"]
fn redcode_z80_fits_the_frame_budget() {
    let r = run_bench(&build_dir(), "zx_bench", 2);
    println!("result words: {:08x?}", r.words);
    assert_eq!(r.words[1], 150);
    let mut m = Machine::new();
    for _ in 0..150 {
        m.run_frame();
    }
    assert_eq!(r.words[2], fnv1a((0x4000..0x5B00).map(|a| m.peek(a))), "device and host screens differ");
    assert_eq!(r.words[3], u32::from(m.regs().pc));
    let per_t = r.phases[0] as f64 / (150.0 * 69888.0);
    println!("pi32v2 instructions per Z80 T-state: {per_t:.2}");
    assert!(per_t < 30.0, "over the budget: {per_t:.2} instructions per T-state");
}

#[test]
#[ignore = "needs the fm1 build: set FMZX_FM1_BUILD and run with --ignored"]
fn profile_by_function() {
    let p = profile_bench(&build_dir(), "zx_bench");
    let total: u64 = p.iter().map(|x| x.1).sum();
    for (n, c) in p.iter().take(10) {
        println!("{c:>12} {:5.1}%  {n}", *c as f64 * 100.0 / total as f64);
    }
}
