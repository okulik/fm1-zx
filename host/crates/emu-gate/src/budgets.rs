// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The firmware's budgets, what the budget and report tests compute from the ELF and the
//! machine, and the manifest's `demo` scripts.

use crate::fw::{BootOpt, Firmware, LcdStats, Machine, APP_BASE, APP_SLOT};
use object::{Object, ObjectSection};

/// CPU per Spectrum frame: `game_frame` plus the `lcd_compose_band` calls of that frame, at
/// 240 MHz and 20 ms a frame (4,800,000 instructions).
pub const CPU_PER_FRAME: u64 = 3_600_000;
/// The audio interrupt (`fm1_alnk0_irq`) per 128-frame block.
pub const ISR_PER_BLOCK: u64 = 8_000;
/// RAMTEXT, the SRAM code region (`.ram_text`).
pub const RAMTEXT_MAX: u32 = 49_152;
/// The user stack's bottom: POOL must end below it (`app.ld`).
pub const STACK_GUARD: u32 = 0x01C7_8000;
/// Where POOL starts (`app.ld`).
pub const POOL_START: u32 = 0x01C1_E000;
/// Frames the Spectrum runs per second.
pub const FRAMES_PER_SECOND: u64 = 50;

/// Size of the ELF section `.ram_text`.
pub fn ram_text_size(fw: &Firmware) -> u32 {
    let f = object::File::parse(&*fw.elf).expect("parse the firmware ELF");
    f.section_by_name(".ram_text").expect("no .ram_text section").size() as u32
}

/// Bytes the app slot holds: from the image's start to the end of the pack, which starts at
/// the first 4 KiB boundary after the image ([`Firmware::pack_addr`]).
pub fn image_and_pack_size(fw: &Firmware, pack_len: usize) -> u32 {
    fw.pack_addr() - APP_BASE + pack_len as u32
}

/// The slot's size, for messages.
pub const fn slot_size() -> u32 {
    APP_SLOT
}

/// POOL's extent from the ELF symbols (`_pool_start`, `_pool_end`): used bytes and the bytes
/// left below the stack guard.
pub fn pool_use(fw: &Firmware) -> (u32, i64) {
    let (a, b) = (fw.symbol("_pool_start"), fw.symbol("_pool_end"));
    assert_eq!(a, POOL_START, "POOL does not start where app.ld says");
    (b - a, i64::from(STACK_GUARD) - i64::from(b))
}

/// Display frames per second of a window of LCD counters ([`lcd_delta`]):
/// `50 * row_sends / pending_frames_sum`, the rate at which a changed row reaches the panel.
/// `None` when nothing was sent (a static screen).
pub fn fps(d: &LcdStats) -> Option<f64> {
    (d.pending_frames_sum != 0)
        .then(|| FRAMES_PER_SECOND as f64 * f64::from(d.row_sends) / f64::from(d.pending_frames_sum))
}

/// The counters' change from `from` to `to`; `max_age` is `to`'s (reset it at the window start).
pub fn lcd_delta(from: &LcdStats, to: &LcdStats) -> LcdStats {
    LcdStats {
        frames: to.frames - from.frames,
        row_sends: to.row_sends - from.row_sends,
        pending_frames_sum: to.pending_frames_sum - from.pending_frames_sum,
        bytes: to.bytes - from.bytes,
        bands: to.bands - from.bands,
        max_age: to.max_age,
    }
}

/// One step of a game's `demo` script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoStep {
    /// fm1-emu key id.
    pub key: u8,
    /// When to press it, in ms after the game started.
    pub at_ms: u64,
    /// How long to hold it.
    pub hold_ms: u64,
}

/// How long a demo key is held when the step gives no `+hold`.
pub const DEMO_HOLD_MS: u64 = 100;

