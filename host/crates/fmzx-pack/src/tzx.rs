// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `.tzx` files in memory: [`to_tap`] turns a standard-speed one into a `.tap`, and
//! [`hardware`] reads the hardware type list. The input is untrusted: every length is checked.

/// The length of the body of TZX block `id` whose body starts at `p` (after the id byte), or
/// `None` for an id this reader does not know. Only the length fields are read here; the
/// caller checks that the whole body fits.
fn body_len(d: &[u8], p: usize, id: u8) -> Result<Option<usize>, String> {
    let at = |o: usize, n: usize| -> Result<&[u8], String> {
        d.get(p + o..(p + o).checked_add(n).ok_or("block past the end")?).ok_or_else(|| "block past the end".to_string())
    };
    let w16 = |o: usize| -> Result<usize, String> { Ok(u16::from_le_bytes(at(o, 2)?.try_into().unwrap()) as usize) };
    let w24 = |o: usize| -> Result<usize, String> {
        let b = at(o, 3)?;
        Ok(b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16)
    };
    let w32 = |o: usize| -> Result<usize, String> { Ok(u32::from_le_bytes(at(o, 4)?.try_into().unwrap()) as usize) };
    Ok(Some(match id {
        // 0x10 standard speed: pause, length word, data
        0x10 => 4 + w16(2)?,
        // 0x11 turbo speed: 0x12 bytes of timings, the data length at 0x0F
        0x11 => 0x12 + w24(0x0F)?,
        // 0x12 pure tone: pulse length and count
        0x12 => 4,
        // 0x13 pulse sequence: a count, then that many words
        0x13 => 1 + 2 * at(0, 1)?[0] as usize,
        // 0x14 pure data: 10 bytes of timings, the data length at 7
        0x14 => 0x0A + w24(7)?,
        // 0x15 direct recording: 8 bytes, the data length at 5
        0x15 => 8 + w24(5)?,
        // 0x18 CSW recording, 0x19 generalized data: a 4-byte length
        0x18 | 0x19 => 4 + w32(0)?,
        // 0x20 pause, 0x23 jump, 0x24 loop start: a word
        0x20 | 0x23 | 0x24 => 2,
        // 0x21 group start, 0x30 text description: a length byte and the text
        0x21 | 0x30 => 1 + at(0, 1)?[0] as usize,
        // 0x22 group end, 0x25 loop end, 0x27 return from sequence
        0x22 | 0x25 | 0x27 => 0,
        // 0x26 call sequence: a count word, then that many words
        0x26 => 2 + 2 * w16(0)?,
        // 0x28 select block: a length word and its content
        0x28 => 2 + w16(0)?,
        // 0x2A stop the tape if in 48K mode: a 4-byte length (0)
        0x2a => 4,
        // 0x2B set signal level: length 4 and the level
        0x2b => 5,
        0x31 => 2 + at(1, 1)?[0] as usize,
        // 0x33 hardware type: a count, then 3 bytes each
        0x33 => 1 + 3 * at(0, 1)?[0] as usize,
        // 0x34 emulation info: 8 bytes
        0x34 => 8,
        0x32 => 2 + w16(0)?,
        0x35 => 0x14 + w32(0x10)?,
        0x5a => 9,
        _ => return Ok(None),
    }))
}

/// The blocks of a TZX file: each id with its body, until the end or the first block that is
/// unknown or does not fit (then the error).
fn blocks(d: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    if d.len() < 10 || &d[..8] != b"ZXTape!\x1a" {
        return Err("not a TZX file".into());
    }
    let mut i = 10usize;
    let mut out = Vec::new();
    while i < d.len() {
        let id = d[i];
        i += 1;
        let len = body_len(d, i, id)?.ok_or_else(|| format!("block 0x{id:02X} carries non-standard data"))?;
        let body = i.checked_add(len).and_then(|e| d.get(i..e)).ok_or("block past the end")?;
        out.push((id, body));
        i += len;
    }
    Ok(out)
}

/// The `.tap` image of a TZX file, or why it cannot be loaded. Only block 0x10 carries data;
/// the blocks that carry none (0x20 pause, 0x21/0x22 groups, 0x30 text, 0x31 message, 0x32
/// archive info, 0x35 custom info, 0x5A glue, ...) are skipped by their lengths; any other
/// block (turbo speed, pure tone, direct recording, ...) makes the tape not loadable this way.
pub fn to_tap(d: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for (id, body) in blocks(d)? {
        match id {
            0x10 => out.extend_from_slice(&body[2..]),
            0x11..=0x19 => return Err(format!("block 0x{id:02X} carries non-standard data")),
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("no standard-speed data blocks".into());
    }
    Ok(out)
}

/// The entries of the TZX's hardware type blocks (0x33), as (type, id, info) triples, from the
/// blocks before the first one that is unknown or cut short. Type 0 is a computer: id 0 the
/// 16K, 1 the 48K, 2 the 48K issue 1, 3 the 128K, 4 the +2, 5 the +2A/+3, 8 the Pentagon and
/// 0x0E the Spanish 128K. Info 0 runs on it, 1 uses its special hardware, 2 runs without using
/// it, 3 does not run on it.
pub fn hardware(d: &[u8]) -> Vec<(u8, u8, u8)> {
    if d.len() < 10 || &d[..8] != b"ZXTape!\x1a" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 10usize;
    while i < d.len() {
        let id = d[i];
        i += 1;
        let Some(len) = body_len(d, i, id).ok().flatten() else { break };
        let Some(body) = i.checked_add(len).and_then(|e| d.get(i..e)) else { break };
        if id == 0x33 {
            out.extend(body[1..].as_chunks::<3>().0.iter().map(|e| (e[0], e[1], e[2])));
        }
        i += len;
    }
    out
}
