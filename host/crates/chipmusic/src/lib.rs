// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Chip tunes to notes for the FM-1: a SID tune (PSID or RSID) played in a 6502 and C64 model
//! ([`cpu`], [`c64`]), its SID registers captured once per play call, and turned into notes on
//! the Spectrum's clock ([`notes`]), written as TOML for `tools/arrange-music`
//! (`fmzx-pack chip-notes`). No audio is made.
//!
//! # The notes file
//!
//! Top-level keys: `file` (the SID file as named), `subtune` (1-based), `title`, `author` and
//! `released` (the header's), `clock_hz` (the C64's clock: 985,248 PAL or 1,022,727 NTSC),
//! `seconds` (the time played), `intro_t` and `loop_t` (T-states at 3.5 MHz; `loop_t` 0: no
//! loop, the notes end at `intro_t`). Then each `[[note]]`, by start and voice: `voice` (0-2
//! tonal: 0 the lead, 1 the bass, 2 the pad; 3 the drum), `start_t` and `end_t` (T since the
//! start; a loop's notes are the intro's and then one pass's, from `intro_t`), `period_t` (the
//! pitch's period in T, 0 for the drum) and `level` (1-255).

pub mod c64;
pub mod cpu;
pub mod notes;
pub mod sidfile;

pub use notes::{Note, Notes};
pub use sidfile::SidFile;

/// The time played when none is given (seconds).
pub const DEFAULT_SECONDS: f64 = 180.0;

/// A TOML basic string.
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if (c as u32) < 0x20 || c == '\u{7F}' => out += &format!("\\u{:04X}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// A tune's notes and what they came from.
#[derive(Clone, Debug)]
pub struct ChipNotes {
    pub sid: SidFile,
    pub subtune: u16,
    pub seconds: f64,
    pub clock_hz: u32,
    pub notes: Notes,
}

/// Plays `subtune` (1-based; the file's start song if `None`) of the SID file `data` for
/// `seconds` and makes its notes. Err: why it cannot be played.
pub fn chip_notes(data: &[u8], subtune: Option<u16>, seconds: f64) -> Result<ChipNotes, String> {
    let sid = SidFile::parse(data)?;
    let subtune = subtune.unwrap_or(sid.start_song);
    let cap = c64::capture(&sid, subtune, seconds)?;
    let notes = notes::convert(&cap);
    Ok(ChipNotes { clock_hz: cap.clock_hz, sid, subtune, seconds, notes })
}

impl ChipNotes {
    /// The notes file's text (see the crate docs); `file` the SID file's name as given.
    pub fn toml(&self, file: &str) -> String {
        let n = &self.notes;
        let from: Vec<String> = ["lead", "bass", "pad"]
            .iter()
            .zip(n.sid_voice)
            .filter_map(|(role, v)| v.map(|v| format!("{role} SID voice {}", v + 1)))
            .collect();
        let mut s = format!(
            "# A SID tune's notes for the FM-1, made by fmzx-pack chip-notes.\n\
             # Voices: {}; 3 the drum (noise).\n\
             file = {}\nsubtune = {}\ntitle = {}\nauthor = {}\nreleased = {}\nclock_hz = {}\nseconds = {}\n\
             intro_t = {}\nloop_t = {}\n",
            if from.is_empty() { "none".to_string() } else { from.join(", ") },
            quoted(file),
            self.subtune,
            quoted(&self.sid.title),
            quoted(&self.sid.author),
            quoted(&self.sid.released),
            self.clock_hz,
            self.seconds,
            n.intro_t,
            n.loop_t,
        );
        for x in &n.notes {
            s += &format!(
                "\n[[note]]\nvoice = {}\nstart_t = {}\nend_t = {}\nperiod_t = {}\nlevel = {}\n",
                x.voice, x.start_t, x.end_t, x.period_t, x.level
            );
        }
        s
    }

    /// One line on what was made.
    pub fn summary(&self) -> String {
        let n = &self.notes;
        let secs = (n.intro_t + n.loop_t) as f64 / notes::SPECTRUM_HZ as f64;
        let per: Vec<usize> = (0..4).map(|v| n.notes.iter().filter(|x| x.voice == v).count()).collect();
        let tonal: usize = per[..3].iter().sum();
        let mut s = format!(
            "subtune {} of {}, {} notes over {secs:.1} s ({:.1}/s; lead {}, bass {}, pad {}), {} drum hits",
            self.subtune,
            self.sid.songs,
            tonal,
            if secs > 0.0 { tonal as f64 / secs } else { 0.0 },
            per[0],
            per[1],
            per[2],
            per[3]
        );
        if n.arp_runs > 0 {
            s += &format!(", {} arpeggio runs folded ({} notes before)", n.arp_runs, n.raw_notes);
        }
        s += &if n.loop_t > 0 {
            format!(
                ", loop of {:.2} s from {:.2} s",
                n.loop_t as f64 / notes::SPECTRUM_HZ as f64,
                n.intro_t as f64 / notes::SPECTRUM_HZ as f64
            )
        } else {
            ", no loop".to_string()
        };
        if n.dropped != (0, 0) {
            s += &format!(
                ", {} drum and {} tonal notes dropped (over {} events in a frame)",
                n.dropped.0,
                n.dropped.1,
                notes::EVENTS_PER_FRAME
            );
        }
        s
    }
}
