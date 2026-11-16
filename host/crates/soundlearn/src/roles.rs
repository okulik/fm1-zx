// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The stored roles' analysis: each channel of a routine gets a role in the FM synth (bass, lead
//! or pad) by its pitch over the learner's run, stored in the pack (docs/pack-format.md: a learned
//! entry's `instrument[]`, a built-in entry's kind-1 record).
//!
//! Over the frames method B gives the routine ([`rank`]): per channel, the mean of log2 of its
//! period over the frames it sounds in (the last pitch it sounds in each frame, its events
//! included, as the FM-1's live rule tracks a voice, `core/sound/roles.c`); then the channels
//! ranked as the live rule ranks its voices: the lowest pitch the bass, the highest the lead, a
//! third the pad, a lone one the lead (equal means: the lower channel ranks lower, as the live
//! rule's starting order). A channel that never sounds gets none. A routine whose frames do not
//! pass the music gate ([`crate::gate`]: too few, or not music) gets none.
//!
//! A learned routine's frames are those its replay gives it ([`crate::gate::ReplayStats`]); a
//! built-in routine's are those the built-in table gives it while the learner listens
//! ([`BuiltinRun`]: no run of its own).

use crate::gate::{music_gate, sounded_notes};
use std::collections::{BTreeSet, VecDeque};
use zx_sys::{Machine, SndEngines, SND_DELAY_FRAMES, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD, SND_TONAL};

/// Each frame's notes (per voice, its period; 0 silent) in the frames given to the routine; None
/// where it is not given the frame, or is silent.
pub type Notes = [Option<[u32; SND_TONAL]>];

/// The channels' roles over `notes` (see the module docs); None if they do not pass the music
/// gate.
pub fn rank(notes: &Notes) -> Option<[u8; SND_TONAL]> {
    if !music_gate(notes) {
        return None;
    }
    let (mut sum, mut n) = ([0f64; SND_TONAL], [0u32; SND_TONAL]);
    for v in notes.iter().flatten() {
        for c in (0..SND_TONAL).filter(|&c| v[c] != 0) {
            sum[c] += f64::from(v[c]).log2();
            n[c] += 1;
        }
    }
    let mean = |c: usize| sum[c] / f64::from(n[c]);
    // the lowest pitch (the longest period) first; a stable sort keeps equal ones in channel order
    let mut order: Vec<usize> = (0..SND_TONAL).filter(|&c| n[c] > 0).collect();
    order.sort_by(|&a, &b| mean(b).total_cmp(&mean(a)));
    let mut roles = [0; SND_TONAL];
    for (k, &c) in order.iter().enumerate() {
        roles[c] = if k + 1 == order.len() {
            SND_ROLE_LEAD
        } else if k == 0 {
            SND_ROLE_BASS
        } else {
            SND_ROLE_PAD
        };
    }
    Some(roles)
}

/// Method B with the built-in table alone, alongside the learner's listening run: per built-in
/// entry, the notes of the frames it is given and the speaker `OUT`s that sounded in them.
pub struct BuiltinRun {
    b: SndEngines,
    /// Per built-in entry: each decided frame's notes (see [`Notes`]).
    notes: Vec<Vec<Option<[u32; SND_TONAL]>>>,
    /// Per built-in entry: the `OUT`s with an edge in a frame it is given.
    outs: Vec<BTreeSet<u16>>,
    /// The `OUT`s with an edge in each of the last frames, oldest first: a frame's decision comes
    /// `SND_DELAY_FRAMES` frames after it.
    recent: VecDeque<Vec<u16>>,
}

/// What [`BuiltinRun`] found of one built-in entry the run gave frames to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinHeard {
    /// Its index in the built-in table.
    pub index: usize,
    /// Its channels' roles ([`rank`]).
    pub roles: Option<[u8; SND_TONAL]>,
    /// The speaker `OUT`s that sounded in its frames.
    pub outs: BTreeSet<u16>,
}

impl Default for BuiltinRun {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinRun {
    pub fn new() -> Self {
        let b = SndEngines::new();
        let n = b.entries().len();
        BuiltinRun { b, notes: vec![Vec::new(); n], outs: vec![BTreeSet::new(); n], recent: VecDeque::new() }
    }

    /// The frame `m` just ran.
    pub fn frame(&mut self, m: &Machine) {
        let edges = m.edges();
        let mut pcs: Vec<u16> = edges.iter().map(|e| e.pc).collect();
        pcs.sort_unstable();
        pcs.dedup();
        self.recent.push_back(pcs);
        let d = self.b.frame(m, edges, m.edges_start_level());
        // the frame decided: SND_DELAY_FRAMES back (none yet in the first frames)
        let decided = (self.recent.len() > SND_DELAY_FRAMES as usize).then(|| self.recent.pop_front()).flatten();
        let sounded = Some(sounded_notes(&d)).filter(|v| v.iter().any(|&p| p != 0));
        for (i, notes) in self.notes.iter_mut().enumerate() {
            let mine = d.confident != 0 && usize::from(d.engine) == i + 1;
            notes.push(sounded.filter(|_| mine));
            if let Some(pcs) = decided.as_ref().filter(|_| mine) {
                self.outs[i].extend(pcs);
            }
        }
    }

    /// The built-in entries given any frame, in table order.
    pub fn heard(&self) -> Vec<BuiltinHeard> {
        (0..self.notes.len())
            .filter(|&i| self.notes[i].iter().any(Option::is_some))
            .map(|i| BuiltinHeard { index: i, roles: rank(&self.notes[i]), outs: self.outs[i].clone() })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::rank;
    use zx_sys::{SND_ROLE_BASS as B, SND_ROLE_LEAD as L, SND_ROLE_PAD as P};

    /// Held notes that recur (music): `phrase` (per voice, a period; 0 silent) 8 frames each,
    /// played twice.
    fn tune(phrase: &[[u32; 3]]) -> Vec<Option<[u32; 3]>> {
        let once: Vec<Option<[u32; 3]>> = phrase.iter().flat_map(|&v| std::iter::repeat_n(Some(v), 8)).collect();
        [once.clone(), once].concat()
    }

    #[test]
    fn ranks_by_mean_pitch() {
        // the longest period is the lowest note: channel 2 the bass, channel 0 the lead
        assert_eq!(rank(&tune(&[[1000, 2000, 4000], [1100, 2100, 4400]])), Some([L, P, B]));
        // two channels: the lower the bass; one: the lead; a silent channel none
        assert_eq!(rank(&tune(&[[3000, 1500, 0], [3300, 1600, 0]])), Some([B, L, 0]));
        assert_eq!(rank(&tune(&[[0, 1500, 0], [0, 1600, 0]])), Some([0, L, 0]));
        // the mean over the frames each sounds in: channel 1 crosses channel 0 for a while and
        // stays above it on average
        assert_eq!(rank(&tune(&[[2000, 1000, 0], [2000, 1000, 0], [2000, 2500, 0]])), Some([B, L, 0]));
        // equal means: the lower channel ranks lower
        assert_eq!(rank(&tune(&[[2000, 2000, 2000], [2200, 2200, 2200]])), Some([B, P, L]));
    }

    #[test]
    fn too_little_heard_is_not_ranked() {
        assert_eq!(rank(&[]), None);
        // three frames, no held note
        assert_eq!(rank(&[Some([1000, 2000, 0]), Some([1100, 2100, 0]), Some([1200, 2200, 0])]), None);
    }
}
