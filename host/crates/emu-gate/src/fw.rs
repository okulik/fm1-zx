// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Boots the real firmware image (`build/fm1/platform/fm1/fmzx_app.bin`) in fm1-emu, as the
//! FM-1's boot loader starts it, and looks at it from outside: the LCD, the audio, the
//! firmware's `fmzx_stats`, and instruction counts per function call.
//!
//! Every instruction goes through [`CheckedBus`], so a misaligned RAM access panics with its pc.

use crate::{CheckedBus, RAM_SIZE};
use fm1_core::{Bus, Cpu};
use fm1_soc::{Action, Soc, RAM_BASE, VEC_BASE, XIP_BASE};
use object::{Object, ObjectSymbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where the boot loader enters the app image (XIP).
pub const APP_BASE: u32 = 0x0200_0120;
/// The app slot's size: image and game pack together.
pub const APP_SLOT: u32 = 0x8_DFBC;
/// The game pack starts at the first 4 KiB boundary after the image.
pub const PACK_ALIGN: u32 = 4096;
/// `fmzx_bootopt` word 0 when the boot options are valid ("ZXOP").
pub const BOOTOPT_MAGIC: u32 = 0x5A58_4F50;
/// fm1-emu key id of the OCT- button.
pub const KEY_OCT_DOWN: u8 = 0;
/// The stock audio PLL as the FM-1's boot leaves it (felucca-bsp `FM1_AUDIO_PLL_STOCK0/1`;
/// checked against the vendored header by a unit test). `fm1_audio_init` refuses to start
/// without it.
pub const AUDIO_PLL_STOCK: [(u32, u32); 2] = [(0x0001_19A0, 0x4540_0203), (0x0001_19A4, 0x3F50_3026)];

/// The fm1 build directory from `FMZX_FM1_BUILD`, or a panic that says how to make one.
pub fn build_dir() -> PathBuf {
    let Some(v) = std::env::var_os("FMZX_FM1_BUILD").filter(|v| !v.is_empty()) else {
        panic!(
            "the firmware tests need the device build: from the repository root run\n  \
             tools/fm1-docker cmake --preset fm1 && tools/fm1-docker cmake --build --preset fm1\n\
             then, from host/, set FMZX_FM1_BUILD=$PWD/../build/fm1 and run the tests again"
        );
    };
    std::path::absolute(v).expect("absolute build dir")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\nbuild the firmware first: tools/fm1-docker cmake --preset fm1 && \
             tools/fm1-docker cmake --build --preset fm1",
            path.display()
        )
    })
}

/// The firmware image, its ELF (for symbols) and an optional game pack placed after it.
#[derive(Clone)]
pub struct Firmware {
    pub image: Vec<u8>,
    pub elf: Vec<u8>,
    pub pack: Option<Vec<u8>>,
    symbols: HashMap<String, (u32, u32)>,
}

impl Firmware {
    /// Reads `<build>/platform/fm1/fmzx_app.{bin,elf}`.
    pub fn load(build: &Path) -> Firmware {
        let dir = build.join("platform/fm1");
        let image = read(&dir.join("fmzx_app.bin"));
        let elf = read(&dir.join("fmzx_app.elf"));
        let f = object::File::parse(&*elf).expect("parse the firmware ELF");
        let symbols = f
            .symbols()
            .filter_map(|s| Some((s.name().ok()?.to_string(), (s.address() as u32, s.size() as u32))))
            .collect();
        let fw = Firmware { image, elf, pack: None, symbols };
        assert_eq!(
            fw.symbol("_app_image_end"),
            APP_BASE + fw.image.len() as u32,
            "fmzx_app.bin is not the whole image the ELF describes"
        );
        fw
    }

    /// The same firmware with `pack` in flash at [`Firmware::pack_addr`].
    pub fn with_pack(&self, pack: &[u8]) -> Firmware {
        Firmware { pack: Some(pack.to_vec()), ..self.clone() }
    }

    /// Address of an ELF symbol.
    pub fn symbol(&self, name: &str) -> u32 {
        self.symbol_range(name).start
    }

    /// Address range of an ELF symbol (its size from the symbol table).
    pub fn symbol_range(&self, name: &str) -> std::ops::Range<u32> {
        let &(a, n) = self.symbols.get(name).unwrap_or_else(|| panic!("no symbol {name} in the firmware ELF"));
        a..a + n
    }

