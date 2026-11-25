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
//! a kind-2 record (a recorded score: an intro and optionally a loop, times from each section's start):
//!   u16 size (multiple of 4, the whole record), u8 kind (2), u8 voices (bits 0-5: 1-3 tonal voices;
//!     bit 6: a replacement, its events on their own timeline, only with bit 7;
//!     bit 7: precedence, the score keeps the frames the table claims too)
//!   u16 site, u16 quiet, u32 code, u32 intro_t, u32 loop_t (0: no loop)
//!   u16 intro_buckets, u16 loop_buckets (each section's whole buckets), u16 intro_events, u16 loop_events
//!   a replacement's alone: u32 ev_intro_t, u32 ev_loop_t (its event sections; ev_loop_t 0: no event loop)
//!   u8 n_pitches (1-255), u8 role[3], u24 pitch[n_pitches] (full periods, T)
//!   fingerprint: the intro's buckets then the loop's, one nibble stream (low nibble first; a spare
//!     nibble 0 to a whole byte): each section's first bucket nibble 8 and the count (two nibbles,
//!     low first), each next the change (-7..+7, 4-bit signed) or nibble 8 and the count
//!   events: the intro's, then the loop's (a replacement's: its own sections'); an event:
//!     varint dt (64-T units since the section's event before or its start; LEB128, <= 4 bytes)
//!     u8 b (bits 0-1 the voice; bits 2-7: 0 a note off, else a note on of level b >> 2), a note on's u8 pitch index
//!     (voice 3 the drum: bits 2-7 0 the drum off, else a drum hit of level b >> 2; no pitch byte)
//!   zero padding to size
//! ```
//!
//! Each kind has its own type and encoder ([`LearnedEntry`], [`BuiltinRoles`], [`ScoreRecord`]),
//! which writes its own kind byte. [`decode_scores`] reads a list's kind-2 records back,
//! [`replace_scores`] puts others in their place, and [`score_key`] names a score.

use crate::prog::{encode_prog, Op};
use zx_sys::{zx_edge, SND_LEARNED_MAX, SND_SCORES_MAX, SND_SCORE_BUCKETS_MAX, SND_TONAL, ZX_FRAME_TSTATES};

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
/// time from its trigger edge, T0 (the first speaker edge of `site` after `quiet` frames with
/// none from it): an intro, then optionally a loop that repeats, as note events, with the speaker
/// edges of each frame-long bucket of each section ([`score_fingerprint`]) that method B follows
/// it by, round the loop for as long as the live edges agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreRecord {
    /// The trigger `OUT`'s address (its first byte).
    pub site: u16,
    /// Frames with no speaker edge from `site` before the trigger edge, at least.
    pub quiet: u16,
    /// FNV-1a 32 of the 32 bytes from `site - 16` ([`score_code`]): the code must be the same.
    pub code: u32,
    /// The tonal voices the events use (1-3); the drum ([`SCORE_DRUM`]) is not counted.
    pub voices: u8,
    /// Precedence over method B's table: the score keeps every frame it plays, those a table
    /// entry claims too (bit 7 of the record's voices byte). The personal pack sets it on the
    /// chosen take of a shadow score (music a learned or built-in entry plays).
    pub precedence: bool,
    /// A replacement's own event timeline, `(ev_intro_t, ev_loop_t)` in T (bit 6 of the record's
    /// voices byte; the decoder takes it only with `precedence`). With it, `intro_events` are
    /// timed from T0 below `ev_intro_t` and `loop_events` from each pass of the events' own loop
    /// (pass k from T0 + `ev_intro_t` + k x `ev_loop_t`) below `ev_loop_t`, whatever the
    /// fingerprint's sections; `ev_loop_t` 0: no event loop, the events end at `ev_intro_t`. The
    /// fingerprint (`intro_t`, `loop_t`, the buckets) is followed as any score's, and the score
    /// plays as long as it is. `ev_intro_t + ev_loop_t` is 1 to `SND_SCORE_BUCKETS_MAX` x
    /// 69,888, and `ev_loop_t` 0 or at least 69,888. None: the events are on the fingerprint's
    /// sections ([`ScoreRecord::event_lengths`]).
    pub replacement: Option<(u32, u32)>,
    /// Per voice: its stored role in the FM synth (`SND_ROLE_*`; 0: none).
    pub roles: [u8; SND_TONAL],
    /// The intro's length: T from T0 (with no loop, the whole score).
    pub intro_t: u32,
    /// One pass of the loop: T (0: no loop). Pass k starts at T0 + `intro_t` + k x `loop_t`.
    pub loop_t: u32,
    /// The intro's whole buckets from T0: each one's speaker edges, capped at 255
    /// (`intro_t / ZX_FRAME_TSTATES` of them).
    pub intro_fp: Vec<u8>,
    /// The loop's whole buckets from a pass's start (`loop_t / ZX_FRAME_TSTATES` of them).
    pub loop_fp: Vec<u8>,
    /// The intro's events, in time order, below `intro_t` (a replacement's: below its `ev_intro_t`).
    pub intro_events: Vec<ScoreEvent>,
    /// One pass's events, in time order, below `loop_t` (a replacement's: below its `ev_loop_t`, on
    /// the passes of its own loop): its first restate every voice sounding
    /// at its start (a note on at 0; the player releases at a pass's start the voices they do
    /// not restate).
    pub loop_events: Vec<ScoreEvent>,
}

