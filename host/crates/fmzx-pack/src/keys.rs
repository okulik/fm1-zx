// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Key scripts: `ACTION@ms[+hold]` steps (see `music::corpus`), and the key rows and Kempston bits
//! they hold at a time.

use crate::manifest::parse_action;

/// One step of a key script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub action: u8,
    pub at: u64,
    pub hold: u64,
}

/// Parses a key script: `ACTION@ms` or `ACTION@ms+hold` steps, comma or line separated.
pub fn parse_keys(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        for p in line.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, when) = p.split_once('@').ok_or_else(|| format!("step {p:?}: expected ACTION@ms"))?;
            let (at, hold) = when.split_once('+').map_or((when, "100"), |(a, h)| (a, h));
            let action = parse_action(key.trim())?;
            let num = |s: &str| s.trim().parse::<u64>().map_err(|_| format!("step {p:?}: bad time"));
            steps.push(Step { action, at: num(at)?, hold: num(hold)? });
        }
    }
    Ok(steps)
}

/// The Spectrum key rows and Kempston bits held at `ms`.
pub fn inputs(steps: &[Step], ms: u64) -> ([u8; 8], u8) {
    let (mut rows, mut kemp) = ([0u8; 8], 0u8);
    for s in steps.iter().filter(|s| s.at <= ms && ms < s.at + s.hold) {
        if s.action & 0x40 != 0 {
            kemp |= 1 << (s.action & 0x3f);
        } else if s.action < 40 {
            rows[s.action as usize / 5] |= 1 << (s.action % 5);
        }
    }
    (rows, kemp)
}
