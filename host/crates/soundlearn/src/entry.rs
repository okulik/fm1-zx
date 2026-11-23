// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A learned entry (one music routine of a game), the stored roles for a built-in entry, a
//! recorded score, and the encoding of a game's entry list, as the pack's sound section holds it
//! and `snd_learned_decode` (`core/sound/learned.c`) reads it (docs/pack-format.md, "Entry list").
//!
//! The list (4-byte aligned; little-endian; sizes in bytes): the learned entries, then the kind-1
//! records, then the kind-2 records.
//!
//! ```text
//! u8 count (all kinds: <= 8 of kinds 0 and 1, <= 8 of kind 2), u8 0, u16 0, then count entries; a learned entry:
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
//! a kind-2 record (a recorded score; times in T-states from its trigger edge, T0):
//!   u16 size (multiple of 4, the whole record), u8 kind (2), u8 voices (1-3)
//!   u16 site, u16 quiet, u32 code, u16 buckets, u16 n_events, u8 role[3], u8 0
//!   u8 fingerprint[buckets], events[n_events], zero padding to size; an event:
//!     varint dt (64-T units since the event before or T0; LEB128, <= 4 bytes)
//!     u8 vf (bits 0-1 the voice, bit 7 a note off), a note on's u24 period (T), u8 level
//! ```
//!
//! Each kind has its own type and encoder ([`LearnedEntry`], [`BuiltinRoles`], [`ScoreRecord`]),
//! which writes its own kind byte.

use crate::prog::{encode_prog, Op};
use zx_sys::{zx_edge, ZX_FRAME_TSTATES, SND_TONAL};

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

/// A recorded score (a kind-2 record): a stretch of a game's music that plays the same every
/// time from its trigger edge (the first speaker edge of `site` after `quiet` frames with none
/// from it), as note events, with the speaker edges of each frame-long bucket from that edge
/// ([`score_fingerprint`]) that method B follows it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreRecord {
    /// The trigger `OUT`'s address (its first byte).
    pub site: u16,
    /// Frames with no speaker edge from `site` before the trigger edge, at least.
    pub quiet: u16,
    /// FNV-1a 32 of the 32 bytes from `site - 16` ([`score_code`]): the code must be the same.
    pub code: u32,
    /// Per voice: its stored role in the FM synth (`SND_ROLE_*`; 0: none).
    pub roles: [u8; SND_TONAL],
    /// The voices the events use (1-3).
    pub voices: u8,
    /// Per bucket: its speaker edges, capped at 255 (1 to `SND_SCORE_BUCKETS_MAX` buckets).
    pub fingerprint: Vec<u8>,
    /// The events, in time order.
    pub events: Vec<ScoreEvent>,
}

/// One event of a [`ScoreRecord`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreEvent {
    /// T-states since the event before (the first: since T0). The record holds times in 64-T
    /// units: the encoder rounds each event's time from T0, so the rounding does not add up.
    pub dt_t: u64,
    /// The voice (0-2, below the record's `voices`).
    pub voice: u8,
    /// A note on: its full period in T-states (64 to 2^24 - 1) and its level (1-255); a note on
    /// for a sounding voice changes its pitch. None: a note off.
    pub on: Option<(u32, u8)>,
}

/// FNV-1a 32 of the 32 bytes from `site - 16` (addresses wrapping), read by `peek`: a score's
/// `code`.
pub fn score_code(peek: impl Fn(u16) -> u8, site: u16) -> u32 {
    (0..32u16).fold(0x811c_9dc5u32, |h, i| (h ^ u32::from(peek(site.wrapping_sub(16).wrapping_add(i)))).wrapping_mul(0x0100_0193))
}

/// The speaker edges of a frame's edge list (EAR level changes; `start_level` the level before the
/// first edge): the edges a score's fingerprint and trigger count.
pub fn ear_edges(start_level: u8, edges: &[zx_edge]) -> impl Iterator<Item = &zx_edge> {
    let mut level = start_level;
    edges.iter().filter(move |e| {
        let ear = (e.level ^ level) & 2 != 0;
        level = e.level;
        ear
    })
}

