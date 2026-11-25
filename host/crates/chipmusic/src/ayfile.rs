// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! ZXAYEMUL `.ay` files (the Project AY format): the header, the songs and their Z80 code and
//! data.
//!
//! Words are big-endian. A pointer is a 16-bit offset from its own position in the file (signed;
//! one that would land before the file's start is read unsigned, as files over 32 KB need); a
//! string pointer of 0 is no string. The header: magic `ZXAY`, type `EMUL` (the only type
//! played), the file and player versions, pointers to a special player (0: none; any other is
//! refused when played), the author and misc strings, the songs - 1, the first song (0-based)
//! and a pointer to the songs: 4 bytes each, pointers to the song's name and its data. A song's
//! data: the Amiga channel map (4 bytes, unused here), the length and fade (1/50 s), `HiReg` and
//! `LoReg` (the Z80 registers' high and low bytes at the start), and pointers to the points
//! (stack, init and interrupt addresses; init 0: the first block's address, interrupt 0: the
//! init plays by itself) and to the blocks: address, length and a pointer to the bytes, 6 bytes
//! each, until an address of 0. A block is clipped at $FFFF and at the file's end.

/// A song's memory block: its bytes, loaded at `addr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AyBlock {
    pub addr: u16,
    pub data: Vec<u8>,
}

/// A song of an `.ay` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AySong {
    pub name: String,
    /// The length in 1/50 s (0: unknown).
    pub length: u16,
    pub fade: u16,
    pub hi_reg: u8,
    pub lo_reg: u8,
    pub stack: u16,
    /// As in the file: 0 means the first block's address.
    pub init: u16,
    /// 0: none (the init plays by itself).
    pub interrupt: u16,
    pub blocks: Vec<AyBlock>,
}

/// A ZXAYEMUL file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AyFile {
    pub file_version: u8,
    pub player_version: u8,
    /// The special player's pointer as written (0: none).
    pub special_player: u16,
    pub author: String,
    pub misc: String,
    /// 0-based.
    pub first_song: u8,
    pub songs: Vec<AySong>,
}

/// The file being read: bounds-checked reads that fail with a reason.
struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn u8(&self, at: usize, what: &str) -> Result<u8, String> {
        self.0.get(at).copied().ok_or_else(|| format!("{what} at {at:#x} is past the file's end"))
    }
    fn u16(&self, at: usize, what: &str) -> Result<u16, String> {
        Ok(u16::from(self.u8(at, what)?) << 8 | u16::from(self.u8(at + 1, what)?))
    }
    /// The position the pointer at `at` points to (see the module docs); `None` for 0.
    fn ptr(&self, at: usize, what: &str) -> Result<Option<usize>, String> {
        let raw = self.u16(at, what)?;
        if raw == 0 {
            return Ok(None);
        }
        let signed = at as i64 + i64::from(raw as i16);
        let pos = if signed >= 0 { signed } else { at as i64 + i64::from(raw) };
        if pos as usize >= self.0.len() {
            return Err(format!("{what} at {at:#x} points to {pos:#x}, past the file's end ({:#x} bytes)", self.0.len()));
        }
        Ok(Some(pos as usize))
    }
    /// A pointer that must be there.
    fn need(&self, at: usize, what: &str) -> Result<usize, String> {
        self.ptr(at, what)?.ok_or_else(|| format!("{what} at {at:#x} is missing (0)"))
    }
    /// The NUL-terminated string the pointer at `at` points to (Latin-1; at most 255 bytes).
    fn string(&self, at: usize, what: &str) -> Result<String, String> {
        let Some(p) = self.ptr(at, what)? else { return Ok(String::new()) };
        let s: String = self.0[p..].iter().take(255).take_while(|&&c| c != 0).map(|&c| char::from(c)).collect();
        Ok(s.trim().to_string())
    }
}

impl AyFile {
    /// Whether `data` starts as a ZXAY file does (of any type).
    pub fn is_ay(data: &[u8]) -> bool {
        data.starts_with(b"ZXAY")
    }