impl ScoreRecord {
    /// The lengths in T of the sections the events are timed in, `(intro, loop)`: a replacement's
    /// own, else the fingerprint's (`intro_t`, `loop_t`). Loop 0: no event loop.
    pub fn event_lengths(&self) -> (u32, u32) {
        self.replacement.unwrap_or((self.intro_t, self.loop_t))
    }
}

/// One event of a [`ScoreRecord`]: a tonal voice's note on or off, or a drum hit or off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreEvent {
    /// T-states since its section's start (T0, or its pass's). The record holds times in 64-T
    /// units: the encoder rounds each event's time from its section's start, so the rounding does
    /// not add up.
    pub t: u32,
    /// The voice: 0-2 a tonal voice (below the record's `voices`), [`SCORE_DRUM`] (3) the drum
    /// (method B's drum voice), whatever `voices` is.
    pub voice: u8,
    /// A note on: its full period in T-states (64 to 2^24 - 1) and its level (1-255, stored as
    /// (level + 2) / 4 of 1-63, which the device plays as that x 4 + 3); a note on for a sounding
    /// voice changes its pitch. None: a note off. For the drum: a hit of that level (the same
    /// steps), its period not stored (0 read back); None the drum off. A hit while the drum
    /// sounds does not strike again on the device (its FM drum starts only from off): a drum note
    /// is a hit and an off.
    pub on: Option<(u32, u8)>,
}

/// A score event's voice for the drum ([`ScoreEvent::voice`]).
pub const SCORE_DRUM: u8 = 3;

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

/// A section's fingerprint: the speaker edges ([`ear_edges`]) of each whole frame-long bucket of
/// `[from_t, from_t + len_t)` T from T0, T-state `t0` of frame 0 of `frames` (each frame's edges
/// and the level before them), each capped at 255; buckets past the frames given count what they
/// hold.
pub fn score_fingerprint(frames: &[(Vec<zx_edge>, u8)], t0: u32, from_t: u64, len_t: u64) -> Vec<u8> {
    let f = u64::from(ZX_FRAME_TSTATES);
    let mut fp = vec![0u32; (len_t / f) as usize];
    for (k, (edges, level)) in frames.iter().enumerate() {
        for e in ear_edges(*level, edges) {
            let t = k as u64 * f + u64::from(e.tstate);
            let Some(d) = t.checked_sub(u64::from(t0) + from_t) else { continue };
            if let Some(x) = fp.get_mut((d / f) as usize) {
                *x += 1;
            }
        }
    }
    fp.into_iter().map(|x| x.min(255) as u8).collect()
}

/// Fixed part of an entry.
const HEAD: usize = 36;
/// A kind-2 record's voices byte: the precedence bit, the replacement bit.
const PRECEDENCE: u8 = 0x80;
const REPLACEMENT: u8 = 0x40;
/// A replacement's event sections together, at most (T).
const EV_MAX: u64 = SND_SCORE_BUCKETS_MAX as u64 * ZX_FRAME_TSTATES as u64;
/// The kind byte of each.
const KIND_LEARNED: u8 = 0;
const KIND_ROLES: u8 = 1;
const KIND_SCORE: u8 = 2;
/// A kind-2 record's fixed part (a replacement's: 8 bytes more).
const SCORE_HEAD: usize = 32;
const REPL_HEAD: usize = SCORE_HEAD + 8;
/// A score's fingerprint: the nibble before a count.
const ESC: u8 = 8;
/// A score's most pitches.
const PITCHES_MAX: usize = 255;
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

