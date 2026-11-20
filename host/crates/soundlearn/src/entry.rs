// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A learned entry (one music routine of a game), the stored roles for a built-in entry, and the
//! encoding of a game's entry list, as the pack's sound section holds it and `snd_learned_decode`
//! (`core/sound/learned.c`) reads it (docs/pack-format.md, "Entry list").
//!
//! The list (4-byte aligned; little-endian; sizes in bytes): the learned entries, then the kind-1
//! records.
//!
//! ```text
//! u8 count (<= 8, both kinds), u8 0, u16 0, then count entries; a learned entry:
//!   u16 size (multiple of 4, the whole entry), u8 kind (0: a learned entry), u8 channels (1-3)
//!   u64 digest
//!   u16 len, u8 n_wild, u8 n_reloc
//!   u8 n_sites, u8 flags (0, 1: flips, 2: observed), u8 drum, u8 timing (1: timing program present)
//!   u16 pass, u16 stale_from, u16 stale_len   (stale: anchor-relative pc range; len 0 = none)
//!   u8 level[3], u8 instrument[3] (each channel's stored role: 0 none, 1 bass, 2 lead, 3 pad)
//!   u8 prog_len[3], u8 timing_len
//!   u16 wild[n_wild], u16 reloc[n_reloc], {u16 off, u8 role, u8 chan} sites[n_sites], u16 pos[channels]
//!   channel programs (prog_len[c] bytes each, c < channels), timing program, zero padding to size
//! a kind-1 record (roles for a built-in entry):
//!   u16 size (16), u8 kind (1), u8 channels (1-3), u64 digest (the built-in entry's), u8 role[3], u8 0
//! ```
//!
//! Each kind has its own type and encoder ([`LearnedEntry`], [`BuiltinRoles`]), which writes its
//! own kind byte.

use crate::prog::{encode_prog, Op};
use zx_sys::SND_TONAL;

/// One learned routine: what a built-in `snd_engine` carries, with programs for its reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearnedEntry {
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
    /// The routine mixes its channels into the one speaker bit (learned by observation, verified
    /// on the host): method B takes its frames with the voices read, without its edge check. Its
    /// tonal sites are all channel 0; not with `flips` (the decoder refuses both).
    pub observed: bool,
    /// The drum's strength, for an entry with drum sites.
    pub drum: u8,
    /// The frame-end pcs (offset from the anchor, length) where the memory is not to be read.
    pub stale: Option<(u16, u16)>,
    /// Per channel: its loudness.
    pub level: Vec<u8>,
    /// Per channel: its stored role in the FM synth (`SND_ROLE_*`; 0: none, the live rule's).
    pub instrument: [u8; SND_TONAL],
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

/// The stored roles of a built-in entry in one game (a kind-1 record): the built-in entry plays
/// that game's music with these roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinRoles {
    /// The built-in entry's digest (`snd_engine.digest`), which names it.
    pub digest: u64,
    /// Its channels (1-3).
    pub channels: u8,
    /// Per channel: the role (`SND_ROLE_*`; 0: none).
    pub roles: [u8; SND_TONAL],
}

/// Fixed part of an entry.
const HEAD: usize = 36;
/// The kind byte of each.
const KIND_LEARNED: u8 = 0;
const KIND_ROLES: u8 = 1;
/// A kind-1 record's size: 15 bytes and a byte of padding.
const ROLES_SIZE: u8 = 16;

fn encode_entry(e: &LearnedEntry) -> Vec<u8> {
    let channels = e.progs.len();
    let progs: Vec<Vec<u8>> = e.progs.iter().map(|p| encode_prog(p)).collect();
    let timing = e.timing.as_deref().map(encode_prog);
    let (stale_from, stale_len) = e.stale.unwrap_or((0, 0));
    let mut b = Vec::with_capacity(128);
    b.extend([0, 0, KIND_LEARNED, channels as u8]);
    b.extend(e.digest.to_le_bytes());
    b.extend(e.len.to_le_bytes());
    b.extend([e.wild.len() as u8, e.reloc.len() as u8, e.sites.len() as u8, u8::from(e.flips) | u8::from(e.observed) << 1, e.drum, timing.is_some().into()]);
    for w in [e.pass, stale_from, stale_len] {
        b.extend(w.to_le_bytes());
    }
    b.extend((0..3).map(|c| e.level.get(c).copied().unwrap_or(0)));
    b.extend(e.instrument);
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

fn encode_roles(r: &BuiltinRoles) -> Vec<u8> {
    let mut b = vec![ROLES_SIZE, 0, KIND_ROLES, r.channels];
    b.extend(r.digest.to_le_bytes());
    b.extend(r.roles);
    b.push(0);
    debug_assert_eq!(b.len(), usize::from(ROLES_SIZE));
    b
}

/// A game's entry list of learned entries alone.
pub fn encode_list(entries: &[LearnedEntry]) -> Vec<u8> {
    encode_list_with(entries, &[])
}

/// A game's entry list: the learned entries, then the built-in entries' roles (kind-1 records).
/// The encoder does not check them: the decoder refuses a list with any that is not valid (or
/// with over 8 of both kinds).
pub fn encode_list_with(entries: &[LearnedEntry], roles: &[BuiltinRoles]) -> Vec<u8> {
    let mut b = vec![(entries.len() + roles.len()) as u8, 0, 0, 0];
    for e in entries {
        b.extend(encode_entry(e));
    }
    for r in roles {
        b.extend(encode_roles(r));
    }
    b
}