    /// The XIP address of the game pack: the first 4 KiB boundary after the image.
    pub fn pack_addr(&self) -> u32 {
        (APP_BASE + self.image.len() as u32).next_multiple_of(PACK_ALIGN)
    }

    /// The XIP window from `XIP_BASE`: the boot area (zeros), the image, then the pack.
    fn xip(&self) -> Vec<u8> {
        let mut x = vec![0u8; (APP_BASE - XIP_BASE) as usize];
        x.extend_from_slice(&self.image);
        if let Some(p) = &self.pack {
            let end = self.pack_addr() + p.len() as u32;
            assert!(end <= APP_BASE + APP_SLOT, "image and pack ({end:#x}) do not fit the app slot");
            x.resize((self.pack_addr() - XIP_BASE) as usize, 0xFF);
            x.extend_from_slice(p);
        }
        x
    }
}

/// What the firmware reads from `fmzx_bootopt` at boot.
#[derive(Clone, Copy, Debug)]
pub struct BootOpt {
    /// RGB565 instead of RGB444 (fm1-emu's LCD always parses RGB565: screen tests set it).
    pub rgb565: bool,
    /// LCD SPI clock divider, 0-15.
    pub divider: u8,
}

/// The firmware's LCD scheduler statistics (`lcd_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LcdStats {
    pub frames: u32,
    pub row_sends: u32,
    pub pending_frames_sum: u32,
    pub bytes: u32,
    pub bands: u32,
}

/// The firmware's `fmzx_stats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FwStats {
    pub frames: u32,
    pub underruns: u32,
    pub overflows: u32,
    pub view: u32,
    pub game: u32,
    pub divider: u32,
    pub budget: u32,
    pub pending: u32,
    pub lcd: LcdStats,
}

/// `fmzx_stats` is 13 words.
const STATS_WORDS: u32 = 13;

struct OpenCall {
    rets: u32,
    sp: u32,
    depth: usize,
    start: u64,
}

struct Watch {
    name: String,
    entry: u32,
    hits: u64,
    first_hit_ms: Option<u64>,
    open: Option<OpenCall>,
    costs: Vec<u64>,
}

/// A booted firmware in fm1-emu.
pub struct Machine {
    pub soc: Soc,
    pub cpu: Cpu,
    symbols: HashMap<String, (u32, u32)>,
    watches: Vec<Watch>,
    /// Instructions executed at each interrupt depth (0 = main).
    depth_steps: Vec<u64>,
    resets: u32,
    fault: Option<u32>,
}

impl Machine {
    /// Starts the image as the FM-1's boot loader does: in user mode at [`APP_BASE`], the
    /// stock audio PLL running, `fmzx_bootopt` written.
    pub fn boot(fw: &Firmware, opt: BootOpt) -> Machine {
        let mut soc = Soc::new(RAM_SIZE as usize, fw.xip());
        soc.periph.skip_idle = true;
        for (reg, v) in AUDIO_PLL_STOCK {
            soc.write32(reg, v);
        }
        let bo = fw.symbol("fmzx_bootopt");
        soc.write32(bo, BOOTOPT_MAGIC);
        soc.write32(bo + 4, u32::from(opt.rgb565) | u32::from(opt.divider & 15) << 8);
        let mut cpu = Cpu::new(APP_BASE);
        cpu.user = true;
        soc.take_boot(&cpu);
        Machine {
            soc,
            cpu,
            fault: fw.symbols.get("fm1_fault_c").map(|s| s.0),
            symbols: fw.symbols.clone(),
            watches: Vec::new(),
            depth_steps: vec![0; 34],
            resets: 0,
        }
    }

    /// Address of an ELF symbol.
    pub fn symbol(&self, name: &str) -> u32 {
        self.symbols.get(name).unwrap_or_else(|| panic!("no symbol {name} in the firmware ELF")).0
    }

    /// Emulated milliseconds since power-on.
    pub fn ms(&self) -> u64 {
        self.soc.periph.ms()
    }

    /// Chip resets so far (watchdog or software).
    pub fn resets(&self) -> u32 {
        self.resets
    }

    /// Runs for `ms` emulated milliseconds.
    pub fn run_ms(&mut self, ms: u64) {
        let end = self.soc.periph.cycles + self.soc.periph.cycle_of_ms(ms);
        while self.soc.periph.cycles < end {
            self.step();
        }
    }