/// A score's pitch table: its note ons' periods, each once, ascending; over [`PITCHES_MAX`], the
/// two nearest (by cents) merged, the one more notes use kept, until that many. With no note on,
/// one pitch (unused).
fn score_pitches(r: &ScoreRecord) -> Vec<u32> {
    let mut uses: Vec<(u32, usize)> = Vec::new();
    for p in r.intro_events.iter().chain(&r.loop_events).filter(|e| e.voice & 3 != SCORE_DRUM).filter_map(|e| e.on.map(|o| o.0 & 0xFF_FFFF)) {
        match uses.binary_search_by_key(&p, |u| u.0) {
            Ok(i) => uses[i].1 += 1,
            Err(i) => uses.insert(i, (p, 1)),
        }
    }
    while uses.len() > PITCHES_MAX {
        let gap = |i: usize| f64::from(uses[i + 1].0.max(1)).log2() - f64::from(uses[i].0.max(1)).log2();
        let i = (0..uses.len() - 1).min_by(|&a, &b| gap(a).total_cmp(&gap(b))).unwrap_or(0);
        let keep = if uses[i + 1].1 > uses[i].1 { uses[i + 1].0 } else { uses[i].0 };
        uses[i] = (keep, uses[i].1 + uses[i + 1].1);
        uses.remove(i + 1);
    }
    if uses.is_empty() {
        return vec![64];
    }
    uses.into_iter().map(|u| u.0).collect()
}

/// A note on's level (1-255) as a score stores it: 1-63.
fn score_level6(level: u8) -> u8 {
    ((u32::from(level) + 2) / 4).clamp(1, 63) as u8
}

/// A note on's level (1-255) as the device plays it from a score: the stored level x 4 + 3.
pub fn score_level(level: u8) -> u8 {
    score_level6(level) * 4 + 3
}

/// A score's fingerprint nibbles: each section's buckets, its first an escape and the count.
fn score_nibbles(r: &ScoreRecord) -> Vec<u8> {
    let mut n = Vec::new();
    for fp in [&r.intro_fp, &r.loop_fp] {
        let mut prev: Option<u8> = None;
        for &c in fp.iter() {
            let d = prev.map_or(i32::from(ESC), |p| i32::from(c) - i32::from(p));
            if prev.is_some() && (-7..=7).contains(&d) {
                n.push((d & 15) as u8);
            } else {
                n.extend([ESC, c & 15, c >> 4]);
            }
            prev = Some(c);
        }
    }
    n
}

/// A section's events (`len` T long) into `b`: each time rounded from the section's start to
/// 64-T units (the last unit inside the section at most).
fn score_events_into(b: &mut Vec<u8>, events: &[ScoreEvent], len: u32, pitches: &[u32]) {
    let last = (len.max(1) - 1) / 64;
    let index = |p: u32| -> u8 {
        let cents = |q: u32| (f64::from(q.max(1)).log2() - f64::from(p.max(1)).log2()).abs();
        (0..pitches.len()).min_by(|&a, &b| cents(pitches[a]).total_cmp(&cents(pitches[b]))).unwrap_or(0) as u8
    };
    let mut prev = 0u32;
    for e in events {
        let u = (e.t.saturating_add(32) / 64).min(last);
        let mut dt = u.saturating_sub(prev);
        prev = prev.max(u);
        loop {
            let more = dt >= 0x80;
            b.push((dt & 0x7F) as u8 | u8::from(more) << 7);
            dt >>= 7;
            if !more {
                break;
            }
        }
        match e.on {
            Some((_, level)) if e.voice & 3 == SCORE_DRUM => b.push(SCORE_DRUM | score_level6(level) << 2),
            Some((period, level)) => {
                b.extend([e.voice & 3 | score_level6(level) << 2, index(period & 0xFF_FFFF)]);
            }
            None => b.push(e.voice & 3),
        }
    }
}

