// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs bare-metal pi32v2 images in fm1-emu and counts instructions.
//!
//! [`fw`] boots the real firmware image; the functions here run the bench images (`bench/fm1`).

pub mod budgets;
pub mod fw;
pub mod upgrade;

use fm1_core::{Bus, Cpu, JitMem};
use fm1_soc::{Soc, RAM_BASE, XIP_BASE};
use object::{Object, ObjectSymbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// RAM size given to the emulated SoC.
pub const RAM_SIZE: u32 = 1 << 20;
/// Address of the result block the bench image writes.
pub const RESULT_ADDR: u32 = 0x01C8_0000;
/// Value of word 0 once the result block is complete.
pub const MAGIC: u32 = 0x5A58_424E;
/// Step limit before `run_bench` gives up.
pub const MAX_STEPS: u64 = 2_000_000_000;

/// Outcome of a bench run.
pub struct Run {
    /// Instructions executed between successive visits to `bench_mark`.
    pub phases: Vec<u64>,
    /// The eight result words.
    pub words: [u32; 8],
}

/// FNV-1a, 32 bit.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u32 {
    bytes.into_iter().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// The build directory from `FMZX_FM1_BUILD` (absolute, or relative to the cwd).
pub fn build_dir() -> PathBuf {
    let v = std::env::var_os("FMZX_FM1_BUILD").expect("set FMZX_FM1_BUILD to the fm1 build directory");
    std::path::absolute(v).expect("absolute build dir")
}

/// A bus that forwards to the SoC and panics on a misaligned 16- or 32-bit RAM access, which
/// the FM-1 faults on. `pc` is the instruction's address, for the message.
pub struct CheckedBus<'a> {
    pub soc: &'a mut Soc,
    pub pc: u32,
}

impl CheckedBus<'_> {
    #[inline(always)]
    fn check(&self, addr: u32, size: u32, what: &str) {
        if !addr.is_multiple_of(size) {
            misaligned(self.pc, addr, what);
        }
    }
}

#[cold]
#[inline(never)]
fn misaligned(pc: u32, addr: u32, what: &str) {
    let in_ram = (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&addr);
    assert!(!in_ram, "pc {pc:#010x}: misaligned {what} at {addr:#010x}");
}

impl Bus for CheckedBus<'_> {
    fn read8(&mut self, a: u32) -> u8 {
        self.soc.read8(a)
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.check(a, 2, "read16");
        self.soc.read16(a)
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.check(a, 4, "read32");
        self.soc.read32(a)
    }
    fn fetch32(&mut self, a: u32) -> u32 {
        self.soc.fetch32(a)
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.soc.write8(a, v);
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.check(a, 2, "write16");
        self.soc.write16(a, v);
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.check(a, 4, "write32");
        self.soc.write32(a, v);
    }
    fn tick(&mut self) -> bool {
        self.soc.tick()
    }
    fn irq_pending(&self) -> bool {
        self.soc.irq_pending()
    }
    fn jit_mem(&mut self) -> Option<JitMem> {
        // No JIT: every access goes through the checks above.
        None
    }
    fn tick_room(&self) -> u64 {
        self.soc.tick_room()
    }
    fn add_ticks(&mut self, n: u64) {
        self.soc.add_ticks(n);
    }
    fn hw_lock(&mut self, core: u32, set: bool) -> bool {
        self.soc.hw_lock(core, set)
    }
    fn next_tick_due(&self) -> bool {
        self.soc.next_tick_due()
    }
    fn idle(&mut self, core: u32) {
        self.soc.idle(core);
    }
    fn irq_window(&mut self) {
        self.soc.irq_window();
    }
    fn service(&mut self) {
        self.soc.service();
    }
    fn code_writes(&self) -> Option<u64> {
        self.soc.code_writes()
    }
    fn code_mark(&mut self, lo: u32, hi: u32) {
        self.soc.code_mark(lo, hi);
    }
}