/// A score's fingerprint: the speaker edges ([`ear_edges`]) of `buckets` frame-long buckets from
/// T0, T-state `t0` of frame `frame0` of `frames` (each frame's edges and the level before them),
/// each capped at 255. Buckets past the frames given count what they hold.
pub fn score_fingerprint(frames: &[(Vec<zx_edge>, u8)], frame0: usize, t0: u32, buckets: usize) -> Vec<u8> {
    let mut fp = vec![0u32; buckets];
    for (f, (edges, level)) in frames.iter().enumerate().skip(frame0) {
        for e in ear_edges(*level, edges) {
            let t = (f - frame0) as u64 * u64::from(ZX_FRAME_TSTATES) + u64::from(e.tstate);
            let Some(b) = t.checked_sub(u64::from(t0)).map(|d| (d / u64::from(ZX_FRAME_TSTATES)) as usize) else { continue };
            if let Some(x) = fp.get_mut(b) {
                *x += 1;
            }
        }
    }
    fp.into_iter().map(|x| x.min(255) as u8).collect()
}

/// Fixed part of an entry.
const HEAD: usize = 36;
/// The kind byte of each.
const KIND_LEARNED: u8 = 0;
const KIND_ROLES: u8 = 1;
const KIND_SCORE: u8 = 2;
/// A kind-2 record's fixed part.
const SCORE_HEAD: usize = 20;
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

fn encode_score(r: &ScoreRecord) -> Vec<u8> {
    let mut b = vec![0, 0, KIND_SCORE, r.voices];
    for w in [r.site, r.quiet] {
        b.extend(w.to_le_bytes());
    }
    b.extend(r.code.to_le_bytes());
    for w in [r.fingerprint.len() as u16, r.events.len() as u16] {
        b.extend(w.to_le_bytes());
    }
    b.extend(r.roles);
    b.push(0);
    debug_assert_eq!(b.len(), SCORE_HEAD);
    b.extend(&r.fingerprint);
    // each event's time from T0, rounded to 64-T units; dt the units since the event before
    let (mut t, mut prev) = (0u64, 0u64);
    for e in &r.events {
        t += e.dt_t;
        let u = (t + 32) / 64;
        let mut dt = u - prev;
        prev = u;
        loop {
            let more = dt >= 0x80;
            b.push((dt & 0x7F) as u8 | u8::from(more) << 7);
            dt >>= 7;
            if !more {
                break;
            }
        }
        match e.on {
            Some((period, level)) => {
                b.push(e.voice & 3);
                b.extend(&period.to_le_bytes()[..3]);
                b.push(level);
            }
            None => b.push(e.voice & 3 | 0x80),
        }
    }
    b.resize(b.len().next_multiple_of(4), 0);
    let size = b.len() as u16;
    b[..2].copy_from_slice(&size.to_le_bytes());
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
    encode_list_scores(entries, roles, &[])
}

/// A game's entry list: the learned entries, the built-in entries' roles (kind-1 records), then
/// the recorded scores (kind-2 records). Without scores, as [`encode_list_with`]. The encoder does
/// not check them: the decoder refuses a list with any that is not valid (or with over 8 of kinds
/// 0 and 1, or over 8 scores).
pub fn encode_list_scores(entries: &[LearnedEntry], roles: &[BuiltinRoles], scores: &[ScoreRecord]) -> Vec<u8> {
    let mut b = vec![(entries.len() + roles.len() + scores.len()) as u8, 0, 0, 0];
    for e in entries {
        b.extend(encode_entry(e));
    }
    for r in roles {
        b.extend(encode_roles(r));
    }
    for r in scores {
        b.extend(encode_score(r));
    }
    b
}
