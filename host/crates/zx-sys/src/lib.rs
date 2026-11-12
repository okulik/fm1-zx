// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Raw bindings to the C machine core, and the safe [`Machine`] wrapper.

use std::marker::PhantomData;
use std::mem::MaybeUninit;

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unnecessary_transmutes,
    clippy::all
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub use sys::{lcd_stats, zx_edge, zx_regs, ZX_ERR_FORMAT, ZX_ERR_UNSUPPORTED, ZX_FRAME_TSTATES, ZX_RAM_SIZE, ZX_ROM_SIZE};

static ROM_48: &[u8; ZX_ROM_SIZE as usize] = include_bytes!("../../../../third_party/zx-rom/48.rom");

/// A 48K Spectrum. Not `Sync`: the C core has no locking.
pub struct Machine {
    raw: Box<sys::zx_machine>,
    /// The inserted .tap; the C core keeps a pointer into this heap buffer.
    tape: Option<Box<[u8]>>,
    _not_sync: PhantomData<*const ()>,
}

impl Machine {
    /// A powered-on machine with the built-in 48K ROM.
    pub fn new() -> Self {
        // SAFETY: zx_machine is plain data (integers, arrays, nullable fn
        // pointers); all-zero is a valid value, and zx_init sets it up.
        let mut raw: Box<sys::zx_machine> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: the ROM is 'static, so it outlives the machine; `raw` is
        // boxed, so the CPU's context pointer to it stays valid.
        unsafe { sys::zx_init(&mut *raw, ROM_48.as_ptr()) };
        Machine { raw, tape: None, _not_sync: PhantomData }
    }