fn encode_score(r: &ScoreRecord) -> Vec<u8> {
    let pitches = score_pitches(r);
    let flags = if r.precedence { PRECEDENCE } else { 0 } | if r.replacement.is_some() { REPLACEMENT } else { 0 };
    let mut b = vec![0, 0, KIND_SCORE, r.voices | flags];
    for w in [r.site, r.quiet] {
        b.extend(w.to_le_bytes());
    }
    for d in [r.code, r.intro_t, r.loop_t] {
        b.extend(d.to_le_bytes());
    }
    for w in [r.intro_fp.len(), r.loop_fp.len(), r.intro_events.len(), r.loop_events.len()] {
        b.extend((w as u16).to_le_bytes());
    }
    if let Some((ev_intro_t, ev_loop_t)) = r.replacement {
        b.extend(ev_intro_t.to_le_bytes());
        b.extend(ev_loop_t.to_le_bytes());
    }
    b.push(pitches.len() as u8);
    b.extend(r.roles);
    debug_assert_eq!(b.len(), if r.replacement.is_some() { REPL_HEAD } else { SCORE_HEAD });
    for p in &pitches {
        b.extend(&p.to_le_bytes()[..3]);
    }
    let nib = score_nibbles(r);
    b.extend(nib.chunks(2).map(|c| c[0] | c.get(1).map_or(0, |h| h << 4)));
    let (ev_intro_t, ev_loop_t) = r.event_lengths();
    score_events_into(&mut b, &r.intro_events, ev_intro_t, &pitches);
    score_events_into(&mut b, &r.loop_events, ev_loop_t, &pitches);
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

/// The entries of an encoded list: (offset, size, kind) each, checked as `snd_learned_decode`
/// checks a list's structure (the header, each entry's size and kind within the list, at most 8
/// of kinds 0 and 1 and 8 of kind 2); nothing inside the entries.
fn list_entries(list: &[u8]) -> Result<Vec<(usize, usize, u8)>, String> {
    if list.len() < 4 {
        return Err(format!("a list of {} bytes", list.len()));
    }
    let mut out = Vec::new();
    let mut off = 4;
    for k in 0..usize::from(list[0]) {
        if list.len() - off < 4 {
            return Err(format!("entry {k} past the list"));
        }
        let size = usize::from(u16::from_le_bytes([list[off], list[off + 1]]));
        let kind = list[off + 2];
        let min = match kind {
            KIND_LEARNED => HEAD,
            KIND_ROLES => usize::from(ROLES_SIZE),
            KIND_SCORE => SCORE_HEAD,
            _ => return Err(format!("entry {k}: kind {kind}")),
        };
        if size < min || size % 4 != 0 || size > list.len() - off {
            return Err(format!("entry {k} (kind {kind}): size {size}"));
        }
        out.push((off, size, kind));
        off += size;
    }
    let scores = out.iter().filter(|e| e.2 == KIND_SCORE).count();
    if out.len() - scores > SND_LEARNED_MAX || scores > SND_SCORES_MAX {
        return Err(format!("{} entries of kinds 0 and 1, {scores} scores", out.len() - scores));
    }
    Ok(out)
}

/// A kind-2 record (`r`, its `size` bytes) read back, checked as `snd_learned_decode` checks it.
fn decode_score(r: &[u8]) -> Result<ScoreRecord, String> {
    let w = |at: usize| u16::from_le_bytes([r[at], r[at + 1]]);
    let d = |at: usize| u32::from_le_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]]);
    let size = r.len();
    let (voices, precedence, replaced) = (r[3] & !(PRECEDENCE | REPLACEMENT), r[3] & PRECEDENCE != 0, r[3] & REPLACEMENT != 0);
    let (intro_t, loop_t) = (d(12), d(16));
    let buckets = [usize::from(w(20)), usize::from(w(22))];
    if replaced && !precedence {
        return Err("a replacement without precedence".into());
    }
    if replaced && size < REPL_HEAD {
        return Err(format!("a replacement of {size} bytes"));
    }
    let replacement = replaced.then(|| (d(28), d(32)));
    let head = if replaced { REPL_HEAD } else { SCORE_HEAD };
    let np = usize::from(r[head - 4]);
    if !(1..=SND_TONAL as u8).contains(&voices) {
        return Err(format!("{voices} voices"));
    }
    if buckets[0] as u32 != intro_t / ZX_FRAME_TSTATES || buckets[1] as u32 != loop_t / ZX_FRAME_TSTATES || buckets[0] + buckets[1] > SND_SCORE_BUCKETS_MAX {
        return Err(format!("buckets {buckets:?} for an intro of {intro_t} T and a loop of {loop_t} T"));
    }
    if (intro_t == 0 && loop_t == 0) || (loop_t != 0 && buckets[1] == 0) {
        return Err(format!("an intro of {intro_t} T and a loop of {loop_t} T"));
    }
    if let Some((ei, el)) = replacement {
        if (ei == 0 && el == 0) || u64::from(ei) + u64::from(el) > EV_MAX || (el != 0 && el < ZX_FRAME_TSTATES) {
            return Err(format!("event sections of {ei} T and {el} T"));
        }
    }
    let (ev_intro_t, ev_loop_t) = replacement.unwrap_or((intro_t, loop_t));
    let fp0 = head + 3 * np;
    if np == 0 || fp0 > size {
        return Err(format!("{np} pitches"));
    }
    let pitches: Vec<u32> = (0..np).map(|k| u32::from_le_bytes([r[head + 3 * k], r[head + 3 * k + 1], r[head + 3 * k + 2], 0])).collect();
    if let Some(p) = pitches.iter().find(|&&p| p < 64) {
        return Err(format!("a pitch of {p} T"));
    }
    // the fingerprint: a nibble stream from fp0
    let nibbles = 2 * (size - fp0);
    let nib = |i: usize| (r[fp0 + i / 2] >> (4 * (i % 2))) & 15;
    let mut i = 0;
    let mut prev = 0i32;
    let mut fps = [Vec::new(), Vec::new()];
    for (fp, &n) in fps.iter_mut().zip(&buckets) {
        for k in 0..n {
            if i >= nibbles {
                return Err("the fingerprint past the record".into());
            }
            let c = nib(i);
            i += 1;
            prev = if c == ESC {
                if i + 2 > nibbles {
                    return Err("the fingerprint past the record".into());
                }
                i += 2;
                i32::from(nib(i - 2) | nib(i - 1) << 4)
            } else if k == 0 {
                return Err("a section's first bucket not an escape".into());
            } else {
                prev + i32::from(c) - if c & 8 != 0 { 16 } else { 0 }
            };
            fp.push(u8::try_from(prev).map_err(|_| format!("a bucket of {prev} edges"))?);
        }
    }
    if i % 2 == 1 && nib(i) != 0 {
        return Err("the fingerprint's spare nibble not 0".into());
    }
    // the events
    let mut at = fp0 + i.div_ceil(2);
    let mut sections = [Vec::new(), Vec::new()];
    for (s, ev) in sections.iter_mut().enumerate() {
        let (n, len) = [(w(24), ev_intro_t), (w(26), ev_loop_t)][s];
        let units = u64::from(len).div_ceil(64);
        let mut u = 0u64;
        for k in 0..n {
            let mut dt = 0u64;
            let mut done = false;
            for sh in 0..4 {
                let Some(&c) = r.get(at) else { break };
                at += 1;
                dt |= u64::from(c & 0x7F) << (7 * sh);
                if c & 0x80 == 0 {
                    done = true;
                    break;
                }
            }
            u += dt;
            if !done || u >= units || at >= size {
                return Err(format!("section {s}, event {k}: its time or its byte past the section or the record"));
            }
            let b = r[at];
            at += 1;
            let (voice, level) = (b & 3, (b >> 2) * 4);
            let on = if voice == SCORE_DRUM {
                (level != 0).then_some((0, level))
            } else if voice >= voices {
                return Err(format!("section {s}, event {k}: voice {voice} of {voices}"));
            } else if level != 0 {
                let p = r.get(at).map(|&p| usize::from(p)).filter(|&p| p < np).ok_or_else(|| format!("section {s}, event {k}: its pitch"))?;
                at += 1;
                Some((pitches[p], level))
            } else {
                None
            };
            ev.push(ScoreEvent { t: (u * 64) as u32, voice, on });
        }
    }
    let [intro_fp, loop_fp] = fps;
    let [intro_events, loop_events] = sections;
    Ok(ScoreRecord {
        site: w(4),
        quiet: w(6),
        code: d(8),
        voices,
        precedence,
        replacement,
        roles: [r[head - 3], r[head - 2], r[head - 1]],
        intro_t,
        loop_t,
        intro_fp,
        loop_fp,
        intro_events,
        loop_events,
    })
}

