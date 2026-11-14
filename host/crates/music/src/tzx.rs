// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Standard-speed `.tzx` to `.tap`, in memory. Only block 0x10 carries data; the blocks that
//! carry none (0x20 pause, 0x21/0x22 groups, 0x30 text, 0x31 message, 0x32 archive info, 0x35
//! custom info, 0x5A glue) are skipped by their lengths; any other block (turbo speed, pure
//! tone, direct recording, ...) makes the tape not loadable. The input is untrusted: every
//! length is checked.

/// The `.tap` image of a TZX file, or why it cannot be loaded.
pub fn to_tap(d: &[u8]) -> Result<Vec<u8>, String> {
    if d.len() < 10 || &d[..8] != b"ZXTape!\x1a" {
        return Err("not a TZX file".into());
    }
    let mut i = 10usize;
    let mut out = Vec::new();
    let at = |p: usize, n: usize| -> Result<&[u8], String> {
        d.get(p..p.checked_add(n).ok_or("block past the end")?).ok_or_else(|| "block past the end".to_string())
    };
    while i < d.len() {
        let id = d[i];
        i += 1;
        let len = match id {
            0x10 => {
                let h = at(i, 4)?;
                let ln = u16::from_le_bytes([h[2], h[3]]) as usize;
                out.extend_from_slice(&(ln as u16).to_le_bytes());
                out.extend_from_slice(at(i + 4, ln)?);
                4 + ln
            }
            // 0x20 pause, 0x23 jump, 0x24 loop start: a word
            0x20 | 0x23 | 0x24 => 2,
            // 0x21 group start, 0x30 text description: a length byte and the text
            0x21 | 0x30 => 1 + at(i, 1)?[0] as usize,
            // 0x22 group end, 0x25 loop end, 0x27 return from sequence
            0x22 | 0x25 | 0x27 => 0,
            // 0x26 call sequence: a count word, then that many words
            0x26 => 2 + 2 * u16::from_le_bytes(at(i, 2)?.try_into().unwrap()) as usize,
            // 0x28 select block: a length word and its content
            0x28 => 2 + u16::from_le_bytes(at(i, 2)?.try_into().unwrap()) as usize,
            // 0x2A stop the tape if in 48K mode: a 4-byte length (0)
            0x2a => 4,
            // 0x2B set signal level: length 4 and the level
            0x2b => 5,
            0x31 => 2 + at(i + 1, 1)?[0] as usize,
            // 0x33 hardware type: a count, then 3 bytes each
            0x33 => 1 + 3 * at(i, 1)?[0] as usize,
            // 0x34 emulation info: 8 bytes
            0x34 => 8,
            0x32 => 2 + u16::from_le_bytes(at(i, 2)?.try_into().unwrap()) as usize,
            0x35 => 0x14 + u32::from_le_bytes(at(i + 0x10, 4)?.try_into().unwrap()) as usize,
            0x5a => 9,
            _ => return Err(format!("block 0x{id:02X} carries non-standard data")),
        };
        at(i, len)?;
        i += len;
    }
    if out.is_empty() {
        return Err("no standard-speed data blocks".into());
    }
    Ok(out)
}