    fn step(&mut self) {
        let soc = &mut self.soc;
        let cpu = &mut self.cpu;
        if soc.periph.tick() {
            soc.advance(0);
        }
        if soc.reset_due() {
            soc.chip_reset(cpu);
            self.resets += 1;
            for w in &mut self.watches {
                w.open = None;
            }
            return;
        }
        soc.periph.in_isr = !cpu.irq_levels.is_empty();
        if soc.periph.any_pending() {
            if let Some((irq, prio)) = soc.pending_irq() {
                if cpu.irq_ready(prio) {
                    let handler = soc.read32(VEC_BASE + 4 * u32::from(irq));
                    cpu.interrupt(handler, prio);
                    soc.periph.in_isr = true;
                }
            }
        }
        let pc = cpu.pc;
        let depth = cpu.irq_levels.len().min(self.depth_steps.len() - 1);
        if Some(pc) == self.fault {
            let frame = cpu.regs[0];
            let at = soc.ram.read32(frame + 64 - RAM_BASE);
            let vec = (soc.ram.read32(frame + 76 - RAM_BASE)).wrapping_sub(self.symbols["fm1_fatal_stubs"].0) / 6;
            panic!("firmware fault: vector {}, pc {at:#010x}", vec.wrapping_sub(1));
        }
        for w in &mut self.watches {
            if pc == w.entry {
                w.hits += 1;
                w.first_hit_ms.get_or_insert(soc.periph.ms());
                if w.open.is_none() {
                    w.open = Some(OpenCall { rets: cpu.rets, sp: cpu.sp, depth, start: self.depth_steps[depth] });
                }
            } else if let Some(o) = &w.open {
                if o.depth == depth && pc == o.rets && cpu.sp >= o.sp {
                    w.costs.push(self.depth_steps[depth] - o.start);
                    w.open = None;
                }
            }
        }
        self.depth_steps[depth] += 1;
        let mut bus = CheckedBus { soc, pc };
        cpu.step(&mut bus).unwrap_or_else(|e| panic!("cpu fault at pc {pc:#010x}: {e:?}"));
    }

    /// Starts counting calls of `symbol` (idempotent); see [`Machine::cost_of`].
    pub fn watch(&mut self, symbol: &str) {
        if self.watches.iter().any(|w| w.name == symbol) {
            return;
        }
        let entry = self.symbol(symbol);
        self.watches.push(Watch { name: symbol.to_string(), entry, hits: 0, first_hit_ms: None, open: None, costs: Vec::new() });
    }

    fn watched(&mut self, symbol: &str) -> &Watch {
        self.watch(symbol);
        self.watches.iter().find(|w| w.name == symbol).expect("watched")
    }

    /// Instructions per completed call of `symbol` since it was first watched: from its entry
    /// to the return to its caller, at the caller's interrupt depth (interrupts taken during
    /// the call are not counted). A recursive call counts within the outer one. The first
    /// call of `cost_of` for a symbol starts watching it (and returns nothing yet).
    pub fn cost_of(&mut self, symbol: &str) -> Vec<u64> {
        self.watched(symbol).costs.clone()
    }

    /// Times execution reached `symbol`'s entry since it was first watched.
    pub fn hits(&mut self, symbol: &str) -> u64 {
        self.watched(symbol).hits
    }

    /// Emulated time of the first entry into `symbol` since it was watched.
    pub fn first_hit_ms(&mut self, symbol: &str) -> Option<u64> {
        self.watched(symbol).first_hit_ms
    }

    /// Presses fm1-emu key `key_id` (buttons 0-13, notes 14-40), runs `ms`, releases it.
    pub fn press(&mut self, key_id: u8, ms: u64) {
        self.hold(key_id, true);
        self.run_ms(ms);
        self.hold(key_id, false);
    }

    /// Holds or releases a key.
    pub fn hold(&mut self, key_id: u8, down: bool) {
        self.soc.periph.apply(&Action::Key { id: key_id, down });
    }

    /// Turns encoder `enc` by `clicks` detents (+ = clockwise).
    pub fn turn(&mut self, enc: u8, clicks: i32) {
        self.soc.periph.apply(&Action::Enc { enc, clicks });
    }

    /// Sets ADC channel `ch` (MASTER pot = 4) to `v` (0-1023).
    pub fn set_adc(&mut self, ch: u8, v: u16) {
        self.soc.periph.apply(&Action::Adc { ch, value: v });
    }

    /// The LCD's frame buffer, 240x240 RGB565, row by row.
    pub fn lcd_rgb565(&self) -> &[u16] {
        &self.soc.lcd.fb
    }