    pub fn run_frame(&mut self) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_run_frame(&mut *self.raw) }
    }

    pub fn regs(&self) -> zx_regs {
        let mut r = MaybeUninit::<zx_regs>::zeroed();
        // SAFETY: both pointers are valid; zx_get_regs fills every field.
        unsafe {
            sys::zx_get_regs(&*self.raw, r.as_mut_ptr());
            r.assume_init()
        }
    }

    pub fn set_regs(&mut self, r: &zx_regs) {
        // SAFETY: both pointers are valid.
        unsafe { sys::zx_set_regs(&mut *self.raw, r) }
    }

    pub fn peek(&self, addr: u16) -> u8 {
        // SAFETY: any 16-bit address is valid.
        unsafe { sys::zx_peek(&*self.raw, addr) }
    }

    pub fn poke(&mut self, addr: u16, value: u8) {
        // SAFETY: any 16-bit address is valid; ROM writes are ignored.
        unsafe { sys::zx_poke(&mut *self.raw, addr, value) }
    }

    /// T-states already run into the next frame.
    pub fn frame_tstate(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_frame_tstate(&*self.raw) }
    }

    pub fn frame_count(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_frame_count(&*self.raw) }
    }

    /// Row r is address line A(8+r); a set bit is a pressed key.
    pub fn set_keys(&mut self, rows: [u8; 8]) {
        // SAFETY: `rows` is 8 bytes.
        unsafe { sys::zx_set_keys(&mut *self.raw, rows.as_ptr()) }
    }

    /// Bit 0 right, 1 left, 2 down, 3 up, 4 fire.
    pub fn set_kempston(&mut self, bits: u8) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_set_kempston(&mut *self.raw, bits) }
    }

    /// Beeper level changes of the frame run last.
    pub fn edges(&self) -> &[zx_edge] {
        let mut n = 0u32;
        // SAFETY: the returned array holds `n` edges and lives as long as `self`.
        unsafe {
            let p = sys::zx_edges(&*self.raw, &mut n);
            std::slice::from_raw_parts(p, n as usize)
        }
    }

    /// Beeper level in effect at the start of the current edge list; feed it
    /// to [`Beeper::set_level`] before rendering that list.
    pub fn edges_start_level(&self) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_edges_start_level(&*self.raw) }
    }

    /// Border colour on a frame line (0-311).
    pub fn border_line(&self, line: usize) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_border_line(&*self.raw, line as u32) }
    }

    /// Whether screen line `y` (0-191) changed since the last frame began.
    pub fn line_dirty(&self, y: usize) -> bool {
        // SAFETY: `raw` was initialised by zx_init; out-of-range lines return 0.
        unsafe { sys::zx_line_dirty(&*self.raw, y as u32) != 0 }
    }

    /// Palette indices (0-15, bright = +8) of screen line `y` (0-191).
    pub fn screen_line(&self, y: usize) -> [u8; 256] {
        let mut out = [0u8; 256];
        // SAFETY: `out` is 256 bytes.
        unsafe { sys::zx_screen_line(&*self.raw, y as u32, out.as_mut_ptr()) };
        out
    }

    /// LCD row `row` (0-239) as palette indices, cropped at `crop_x`.
    pub fn compose_row(&self, row: usize, crop_x: usize) -> [u8; 240] {
        let mut out = [0u8; 240];
        // SAFETY: `raw` was initialised by zx_init; `out` is LCD_W bytes.
        unsafe { sys::pix_compose_zx(&*self.raw, row as u32, crop_x as u32, out.as_mut_ptr()) };
        out
    }

    /// Loads a 48K `.sna` snapshot; `Err` carries the C error code and leaves
    /// the machine unchanged.
    pub fn load_sna(&mut self, data: &[u8]) -> Result<(), i32> {
        // SAFETY: the pointer and length describe `data`.
        match unsafe { sys::zx_load_sna(&mut *self.raw, data.as_ptr(), data.len() as u32) } {
            0 => {
                // the C side ejected the tape; free the buffer it was reading
                self.tape = None;
                Ok(())
            }
            e => Err(e),
        }
    }

    /// Loads a 48K `.z80` snapshot (v1, v2 or v3); errors as [`Machine::load_sna`].
    pub fn load_z80(&mut self, data: &[u8]) -> Result<(), i32> {
        // SAFETY: the pointer and length describe `data`.
        match unsafe { sys::zx_load_z80(&mut *self.raw, data.as_ptr(), data.len() as u32) } {
            0 => {
                // the C side ejected the tape; free the buffer it was reading
                self.tape = None;
                Ok(())
            }
            e => Err(e),
        }
    }

    /// Inserts a `.tap` (the machine keeps its own copy). `Err` carries the C
    /// error code (`ZX_ERR_FORMAT` for a block that overruns the data) and
    /// leaves the previous tape, if any, in place.
    pub fn tape_insert(&mut self, tap: &[u8]) -> Result<(), i32> {
        let buf: Box<[u8]> = tap.into();
        // SAFETY: the pointer and length describe `buf`, which `self.tape`
        // keeps alive (heap storage does not move) for as long as the C side
        // may read it; the C side only stores them on success.
        match unsafe { sys::zx_tape_insert(&mut *self.raw, buf.as_ptr(), buf.len() as u32) } {
            0 => {
                self.tape = Some(buf);
                Ok(())
            }
            e => Err(e),
        }
    }

    /// True while the machine owns a tape buffer.
    pub fn has_tape_buffer(&self) -> bool {
        self.tape.is_some()
    }

    /// Removes the tape and drops the buffer the machine was reading.
    pub fn tape_eject(&mut self) {
        // SAFETY: `raw` was initialised by zx_init; the C side forgets the pointer first.
        unsafe { sys::zx_tape_eject(&mut *self.raw) }
        self.tape = None;
    }

    /// Resets the machine and types `LOAD ""` + ENTER over the next frames.
    pub fn tape_autostart(&mut self) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_tape_autostart(&mut *self.raw) }
    }

    /// Host tests only: the current beeper level.
    pub fn test_level(&self) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_level(&*self.raw) }
    }

    /// Host tests only: sets the frame T-state the next instruction starts at.
    pub fn test_set_tstate(&mut self, t: u32) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_set_tstate(&mut *self.raw, t) }
    }

    /// Host tests only: runs one instruction and returns its T-states,
    /// contention included.
    pub fn test_step(&mut self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_step(&mut *self.raw) }
    }
}

