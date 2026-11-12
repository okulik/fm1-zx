// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared fixtures for the machine tests.

pub use zx_sys::Machine;

/// Pokes `bytes` into the machine starting at `addr`.
pub fn load(m: &mut Machine, addr: u16, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        m.poke(addr.wrapping_add(i as u16), *b);
    }
}
