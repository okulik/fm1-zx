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
            0x20 => 2,
            0x21 | 0x30 => 1 + at(i, 1)?[0] as usize,
            0x22 => 0,
            0x31 => 2 + at(i + 1, 1)?[0] as usize,
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
