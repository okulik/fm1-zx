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

pub use sys::{zx_edge, zx_regs, ZX_FRAME_TSTATES, ZX_RAM_SIZE, ZX_ROM_SIZE};

static ROM_48: &[u8; ZX_ROM_SIZE as usize] = include_bytes!("../../../../third_party/zx-rom/48.rom");

/// A 48K Spectrum. Not `Sync`: the C core has no locking.
pub struct Machine {
    raw: Box<sys::zx_machine>,
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
        Machine { raw, _not_sync: PhantomData }
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
