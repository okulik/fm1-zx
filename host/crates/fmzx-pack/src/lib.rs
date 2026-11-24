// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host tool that builds the FM-1 ZX game pack and firmware package.
//!
//! `fwsc` and `lz4blk` are GPL-3.0-only (ports of Felucca's package tools); the rest is
//! GPL-3.0-or-later. The tool as a whole is therefore distributed under GPL-3.0-only. The
//! firmware image links none of it.

pub mod arrange;
pub mod fwsc;
pub mod inputs;
pub mod keymaps;
pub mod keys;
pub mod layout;
pub mod lz4blk;
pub mod manifest;
pub mod menu_art;
pub mod pack;
pub mod package;
pub mod personal;
pub mod tapeload;
pub mod testprogs;
pub mod tzx;
