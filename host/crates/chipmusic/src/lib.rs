// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Chip tunes to notes for the FM-1, written as TOML for `tools/arrange-music`
//! (`fmzx-pack chip-notes`). No audio is made. Two kinds of tune, told apart by their magic:
//!
//! - A C64 SID tune (PSID or RSID, [`sidfile`]) played in a 6502 and C64 model ([`cpu`],
//!   [`c64`]), its SID registers captured once per play call.
//! - A ZX Spectrum 128 AY song (a ZXAYEMUL `.ay` file, [`ayfile`]) played in the Spectrum
//!   machine core ([`ay`]), its AY registers captured once per frame.
//!
//! Either capture is turned into notes on the Spectrum's clock ([`notes`]).
//!
//! # The notes file
//!
//! Top-level keys: `file` (the tune's file as named), `format` (`PSID`, `RSID` or `AY`),
//! `subtune` (1-based), `title`, `author` and `released` (a SID's header's; an `.ay`'s song name
//! (else its misc string), its author, and its misc string when that is not the title),
//! `clock_hz` (the chip's clock: the C64's 985,248 PAL or 1,022,727 NTSC, the 128K's AY
//! 1,773,400), `seconds` (the time played), `intro_t` and `loop_t` (T-states at 3.5 MHz;
//! `loop_t` 0: no loop, the notes end at `intro_t`). Then each `[[note]]`, by start and voice:
//! `voice` (0-2 tonal: 0 the lead, 1 the bass, 2 the pad; 3 the drum), `start_t` and `end_t` (T
//! since the start; a loop's notes are the intro's and then one pass's, from `intro_t`),
//! `period_t` (the pitch's period in T, 0 for the drum) and `level` (1-255).

pub mod ay;
pub mod ayfile;
pub mod c64;
pub mod cpu;
pub mod notes;
pub mod sidfile;

pub use ayfile::AyFile;
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

/// The file a tune comes from.
#[derive(Clone, Debug)]
pub enum Source {
    Sid(SidFile),
    Ay(AyFile),
}

impl Source {
    /// Parses a SID or `.ay` file, by its magic. Err: why it is neither.
    pub fn parse(data: &[u8]) -> Result<Source, String> {
        if AyFile::is_ay(data) {
            return AyFile::parse(data).map(Source::Ay);
        }
        SidFile::parse(data).map(Source::Sid).map_err(|e| match e.strip_prefix("not a SID file: no ") {
            Some(rest) => format!("not a SID or AY file: no {rest}, nor ZXAY"),
            None => e,
        })
    }

    /// `PSID`, `RSID` or `AY`.
    pub fn format(&self) -> &'static str {
        match self {
            Source::Sid(s) => s.format(),
            Source::Ay(_) => "AY",
        }
    }

    /// The number of subtunes.
    pub fn songs(&self) -> u16 {
        match self {
            Source::Sid(s) => s.songs,
            Source::Ay(a) => a.count(),
        }
    }

    /// The subtune played when none is asked for (1-based).
    pub fn start_song(&self) -> u16 {
        match self {
            Source::Sid(s) => s.start_song,
            Source::Ay(a) => a.start_song(),
        }
    }

    /// Subtune `song`'s (1-based) title.
    pub fn title(&self, song: u16) -> String {
        match self {
            Source::Sid(s) => s.title.clone(),
            Source::Ay(a) => a.title(song),
        }
    }

    pub fn author(&self) -> String {
        match self {
            Source::Sid(s) => s.author.clone(),
            Source::Ay(a) => a.author.clone(),
        }
    }

    /// A SID's released string; an `.ay`'s misc string unless it is subtune `song`'s title.
    pub fn released(&self, song: u16) -> String {
        match self {
            Source::Sid(s) => s.released.clone(),
            Source::Ay(a) if a.title(song) == a.misc => String::new(),
            Source::Ay(a) => a.misc.clone(),
        }
    }

    /// The chip's clock in Hz.
    pub fn clock_hz(&self) -> u32 {
        match self {
            Source::Sid(s) => s.clock_hz(),
            Source::Ay(_) => ay::AY_HZ,
        }
    }

    /// The header as text, for `fmzx-pack chip-notes --info`.
    pub fn info(&self) -> String {
        match self {
            Source::Sid(s) => s.info(),
            Source::Ay(a) => a.info(),
        }
    }
}

/// The header of the SID or `.ay` file `data` as text, for `fmzx-pack chip-notes --info`.
/// Err: why it is neither.
pub fn info(data: &[u8]) -> Result<String, String> {
    Source::parse(data).map(|s| s.info())
}

/// A tune's notes and what they came from.
#[derive(Clone, Debug)]
pub struct ChipNotes {
    pub source: Source,
    pub subtune: u16,
    pub seconds: f64,
    pub clock_hz: u32,
    pub notes: Notes,
}

/// Plays `subtune` (1-based; the file's start song if `None`) of the SID or `.ay` file `data`
/// for `seconds` and makes its notes. Err: why it cannot be played.
pub fn chip_notes(data: &[u8], subtune: Option<u16>, seconds: f64) -> Result<ChipNotes, String> {
    let source = Source::parse(data)?;
    let subtune = subtune.unwrap_or(source.start_song());
    let notes = match &source {
        Source::Sid(sid) => notes::convert(&c64::capture(sid, subtune, seconds)?),
        Source::Ay(file) => notes::convert_ay(&ay::capture(file, subtune, seconds)?),
    };
    Ok(ChipNotes { clock_hz: source.clock_hz(), source, subtune, seconds, notes })
}

impl ChipNotes {
    /// The notes file's text (see the crate docs); `file` the tune file's name as given.
    pub fn toml(&self, file: &str) -> String {
        let n = &self.notes;
        let (chip, voice, names): (&str, &str, [&str; 3]) = match self.source {
            Source::Sid(_) => ("A SID", "SID voice", ["1", "2", "3"]),
            Source::Ay(_) => ("An AY", "AY channel", ["A", "B", "C"]),
        };
        let from: Vec<String> = ["lead", "bass", "pad"]
            .iter()
            .zip(n.chip_voice)
            .filter_map(|(role, v)| v.map(|v| format!("{role} {voice} {}", names[usize::from(v)])))
            .collect();
        let src = &self.source;
        let mut s = format!(
            "# {chip} tune's notes for the FM-1, made by fmzx-pack chip-notes.\n\
             # Voices: {}; 3 the drum (noise).\n\
             file = {}\nformat = {}\nsubtune = {}\ntitle = {}\nauthor = {}\nreleased = {}\nclock_hz = {}\nseconds = {}\n\
             intro_t = {}\nloop_t = {}\n",
            if from.is_empty() { "none".to_string() } else { from.join(", ") },
            quoted(file),
            quoted(src.format()),
            self.subtune,
            quoted(&src.title(self.subtune)),
            quoted(&src.author()),
            quoted(&src.released(self.subtune)),
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
            self.source.songs(),
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