const BUTTONS: [&str; 14] =
    ["OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
const PITCHES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// fm1-emu key id of a manifest input name (`note.F3`, `button.OCT-`) or `PLAY` / `HOME`.
/// Buttons are 0-13, notes 14-40 (note 0 is F3).
pub fn key_id(name: &str) -> Result<u8, String> {
    let bad = || format!("unknown demo key {name:?}");
    if let Some(i) = BUTTONS.iter().position(|b| *b == name) {
        return Ok(i as u8);
    }
    match name.split_once('.').ok_or_else(bad)? {
        ("button", b) => BUTTONS.iter().position(|x| *x == b).map(|i| i as u8).ok_or_else(bad),
        ("note", n) => (0..27usize)
            .find(|&i| {
                let s = i + 5;
                format!("{}{}", PITCHES[s % 12], 3 + s / 12) == n
            })
            .map(|i| 14 + i as u8)
            .ok_or_else(bad),
        _ => Err(bad()),
    }
}

/// Parses `demo = "<key>@<ms>[+<hold>],..."`: each key is pressed `ms` after the game started
/// and held for `hold` ms ([`DEMO_HOLD_MS`] when not given). Sorted by time.
pub fn parse_demo(s: &str) -> Result<Vec<DemoStep>, String> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, when) = part.split_once('@').ok_or_else(|| format!("demo step {part:?}: expected key@ms"))?;
        let (at, hold) = match when.split_once('+') {
            Some((a, h)) => (a, h.trim().parse::<u64>().map_err(|e| format!("demo step {part:?}: {e}"))?),
            None => (when, DEMO_HOLD_MS),
        };
        let at_ms = at.trim().parse::<u64>().map_err(|e| format!("demo step {part:?}: {e}"))?;
        v.push(DemoStep { key: key_id(key.trim())?, at_ms, hold_ms: hold });
    }
    v.sort_by_key(|s| s.at_ms);
    Ok(v)
}

/// PLAY, HOME and the SELECT encoder as fm1-emu knows them.
const PLAY: u8 = 12;
const ENC_SELECT: u8 = 0;

/// Where a measurement starts, in ms after the game was started: the tapes are loaded by about
/// 10 s (a tape's ROM loader runs at full speed with the audio paced frames) and the demos have
/// got the two tapes past their title screens and menus by 15.5 s; the snapshots just run.
pub const SETTLE_MS: u64 = 16_000;

/// What one game run measured over its window.
#[derive(Clone, Debug)]
pub struct Measure {
    /// LCD scheduler counters over the window (`max_age` from the window's start).
    pub lcd: LcdStats,
    /// Spectrum frames run in the window (`fmzx_stats.frames`).
    pub frames: u32,
    /// `game_frame` plus its `lcd_compose_band` calls, per frame.
    pub cpu: Vec<u64>,
    /// `fm1_alnk0_irq` per call (interrupt time excluded from `cpu`).
    pub isr: Vec<u64>,
    /// LCD rows sent in each 1-second sub-window of the window (a static screen sends none).
    pub rows_per_second: Vec<u32>,
    /// Chip resets during the run.
    pub resets: u32,
    /// The game was still running (`fmzx_stats.view` = game) at the end.
    pub in_game: bool,
}

/// Boots `fw` (with its pack) in `opt`, selects game `index` in the picker and starts it with
/// PLAY; the clock for [`Demo::run_to`] starts when this returns.
pub fn start_game(fw: &Firmware, opt: BootOpt, index: u32) -> Machine {
    let mut m = Machine::boot(fw, opt);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0, "the firmware reset while booting");
    if index > 0 {
        m.turn(ENC_SELECT, index as i32);
        m.run_ms(200);
    }
    m.press(PLAY, 30);
    m.run_ms(20);
    m
}

/// A demo script being played: its keys go down and up at their times, across several calls.
pub struct Demo {
    /// (ms after the start, key, down), in time order; releases before presses at the same time.
    events: Vec<(u64, u8, bool)>,
    next: usize,
}

impl Demo {
    pub fn new(steps: &[DemoStep]) -> Demo {
        let mut events: Vec<(u64, u8, bool)> =
            steps.iter().flat_map(|s| [(s.at_ms, s.key, true), (s.at_ms + s.hold_ms, s.key, false)]).collect();
        events.sort_by_key(|&(t, _, down)| (t, down));
        Demo { events, next: 0 }
    }

