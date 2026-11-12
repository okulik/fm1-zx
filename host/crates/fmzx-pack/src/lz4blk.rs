// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Copyright (C) 2026 Leo Kuroshita (@kurogedelic), Hügelton Instruments
//
// A Rust port of Felucca's tools/lz4blk.py and of the loader wrapping in its tools/build.py
// (ldr_head, ldr_wrap) (https://github.com/hugelton/Felucca, commit
// a53ecf428c29c780108fd70a31b71a32a6b5c313, Felucca 1.6.1), GPL-3.0-only like the originals. The
// FM-1 ZX firmware image never links this file: it is part of the host tool that builds the
// update package.

//! The update loader as the package carries it (`ota.bin`), and the minimal LZ4 *block* format
//! (no frame) it is compressed with: a greedy compressor without hash chains, and a decoder.
//! Block format: <https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md> (minimum match 4,
//! the last 5 bytes literals, the last match starting at least 12 bytes before the end).
//!
//! `ota.bin` = an outer JLFS head (32 bytes), an inner head (32 bytes, the load address), then
//! per 4,096 bytes of the loader `[compressed length u32][length u32][LZ4 block]`.

use crate::fwsc::crc16;
use std::collections::HashMap;

/// Where the boot loader decompresses the update loader to (RAM), and calls it.
pub const LOADER_LOAD: u32 = 0x01C0_A800;
/// The file name the boot loader looks for.
pub const LOADER_NAME: &[u8; 15] = b"usb_hid_ota.bin";
/// Bytes of the loader per LZ4 block.
pub const BLOCK: usize = 4096;

fn len_bytes(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

/// Compresses `src` into one LZ4 block.
pub fn compress(src: &[u8]) -> Vec<u8> {
    let n = src.len();
    let mut out = Vec::new();
    let mut table: HashMap<[u8; 4], usize> = HashMap::new();
    let (mut anchor, mut i) = (0usize, 0usize);
    let limit = n.saturating_sub(12); // matches must start before this
    while i < limit {
        let key: [u8; 4] = src[i..i + 4].try_into().expect("4 bytes");
        let cand = match table.insert(key, i) {
            Some(c) if i - c <= 65535 => c,
            _ => {
                i += 1;
                continue;
            }
        };
        let mut mlen = 4; // extend; leave the last 5 bytes as literals
        while i + mlen < n - 5 && src[cand + mlen] == src[i + mlen] {
            mlen += 1;
        }
        let lit = &src[anchor..i];
        out.push((lit.len().min(15) << 4) as u8 | (mlen - 4).min(15) as u8);
        if lit.len() >= 15 {
            len_bytes(&mut out, lit.len() - 15);
        }
        out.extend_from_slice(lit);
        let off = i - cand;
        out.extend_from_slice(&[off as u8, (off >> 8) as u8]);
        if mlen - 4 >= 15 {
            len_bytes(&mut out, mlen - 4 - 15);
        }
        i += mlen;
        anchor = i;
    }
    let lit = &src[anchor..]; // the final literal run
    out.push((lit.len().min(15) << 4) as u8);
    if lit.len() >= 15 {
        len_bytes(&mut out, lit.len() - 15);
    }
    out.extend_from_slice(lit);
    out
}

/// Decodes one LZ4 block.
pub fn decompress(blk: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "malformed LZ4 block".to_string();
    let mut out = Vec::new();
    let mut i = 0;
    let byte = |i: &mut usize| -> Result<usize, String> {
        let b = *blk.get(*i).ok_or_else(bad)?;
        *i += 1;
        Ok(usize::from(b))
    };
    let more = |i: &mut usize, mut n: usize| -> Result<usize, String> {
        loop {
            let b = byte(i)?;
            n += b;
            if b != 255 {
                return Ok(n);
            }
        }
    };
    while i < blk.len() {
        let tok = byte(&mut i)?;
        let mut ll = tok >> 4;
        if ll == 15 {
            ll = more(&mut i, ll)?;
        }
        out.extend_from_slice(blk.get(i..i + ll).ok_or_else(bad)?);
        i += ll;
        if i >= blk.len() {
            break;
        }
        let off = byte(&mut i)? | byte(&mut i)? << 8;
        let mut ml = tok & 15;
        if ml == 15 {
            ml = more(&mut i, ml)?;
        }
        if off == 0 || off > out.len() {
            return Err(bad());
        }
        for _ in 0..ml + 4 {
            out.push(out[out.len() - off]);
        }
    }
    Ok(out)
}

/// A 32-byte JLFS head as the loader wrapping writes it.
fn ldr_head(dcrc: u16, a: u32, b: u32, attr: u8, name: &[u8]) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[2..4].copy_from_slice(&dcrc.to_le_bytes());
    h[4..8].copy_from_slice(&a.to_le_bytes());
    h[8..12].copy_from_slice(&b.to_le_bytes());
    h[12] = attr;
    h[16..16 + name.len()].copy_from_slice(name);
    let c = crc16(&h[2..]);
    h[..2].copy_from_slice(&c.to_le_bytes());
    h
}

/// `ota.bin` from the loader image (`loader.bin`).
pub fn wrap_loader(image: &[u8]) -> Result<Vec<u8>, String> {
    let mut blocks = Vec::new();
    for (i, raw) in image.chunks(BLOCK).enumerate() {
        let c = compress(raw);
        if c.len() > BLOCK || decompress(&c).as_deref() != Ok(raw) {
            return Err(format!("loader: block {i} does not compress below {BLOCK} bytes"));
        }
        blocks.extend_from_slice(&(c.len() as u32).to_le_bytes());
        blocks.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        blocks.extend_from_slice(&c);
    }
    let mut body = ldr_head(crc16(image), image.len() as u32, LOADER_LOAD, 0, LOADER_NAME).to_vec();
    body.extend_from_slice(&blocks);
    let mut ota = ldr_head(crc16(&body), 0x20, body.len() as u32, 0x41, LOADER_NAME).to_vec();
    ota.extend_from_slice(&body);
    Ok(ota)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut v: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 13) as u8).collect();
        v.extend((0..300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8));
        for src in [&v[..], &v[..1], &v[..12], &v[..13], &[][..], &[0u8; 4096][..]] {
            let c = compress(src);
            assert_eq!(decompress(&c).unwrap(), src);
        }
    }

    #[test]
    fn decoder_agrees_with_lz4_flex() {
        let src: Vec<u8> = (0..4096u32).map(|i| (i / 3 % 251) as u8).collect();
        let c = compress(&src);
        assert_eq!(lz4_flex::block::decompress(&c, src.len()).unwrap(), src);
    }

    #[test]
    fn malformed_blocks_are_errors() {
        assert!(decompress(&[0x1F, 1, 0, 0]).is_err()); // match offset past the output
        assert!(decompress(&[0xF0]).is_err()); // literal length runs off the end
    }
}