/// Executes one instruction through the alignment-checking bus and services the SoC clock.
pub fn step(cpu: &mut Cpu, soc: &mut Soc) {
    let mut bus = CheckedBus { soc, pc: cpu.pc };
    cpu.step(&mut bus).expect("cpu fault");
    if bus.tick() {
        soc.advance(0);
    }
}

/// Reads the result block.
pub fn result_words(soc: &Soc) -> [u32; 8] {
    let mut w = [0u32; 8];
    for (i, v) in w.iter_mut().enumerate() {
        *v = soc.ram.read32(RESULT_ADDR - RAM_BASE + 4 * i as u32);
    }
    w
}

fn symbol_addr(elf: &[u8], name: &str) -> u32 {
    let f = object::File::parse(elf).expect("parse ELF");
    let s = f.symbols().find(|s| s.name() == Ok(name)).unwrap_or_else(|| panic!("no symbol {name}"));
    s.address() as u32
}

/// Runs `<build>/bench/fm1/<name>.bin` until the result block is complete.
///
/// `marks` visits to `bench_mark` are recorded; `phases` holds the instruction counts between
/// consecutive visits (`marks - 1` entries).
pub fn run_bench(build: &Path, name: &str, marks: usize) -> Run {
    run_inner(build, name, marks, None)
}

/// Instructions executed per ELF function (symbol), most first, for the same run as `run_bench`.
pub fn profile_bench(build: &Path, name: &str) -> Vec<(String, u64)> {
    let mut hist = HashMap::new();
    run_inner(build, name, 2, Some(&mut hist));
    let dir = build.join("bench/fm1");
    let elf = std::fs::read(dir.join(format!("{name}.elf"))).expect("read ELF");
    let f = object::File::parse(&*elf).expect("parse ELF");
    let mut syms: Vec<(u32, u32, String)> = f
        .symbols()
        .filter(|s| s.kind() == object::SymbolKind::Text && s.size() > 0)
        .map(|s| (s.address() as u32, s.size() as u32, s.name().unwrap_or("?").to_string()))
        .collect();
    syms.sort();
    let mut by_name: HashMap<String, u64> = HashMap::new();
    for (pc, n) in hist {
        let i = syms.partition_point(|s| s.0 <= pc);
        let nm = if i > 0 && pc < syms[i - 1].0 + syms[i - 1].1 { syms[i - 1].2.clone() } else { format!("?{pc:#x}") };
        *by_name.entry(nm).or_default() += n;
    }
    let mut v: Vec<_> = by_name.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn run_inner(build: &Path, name: &str, marks: usize, mut hist: Option<&mut HashMap<u32, u64>>) -> Run {
    let dir = build.join("bench/fm1");
    let bin = std::fs::read(dir.join(format!("{name}.bin"))).expect("read bin");
    let elf = std::fs::read(dir.join(format!("{name}.elf"))).expect("read ELF");
    let mark = symbol_addr(&elf, "bench_mark");
    let mut soc = Soc::new(RAM_SIZE as usize, bin);
    let mut cpu = Cpu::new(XIP_BASE);
    let mut at = Vec::new();
    let mut steps = 0u64;
    loop {
        if cpu.pc == mark && at.len() < marks {
            at.push(steps);
        }
        if let Some(h) = hist.as_deref_mut() {
            *h.entry(cpu.pc).or_default() += 1;
        }
        step(&mut cpu, &mut soc);
        steps += 1;
        if steps.is_multiple_of(256) && result_words(&soc)[0] == MAGIC {
            break;
        }
        assert!(steps < MAX_STEPS, "bench did not finish in {MAX_STEPS} steps");
    }
    assert_eq!(at.len(), marks, "bench_mark visited {} times, expected {marks}", at.len());
    Run { phases: at.windows(2).map(|w| w[1] - w[0]).collect(), words: result_words(&soc) }
}
