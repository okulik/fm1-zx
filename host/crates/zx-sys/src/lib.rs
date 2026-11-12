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

pub use sys::{zx_regs, ZX_FRAME_TSTATES, ZX_RAM_SIZE, ZX_ROM_SIZE};

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
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}