    /// The panel has had DISPON (and no reset since).
    pub fn display_on(&self) -> bool {
        self.soc.lcd.display_on
    }

    /// Audio played so far, interleaved L/R.
    pub fn audio(&self) -> &[i16] {
        &self.soc.periph.audio
    }

    /// A 32-bit word of RAM.
    pub fn peek32(&self, addr: u32) -> u32 {
        self.soc.ram.read32(addr - RAM_BASE)
    }

    /// The firmware's `fmzx_stats`.
    pub fn stats(&self) -> FwStats {
        let &(a, size) = self.symbols.get("fmzx_stats").expect("no symbol fmzx_stats");
        assert_eq!(size, 4 * STATS_WORDS, "fmzx_stats is not laid out as FwStats");
        let w = |i: u32| self.peek32(a + 4 * i);
        FwStats {
            frames: w(0),
            underruns: w(1),
            overflows: w(2),
            view: w(3),
            game: w(4),
            divider: w(5),
            budget: w(6),
            pending: w(7),
            lcd: LcdStats { frames: w(8), row_sends: w(9), pending_frames_sum: w(10), bytes: w(11), bands: w(12) },
        }
    }
}

/// RGB565 to 8-bit RGB, each channel's top bits repeated into the low ones.
pub fn rgb565_to_rgb8(v: u16) -> [u8; 3] {
    let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
    [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
}

/// Writes a 240x240 RGB565 frame buffer as an 8-bit RGB PNG.
pub fn write_png(path: &Path, fb: &[u16]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
    }
    let file = std::fs::File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 240, 240);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = fb.iter().flat_map(|&v| rgb565_to_rgb8(v)).collect();
    enc.write_header().and_then(|mut w| w.write_image_data(&data)).expect("write PNG");
}

/// Reads an 8-bit RGB PNG as bytes.
pub fn read_png(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; dec.output_buffer_size()?];
    let info = dec.next_frame(&mut buf).ok()?;
    (info.color_type == png::ColorType::Rgb && info.bit_depth == png::BitDepth::Eight).then(|| {
        buf.truncate(info.buffer_size());
        buf
    })
}

/// Compares the LCD with `golden` (a PNG) pixel for pixel. `FMZX_GOLDEN_UPDATE=1` rewrites the
/// golden instead; on a mismatch the screen is written to `<actual>` for a look.
pub fn assert_golden(fb: &[u16], golden: &Path, actual: &Path) {
    if std::env::var_os("FMZX_GOLDEN_UPDATE").is_some_and(|v| v == "1") {
        write_png(golden, fb);
        return;
    }
    let want = read_png(golden);
    let got: Vec<u8> = fb.iter().flat_map(|&v| rgb565_to_rgb8(v)).collect();
    if want.as_deref() != Some(&got[..]) {
        write_png(actual, fb);
        let diff = want.map_or(usize::MAX, |w| w.chunks(3).zip(got.chunks(3)).filter(|(a, b)| a != b).count());
        panic!(
            "the LCD differs from {} ({} pixels); the screen is in {} (FMZX_GOLDEN_UPDATE=1 rewrites the golden)",
            golden.display(),
            if diff == usize::MAX { "missing or unreadable".to_string() } else { diff.to_string() },
            actual.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_pll_matches_felucca_bsp() {
        let h = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../platform/fm1/bsp/boards/fm1/fm1_audio.h"),
        )
        .expect("read the vendored fm1_audio.h");
        let value = |name: &str| {
            let line = h.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let hex = line.split_whitespace().nth(2).expect("value").trim_end_matches('u');
            u32::from_str_radix(hex.trim_start_matches("0x"), 16).expect("hex")
        };
        assert!(h.contains("#define FM1_AUDIO_PLL_CON0 FM1_AUDIO_REG(uint32_t, 0x119A0u)"));
        assert!(h.contains("#define FM1_AUDIO_PLL_CON1 FM1_AUDIO_REG(uint32_t, 0x119A4u)"));
        assert_eq!(AUDIO_PLL_STOCK, [(0x119A0, value("FM1_AUDIO_PLL_STOCK0")), (0x119A4, value("FM1_AUDIO_PLL_STOCK1"))]);
    }

    #[test]
    fn rgb565_expands_to_full_range() {
        assert_eq!(rgb565_to_rgb8(0xFFFF), [255, 255, 255]);
        assert_eq!(rgb565_to_rgb8(0x0000), [0, 0, 0]);
        assert_eq!(rgb565_to_rgb8(0xF800), [255, 0, 0]);
    }
}
