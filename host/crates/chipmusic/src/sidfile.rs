// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! PSID and RSID files (versions 1-4): the header and the C64 data.
//!
//! The header is big-endian: magic `PSID`/`RSID`, version, data offset, load, init and play
//! addresses, songs, start song, the speed bits (bit n set: song n + 1 is timed by CIA 1's
//! timer A, else by the 50/60 Hz vertical blank; songs past 32 use bit 31), and the title,
//! author and released strings (32 bytes each, Latin-1, NUL-padded). Version 2 and later add the
//! flags (bit 0: Compute's Sidplayer MUS data; bit 1: PlaySID samples in a PSID, a BASIC program
//! in an RSID; bits 2-3: the clock, 1 PAL, 2 NTSC, 3 either; bits 4-5: the SID model), the free
//! pages, and (versions 3 and 4) a second and third SID's address. A load address of 0 means
//! the data's first two bytes hold it (little-endian); an init address of 0 means the load
//! address.

/// The C64's PAL clock in Hz.
pub const PAL_HZ: u32 = 985_248;
/// The C64's NTSC clock in Hz.
pub const NTSC_HZ: u32 = 1_022_727;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidFile {
    /// An RSID (a real C64 program: interrupts and timers of its own) rather than a PSID.
    pub rsid: bool,
    pub version: u16,
    pub load: u16,
    pub init: u16,
    /// 0: the tune installs an interrupt handler of its own.
    pub play: u16,
    pub songs: u16,
    /// 1-based.
    pub start_song: u16,
    pub speed: u32,
    pub title: String,
    pub author: String,
    pub released: String,
    pub flags: u16,
    /// The second and third SID's addresses ($Dxx0), 0 for none.
    pub extra_sids: [u16; 2],
    /// The C64 code and data, loaded at `load`.
    pub data: Vec<u8>,
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}

fn latin1(d: &[u8]) -> String {
    d.iter().take_while(|&&c| c != 0).map(|&c| char::from(c)).collect::<String>().trim().to_string()
}

impl SidFile {
    /// Parses a PSID or RSID file. Err: why it is not one.
    pub fn parse(file: &[u8]) -> Result<SidFile, String> {
        if file.len() < 0x76 {
            return Err(format!("not a SID file: {} bytes, shorter than a header", file.len()));
        }
        let rsid = match &file[..4] {
            b"PSID" => false,
            b"RSID" => true,
            _ => return Err("not a SID file: no PSID or RSID magic".into()),
        };
        let version = be16(file, 4);
        if !(1..=4).contains(&version) || (rsid && version < 2) {
            return Err(format!("unknown {} version {version}", if rsid { "RSID" } else { "PSID" }));
        }
        let offset = usize::from(be16(file, 6));
        let need = if version == 1 { 0x76 } else { 0x7C };
        if offset < need || offset > file.len() {
            return Err(format!("bad data offset {offset:#x}"));
        }
        let mut load = be16(file, 8);
        let mut data = file[offset..].to_vec();
        if load == 0 {
            if data.len() < 2 {
                return Err("no data".into());
            }
            load = u16::from_le_bytes([data[0], data[1]]);
            data.drain(..2);
        }
        if data.is_empty() {
            return Err("no data".into());
        }
        if usize::from(load) + data.len() > 0x1_0000 {
            return Err(format!("the data ({} bytes at ${load:04X}) runs past $FFFF", data.len()));
        }
        let init = match be16(file, 10) {
            0 => load,
            a => a,
        };
        let (flags, extra_sids) = if version >= 2 {
            let second = if version >= 3 { u16::from(file[0x7A]) << 4 } else { 0 };
            let third = if version >= 4 { u16::from(file[0x7B]) << 4 } else { 0 };
            let sid = |a: u16| if a == 0 { 0 } else { 0xD000 | a };
            (be16(file, 0x76), [sid(second), sid(third)])
        } else {
            (0, [0, 0])
        };
        let songs = be16(file, 14);
        if songs == 0 {
            return Err("the file says it has no songs".into());
        }
        Ok(SidFile {
            rsid,
            version,
            load,
            init,
            play: be16(file, 12),
            songs,
            start_song: be16(file, 16).clamp(1, songs),
            speed: u32::from_be_bytes([file[18], file[19], file[20], file[21]]),
            title: latin1(&file[0x16..0x36]),
            author: latin1(&file[0x36..0x56]),
            released: latin1(&file[0x56..0x76]),
            flags,
            extra_sids,
            data,
        })
    }

    pub fn format(&self) -> &'static str {
        if self.rsid {
            "RSID"
        } else {
            "PSID"
        }
    }

    /// NTSC only (flags bits 2-3 = 2); PAL otherwise (PAL, either, or unknown).
    pub fn ntsc(&self) -> bool {
        (self.flags >> 2) & 3 == 2
    }

    pub fn clock_hz(&self) -> u32 {
        if self.ntsc() {
            NTSC_HZ
        } else {
            PAL_HZ
        }
    }

    pub fn clock_name(&self) -> &'static str {
        match (self.flags >> 2) & 3 {
            1 => "PAL",
            2 => "NTSC",
            3 => "PAL or NTSC (played as PAL)",
            _ => "unknown (played as PAL)",
        }
    }

    pub fn sid_model(&self) -> &'static str {
        match (self.flags >> 4) & 3 {
            1 => "6581",
            2 => "8580",
            3 => "6581 or 8580",
            _ => "unknown",
        }
    }

    /// Whether `song` (1-based) is timed by CIA 1's timer A (a PSID's speed bit).
    pub fn cia_timed(&self, song: u16) -> bool {
        !self.rsid && self.speed >> (u32::from(song.clamp(1, 32)) - 1) & 1 != 0
    }

    /// Why this file cannot be played, if it cannot.
    pub fn refusal(&self) -> Option<String> {
        if self.flags & 1 != 0 {
            return Some("Compute's Sidplayer (MUS) data needs a Sidplayer, which is not provided".into());
        }
        if self.rsid && self.flags & 2 != 0 {
            return Some("a C64 BASIC program: the BASIC interpreter is not provided".into());
        }
        if self.extra_sids[0] != 0 {
            let n = if self.extra_sids[1] != 0 { 3 } else { 2 };
            return Some(format!("a tune for {n} SIDs (the second at ${:04X}): only one SID is captured", self.extra_sids[0]));
        }
        None
    }

    /// The header as text, for `fmzx-pack chip-notes --info`.
    pub fn info(&self) -> String {
        let end = usize::from(self.load) + self.data.len() - 1;
        let mut s = format!(
            "{} v{}, {} songs, start song {}\ntitle: {}\nauthor: {}\nreleased: {}\nclock: {} ({} Hz)\nSID model: {}\n\
             load ${:04X}-${end:04X}, init ${:04X}, play {}\n",
            self.format(),
            self.version,
            self.songs,
            self.start_song,
            self.title,
            self.author,
            self.released,
            self.clock_name(),
            self.clock_hz(),
            self.sid_model(),
            self.load,
            self.init,
            if self.play == 0 { "0 (the tune's own interrupt)".to_string() } else { format!("${:04X}", self.play) },
        );
        if !self.rsid {
            let cia: Vec<String> = (1..=self.songs).filter(|&n| self.cia_timed(n)).map(|n| n.to_string()).collect();
            s += &if cia.is_empty() {
                "speed: vertical blank for every song\n".to_string()
            } else {
                format!("speed: CIA timer for songs {}, vertical blank for the rest\n", cia.join(", "))
            };
        }
        if let Some(r) = self.refusal() {
            s += &format!("cannot be played: {r}\n");
        }
        s
    }
}