/// Delay, in T-states, of a contended access that starts at frame T-state `tstate`.
pub fn contention(tstate: u32) -> u32 {
    // SAFETY: pure function of its argument.
    unsafe { sys::zx_contention(tstate) }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

/// The beeper renderer: turns each frame's edge list into audio samples.
pub struct Beeper {
    raw: Box<sys::zx_beeper>,
}

impl Beeper {
    /// A renderer for a sample rate of `num / den` Hz.
    pub fn new(num: u32, den: u32) -> Self {
        // SAFETY: zx_beeper is plain integers; all-zero is valid and zx_beeper_init sets it up.
        let mut raw: Box<sys::zx_beeper> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: `raw` is a valid, exclusively owned zx_beeper.
        unsafe { sys::zx_beeper_init(&mut *raw, num, den) };
        Beeper { raw }
    }

    /// Samples the next `render` call produces.
    pub fn frame_samples(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_frame_samples(&*self.raw) }
    }

    /// Renders one frame whose beeper level changes are `edges`.
    pub fn render(&mut self, edges: &[zx_edge]) -> Vec<i16> {
        let mut out = vec![0i16; self.frame_samples() as usize];
        // SAFETY: `out` holds frame_samples() entries, which render writes at most.
        let n = unsafe { sys::zx_beeper_render(&mut *self.raw, edges.as_ptr(), edges.len() as u32, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }

    /// Sets the level the next render starts from.
    pub fn set_level(&mut self, level: u8) {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_set_level(&mut *self.raw, level) }
    }

    /// Host tests only: the renderer's current level.
    pub fn test_level(&self) -> u8 {
        self.raw.level
    }

    /// Host tests only: switches the DC blocker on or off.
    pub fn test_dc_block(&mut self, on: bool) {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_test_dc_block(&mut *self.raw, on as i32) }
    }
}

/// Pixel formats of the LCD.
#[derive(Clone, Copy)]
pub enum PixFormat {
    Rgb444,
    Rgb565,
}

impl PixFormat {
    fn raw(self) -> i32 {
        match self {
            PixFormat::Rgb444 => sys::PIX_RGB444 as i32,
            PixFormat::Rgb565 => sys::PIX_RGB565 as i32,
        }
    }

    /// Bytes of one 240-pixel row.
    pub fn row_bytes(self) -> usize {
        // SAFETY: pure function.
        unsafe { sys::pix_row_bytes(self.raw()) as usize }
    }

    /// Packs 240 palette indices into panel bytes.
    pub fn pack(self, idx: &[u8; 240]) -> Vec<u8> {
        let mut out = vec![0u8; self.row_bytes()];
        // SAFETY: `idx` is LCD_W bytes and `out` holds one packed row.
        unsafe { sys::pix_pack(self.raw(), idx.as_ptr(), out.as_mut_ptr()) };
        out
    }
}

/// `150000 / (divider + 1)` bytes per frame.
pub fn lcd_budget_bytes(divider: u32) -> u32 {
    // SAFETY: pure function.
    unsafe { sys::lcd_budget_bytes(divider) }
}

/// The LCD row scheduler.
pub struct Sched {
    raw: Box<sys::lcd_sched>,
}

impl Sched {
    pub fn new(fmt: PixFormat) -> Self {
        // SAFETY: lcd_sched is plain integers and arrays; all-zero is valid and
        // lcd_sched_init sets it up.
        let mut raw: Box<sys::lcd_sched> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: `raw` is a valid lcd_sched.
        unsafe { sys::lcd_sched_init(&mut *raw, fmt.row_bytes() as u32) };
        Sched { raw }
    }

    pub fn mark(&mut self, row: usize, frame: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init; out-of-range rows are ignored.
        unsafe { sys::lcd_sched_mark(&mut *self.raw, row as u32, frame) }
    }

    pub fn mark_all(&mut self, frame: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_mark_all(&mut *self.raw, frame) }
    }

    pub fn begin_frame(&mut self, frame: u32, budget: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_begin_frame(&mut *self.raw, frame, budget) }
    }

    /// The next band as `(row0, rows)`, or `None` when nothing fits.
    pub fn next_band(&mut self) -> Option<(usize, usize)> {
        let (mut row0, mut rows) = (0u32, 0u32);
        // SAFETY: `raw` was initialised by lcd_sched_init; both outputs are valid.
        let got = unsafe { sys::lcd_sched_next_band(&mut *self.raw, &mut row0, &mut rows) };
        (got != 0).then_some((row0 as usize, rows as usize))
    }

    pub fn pending(&self) -> usize {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_pending(&*self.raw) as usize }
    }

    /// The frame a row has been pending since, if it is pending.
    pub fn pending_since(&self, row: usize) -> Option<u32> {
        (self.raw.pending[row] != 0).then_some(self.raw.pending_since[row])
    }

    pub fn stats(&self) -> lcd_stats {
        self.raw.stats
    }
}