    /// Parses a ZXAYEMUL file. Err: why it is not one.
    pub fn parse(file: &[u8]) -> Result<AyFile, String> {
        if !Self::is_ay(file) {
            return Err("not an AY file: no ZXAY magic".into());
        }
        if file.len() < 20 {
            return Err(format!("not an AY file: {} bytes, shorter than a header", file.len()));
        }
        if &file[4..8] != b"EMUL" {
            let t: String = file[4..8].iter().map(|&c| if c.is_ascii_graphic() { char::from(c) } else { '?' }).collect();
            return Err(format!("an AY file of type {t}: only EMUL files (Z80 code) are played"));
        }
        let r = Rd(file);
        let count = usize::from(file[16]) + 1;
        let first_song = file[17];
        let table = r.need(18, "the songs' pointer")?;
        let mut songs = Vec::with_capacity(count);
        for k in 0..count {
            let at = table + 4 * k;
            let song = Self::song(&r, at).map_err(|e| format!("song {}: {e}", k + 1))?;
            songs.push(song);
        }
        if usize::from(first_song) >= count {
            return Err(format!("the first song is {} of {count}", usize::from(first_song) + 1));
        }
        Ok(AyFile {
            file_version: file[8],
            player_version: file[9],
            special_player: r.u16(10, "the special player's pointer")?,
            author: r.string(12, "the author's pointer")?,
            misc: r.string(14, "the misc pointer")?,
            first_song,
            songs,
        })
    }

    fn song(r: &Rd, at: usize) -> Result<AySong, String> {
        let name = r.string(at, "the name's pointer")?;
        let d = r.need(at + 2, "the data's pointer")?;
        let points = r.need(d + 10, "the points' pointer")?;
        let mut blocks = Vec::new();
        if let Some(mut b) = r.ptr(d + 12, "the blocks' pointer")? {
            loop {
                let addr = r.u16(b, "a block's address")?;
                if addr == 0 {
                    break;
                }
                let len = usize::from(r.u16(b + 2, "a block's length")?);
                let from = r.need(b + 4, "a block's pointer")?;
                let len = len.min(0x1_0000 - usize::from(addr)).min(r.0.len() - from);
                if len > 0 {
                    blocks.push(AyBlock { addr, data: r.0[from..from + len].to_vec() });
                }
                b += 6;
            }
        }
        Ok(AySong {
            name,
            length: r.u16(d + 4, "the song's length")?,
            fade: r.u16(d + 6, "the song's fade")?,
            hi_reg: r.u8(d + 8, "HiReg")?,
            lo_reg: r.u8(d + 9, "LoReg")?,
            stack: r.u16(points, "the stack address")?,
            init: r.u16(points + 2, "the init address")?,
            interrupt: r.u16(points + 4, "the interrupt address")?,
            blocks,
        })
    }

    /// The number of songs.
    pub fn count(&self) -> u16 {
        self.songs.len() as u16
    }

    /// The first song, 1-based.
    pub fn start_song(&self) -> u16 {
        u16::from(self.first_song) + 1
    }

    /// Song `song`'s (1-based) title: its name, else the file's misc string.
    pub fn title(&self, song: u16) -> String {
        match self.songs.get(usize::from(song.max(1)) - 1) {
            Some(s) if !s.name.is_empty() => s.name.clone(),
            _ => self.misc.clone(),
        }
    }

    /// Why song `song` (1-based) cannot be played, if it cannot.
    pub fn refusal(&self, song: u16) -> Option<String> {
        if self.special_player != 0 {
            return Some("the file needs a special player, which is not provided".into());
        }
        let n = self.count();
        if song == 0 || song > n {
            return Some(format!("no subtune {song}: the file has {n} (1-{n})"));
        }
        let s = &self.songs[usize::from(song) - 1];
        if s.blocks.is_empty() {
            return Some("the song has no code or data".into());
        }
        None
    }

    /// The header as text, for `fmzx-pack chip-notes --info`.
    pub fn info(&self) -> String {
        let mut s = format!(
            "AY (ZXAYEMUL) file version {}, player version {}, {} songs, start song {}\nauthor: {}\nmisc: {}\n",
            self.file_version,
            self.player_version,
            self.count(),
            self.start_song(),
            self.author,
            self.misc,
        );
        for (k, song) in self.songs.iter().enumerate() {
            let blocks: Vec<String> = song
                .blocks
                .iter()
                .map(|b| format!("${:04X}-${:04X}", b.addr, usize::from(b.addr) + b.data.len() - 1))
                .collect();
            let length = if song.length == 0 { "unknown".to_string() } else { format!("{:.1} s", f64::from(song.length) / 50.0) };
            s += &format!(
                "song {}: {}\n  length {length}, stack ${:04X}, init ${:04X}, interrupt {}, blocks {}\n",
                k + 1,
                song.name,
                song.stack,
                song.init,
                if song.interrupt == 0 { "0 (the init plays by itself)".to_string() } else { format!("${:04X}", song.interrupt) },
                if blocks.is_empty() { "none".to_string() } else { blocks.join(", ") },
            );
        }
        if self.special_player != 0 {
            s += "cannot be played: the file needs a special player\n";
        }
        s
    }
}