/// The recorded scores (kind-2 records) of an encoded entry list, in list order, read back as
/// [`ScoreRecord`]s: the inverse of the encoder up to what it rounds. Checked as
/// `snd_learned_decode` checks the list's structure and its kind-2 records (an error names what
/// it would refuse); the learned entries and kind-1 records inside are not looked at.
///
/// What a record keeps of the [`ScoreRecord`] it was encoded from (the rest comes back exactly):
/// - each event's time as a multiple of 64 T (the encoder's 64-T unit of it);
/// - each level as its step x 4 (4-252: the encoder's `(level + 2) / 4` of 1-63 x 4, which it
///   stores back as the same step; the device plays the step x 4 + 3, [`score_level`]);
/// - each note on's period as the pitch table's (the encoder's table holds every period exactly
///   up to 255 different ones; past that, merged to the nearest kept), within 24 bits;
/// - a drum hit's period as 0;
/// - the events in the order stored (the encoder keeps the order given and moves a time that
///   goes back to the event before's).
///
/// So the encoder writes a decoded record back byte for byte: [`replace_scores`] with the scores
/// a list decodes to gives that list.
pub fn decode_scores(list: &[u8]) -> Result<Vec<ScoreRecord>, String> {
    list_entries(list)?
        .into_iter()
        .filter(|e| e.2 == KIND_SCORE)
        .enumerate()
        .map(|(k, (off, size, _))| decode_score(&list[off..off + size]).map_err(|e| format!("score {}: {e}", k + 1)))
        .collect()
}