    /// Runs until `until_ms` after `t0` (emulated ms), pressing and releasing the keys as they
    /// come due; a key still held at `until_ms` stays held for the next call.
    pub fn run_to(&mut self, m: &mut Machine, t0: u64, until_ms: u64) {
        while let Some(&(t, key, down)) = self.events.get(self.next).filter(|e| e.0 < until_ms) {
            if t0 + t > m.ms() {
                m.run_ms(t0 + t - m.ms());
            }
            m.hold(key, down);
            self.next += 1;
        }
        if m.ms() < t0 + until_ms {
            m.run_ms(t0 + until_ms - m.ms());
        }
    }
}

/// Sub-windows (1 s) that must have sent at least [`GAMEPLAY_MIN_ROWS`] rows, as a share of all,
/// for a demo game to count as being played: a title screen or a menu sends next to nothing.
pub const GAMEPLAY_MIN_SHARE: f64 = 0.9;
/// Rows a second of gameplay changes at least (a title's blinking cursor is a few rows).
pub const GAMEPLAY_MIN_ROWS: u32 = 8;

/// The share of the 1-second sub-windows in which at least [`GAMEPLAY_MIN_ROWS`] rows were sent.
pub fn active_share(m: &Measure) -> f64 {
    let n = m.rows_per_second.iter().filter(|&&r| r >= GAMEPLAY_MIN_ROWS).count();
    n as f64 / m.rows_per_second.len().max(1) as f64
}

/// Starts game `index` and measures `window_ms` of it from `settle_ms` on, with the demo's keys
/// pressed throughout.
pub fn measure(fw: &Firmware, opt: BootOpt, index: u32, demo: &[DemoStep], settle_ms: u64, window_ms: u64) -> Measure {
    let mut m = start_game(fw, opt, index);
    let t0 = m.ms();
    let mut d = Demo::new(demo);
    d.run_to(&mut m, t0, settle_ms);
    for sym in ["game_frame", "lcd_compose_band", "fm1_alnk0_irq"] {
        m.watch(sym);
    }
    m.clear_costs();
    m.reset_max_age();
    let (s0, resets0) = (m.stats(), m.resets());
    let mut rows_per_second = Vec::new();
    let mut prev = s0.lcd.row_sends;
    for i in 1..=window_ms.div_ceil(1000) {
        d.run_to(&mut m, t0, settle_ms + (i * 1000).min(window_ms));
        let now = m.stats().lcd.row_sends;
        rows_per_second.push(now - prev);
        prev = now;
    }
    let s1 = m.stats();
    Measure {
        lcd: lcd_delta(&s0.lcd, &s1.lcd),
        frames: s1.frames - s0.frames,
        cpu: m.cost_with("game_frame", "lcd_compose_band"),
        isr: m.cost_of("fm1_alnk0_irq"),
        rows_per_second,
        resets: m.resets() - resets0,
        in_game: s1.view == zx_view_game(),
    }
}

/// `menu_view` value while a game runs (`zx_sys::view::GAME`).
const fn zx_view_game() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_id("button.OCT-"), Ok(0));
        assert_eq!(key_id("PLAY"), Ok(12));
        assert_eq!(key_id("HOME"), Ok(8));
        assert_eq!(key_id("note.F3"), Ok(14));
        assert_eq!(key_id("note.A3"), Ok(18));
        assert_eq!(key_id("note.G5"), Ok(40));
        assert!(key_id("note.H9").is_err());
    }

    #[test]
    fn demo_scripts() {
        let d = parse_demo("button.OCT-@3000, note.C5@5000+400,note.A3@4000").unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d[0], DemoStep { key: 0, at_ms: 3000, hold_ms: 100 });
        assert_eq!(d[1].at_ms, 4000);
        assert_eq!(d[2], DemoStep { key: 33, at_ms: 5000, hold_ms: 400 });
        assert!(parse_demo("note.F3").is_err());
        assert_eq!(parse_demo("").unwrap(), vec![]);
    }
}
