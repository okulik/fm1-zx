// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Learned sound entries: the personal-pack builder learns each game's music routines and stores
//! them in the pack as method B entries made of data (`core/sound/learned.c` reads them on the
//! device). This crate holds the learner's parts: listening to a game ([`listen`]), a Z80
//! decoder ([`z80`]), the code reader that models a music routine from its code ([`model`]), the
//! fit of a model to what was heard and the entry built from it ([`fit`]), the gates and the
//! replay through method B ([`gate`]), the channels' stored roles ([`roles`]), the encoder of the
//! programs ([`prog`]) and the entry lists ([`entry`]), and the pipeline for one game
//! ([`learn`](mod@learn)) with what it says ([`report`]). The observing learner ([`observe`])
//! finds a routine's channels from what the speaker plays instead, by its spectrum
//! ([`spectrum`]). The transcriber ([`transcribe`]) reads the notes of music nothing else
//! reads from the speaker's signal alone, for a recorded score ([`scores`]).

pub mod entry;
pub mod fit;
pub mod gate;
pub mod learn;
pub mod listen;
pub mod model;
pub mod observe;
pub mod prog;
pub mod report;
pub mod roles;
pub mod scores;
pub mod spectrum;
pub mod transcribe;
pub mod z80;

pub use entry::{decode_scores, encode_list, encode_list_scores, encode_list_with, replace_scores, score_key, BuiltinRoles, LearnedEntry, ScoreEvent, ScoreRecord, SCORE_DRUM};
pub use learn::{learn, learn_with, GameSound, LearnOptions, LEARNER_VERSION};
pub use prog::{encode_prog, eval_prog, Op};