/// The encoded entry list `list` with its recorded scores (kind-2 records) replaced by `scores`,
/// the first by the first and so on: the learned entries, the kind-1 records, the header and any
/// bytes after the last entry stay byte for byte. Errors: a list [`decode_scores`] refuses, a
/// count of `scores` other than the list's scores, a record over 65,532 bytes, or a new record
/// the decoder would refuse (each checked as [`decode_scores`] checks it).
pub fn replace_scores(list: &[u8], scores: &[ScoreRecord]) -> Result<Vec<u8>, String> {
    decode_scores(list)?;
    let entries = list_entries(list)?;
    let n = entries.iter().filter(|e| e.2 == KIND_SCORE).count();
    if n != scores.len() {
        return Err(format!("{} scores for a list of {n}", scores.len()));
    }
    let mut b = list[..4].to_vec();
    let mut next = scores.iter().enumerate();
    let mut end = 4;
    for &(off, size, kind) in &entries {
        if kind == KIND_SCORE {
            let (k, s) = next.next().expect("a score for each record");
            let r = encode_score(s);
            if r.len() > 0xFFFC {
                return Err(format!("score {}: {} bytes", k + 1, r.len()));
            }
            decode_score(&r).map_err(|e| format!("score {}: {e}", k + 1))?;
            b.extend(r);
        } else {
            b.extend(&list[off..off + size]);
        }
        end = off + size;
    }
    b.extend(&list[end..]);
    Ok(b)
}

/// The encoded entry list `list` with the score `rec` added after its last entry (the list's
/// count one more; any bytes after the last entry stay after it). Errors: a list
/// [`decode_scores`] refuses, a list that would hold over [`SND_SCORES_MAX`] scores or 255
/// entries, a record over 65,532 bytes, or one the decoder would refuse.
pub fn append_score(list: &[u8], rec: &ScoreRecord) -> Result<Vec<u8>, String> {
    decode_scores(list)?;
    let entries = list_entries(list)?;
    let n = entries.iter().filter(|e| e.2 == KIND_SCORE).count();
    if n >= SND_SCORES_MAX || entries.len() >= 255 {
        return Err(format!("the list holds {n} scores already (at most {SND_SCORES_MAX})"));
    }
    let r = encode_score(rec);
    if r.len() > 0xFFFC {
        return Err(format!("its record would take {} bytes", r.len()));
    }
    decode_score(&r)?;
    let end = entries.last().map_or(4, |&(off, size, _)| off + size);
    let mut b = list[..end].to_vec();
    b[0] += 1;
    b.extend(r);
    b.extend(&list[end..]);
    Ok(b)
}

/// A score's key, stable while the learner records the same music: its site as 4 uppercase hex
/// digits, `-`, and the CRC-32 (IEEE) of its intro's fingerprint then its loop's as 8 lowercase
/// hex digits (e.g. `8951-1a2b3c4d`).
pub fn score_key(rec: &ScoreRecord) -> String {
    format!("{:04X}-{:08x}", rec.site, crc32(rec.intro_fp.iter().chain(&rec.loop_fp)))
}

/// CRC-32 (IEEE: reflected, polynomial EDB88320h, initial value and final XOR FFFFFFFFh).
fn crc32<'a>(bytes: impl Iterator<Item = &'a u8>) -> u32 {
    !bytes.fold(!0u32, |mut c, &b| {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & (c & 1).wrapping_neg());
        }
        c
    })
}
