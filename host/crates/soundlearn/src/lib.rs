// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Learned sound entries: the personal-pack builder learns each game's music routines and stores
//! them in the pack as method B entries made of data (`core/sound/learned.c` reads them on the
//! device). This crate holds the learner's parts: listening to a game ([`listen`]), a Z80
//! decoder ([`z80`]), the code reader that models a music routine from its code ([`model`]), the
//! fit of a model to what was heard and the entry built from it ([`fit`]), the gates and the
//! replay through method B ([`gate`]), and the encoder of the programs ([`prog`]) and the entry
//! lists ([`entry`]).

pub mod entry;
pub mod fit;
pub mod gate;
pub mod listen;
pub mod model;
pub mod prog;
pub mod z80;

pub use entry::{encode_list, LearnedEntry};
pub use prog::{encode_prog, eval_prog, Op};
