// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A learned entry (one music routine of a game) and the encoding of a game's entry list, as the
//! pack's sound section holds it and `snd_learned_decode` (`core/sound/learned.c`) reads it.
//!
//! The list (4-byte aligned; little-endian; sizes in bytes):
//!
//! ```text
//! u8 count (<= 8), u8 0, u16 0, then count entries:
//!   u16 size (multiple of 4, the whole entry), u8 kind (0 music), u8 channels (1-3)
//!   u64 digest
//!   u16 len, u8 n_wild, u8 n_reloc
//!   u8 n_sites, u8 flips, u8 drum, u8 timing (1: timing program present)
//!   u16 pass, u16 stale_from, u16 stale_len   (stale: anchor-relative pc range; len 0 = none)
//!   u8 level[3], u8 instrument[3] (reserved, 0)
//!   u8 prog_len[3], u8 timing_len
//!   u16 wild[n_wild], u16 reloc[n_reloc], {u16 off, u8 role, u8 chan} sites[n_sites], u16 pos[channels]
//!   channel programs (prog_len[c] bytes each, c < channels), timing program, zero padding to size
//! ```

use crate::prog::{encode_prog, Op};

/// One learned routine: what a built-in `snd_engine` carries, with programs for its reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearnedEntry {
    /// 0: music.
    pub kind: u8,
    /// FNV-1a 64 of the signature (`snd_engine_digest`).
    pub digest: u64,
    /// Signature bytes from the anchor.
    pub len: u16,
    /// Offsets of the bytes left out of the signature, ascending.
    pub wild: Vec<u16>,
    /// Offsets of the absolute addresses (words) in the signature, ascending.
    pub reloc: Vec<u16>,
    /// The OUT sites: offset from the anchor, role (`SND_SITE_*`), channel.
    pub sites: Vec<(u16, u8, u8)>,
    /// T-states of one pass of the sound loop (0: one channel, or flips).
    pub pass: u16,
    /// Per channel: its OUT's T-state in a pass, from channel 0's (0 for one not given).
    pub pos: Vec<u16>,
    /// Each edge flips one channel's level.
    pub flips: bool,
    /// The drum's strength, for an entry with drum sites.
    pub drum: u8,
    /// The frame-end pcs (offset from the anchor, length) where the memory is not to be read.
    pub stale: Option<(u16, u16)>,
    /// Per channel: its loudness.
    pub level: Vec<u8>,
    /// Per channel (1-3 of them): the program of its period (T-states x 256; 0 or silent: off).
    pub progs: Vec<Vec<Op>>,
    /// The note's T-states so far from the registers given (the site's last edge or the frame's
    /// end); None: not known.
    pub timing: Option<Vec<Op>>,
}

impl LearnedEntry {
    /// The matching's work for this entry: each site's signature hashed (sites x len bytes;
    /// `SND_LEARNED_WORK_MAX` over a game's list).
    pub fn work(&self) -> usize {
        self.sites.len() * usize::from(self.len)
    }
}

/// Fixed part of an entry.
const HEAD: usize = 36;

fn encode_entry(e: &LearnedEntry) -> Vec<u8> {
    let channels = e.progs.len();
    let progs: Vec<Vec<u8>> = e.progs.iter().map(|p| encode_prog(p)).collect();
    let timing = e.timing.as_deref().map(encode_prog);
    let (stale_from, stale_len) = e.stale.unwrap_or((0, 0));
    let mut b = Vec::with_capacity(128);
    b.extend([0, 0, e.kind, channels as u8]);
    b.extend(e.digest.to_le_bytes());
    b.extend(e.len.to_le_bytes());
    b.extend([e.wild.len() as u8, e.reloc.len() as u8, e.sites.len() as u8, e.flips.into(), e.drum, timing.is_some().into()]);
    for w in [e.pass, stale_from, stale_len] {
        b.extend(w.to_le_bytes());
    }
    b.extend((0..3).map(|c| e.level.get(c).copied().unwrap_or(0)));
    b.extend([0, 0, 0]);
    b.extend((0..3).map(|c| progs.get(c).map_or(0, |p| p.len() as u8)));
    b.push(timing.as_ref().map_or(0, |t| t.len() as u8));
    debug_assert_eq!(b.len(), HEAD);
    for &w in e.wild.iter().chain(&e.reloc) {
        b.extend(w.to_le_bytes());
    }
    for &(off, role, chan) in &e.sites {
        b.extend(off.to_le_bytes());
        b.extend([role, chan]);
    }
    for c in 0..channels {
        b.extend(e.pos.get(c).copied().unwrap_or(0).to_le_bytes());
    }
    for p in progs.iter().chain(timing.iter()) {
        b.extend(p);
    }
    b.resize(b.len().next_multiple_of(4), 0);
    let size = b.len() as u16;
    b[..2].copy_from_slice(&size.to_le_bytes());
    b
}

/// A game's entry list. The encoder does not check the entries: the decoder refuses a list with
/// any that is not valid.
pub fn encode_list(entries: &[LearnedEntry]) -> Vec<u8> {
    let mut b = vec![entries.len() as u8, 0, 0, 0];
    for e in entries {
        b.extend(encode_entry(e));
    }
    b
}
