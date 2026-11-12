// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Copyright (C) 2026 Leo Kuroshita (@kurogedelic), Hügelton Instruments
//
// A Rust port of Felucca's tools/fm1pkg_make.py (https://github.com/hugelton/Felucca, commit
// a53ecf428c29c780108fd70a31b71a32a6b5c313, Felucca 1.6.1), GPL-3.0-only like the original. The
// FM-1 ZX firmware image never links this file: it is part of the host tool that builds the
// update package.

//! The FM-1 update package (`.fwsc`): a JieLi UFW container with the flash image (`flash.bin`)
//! and the update loader (`ota.bin`).
//!
//! `flash.bin`, 0x93000 bytes:
//! - head `[0, 0x4000)`: the flash header (JieLi SDK default values), the JLFS entries, the SDK's
//!   boot loader (`uboot.boot`, Apache-2.0) and an `isd_config` blob of our own that decodes to
//!   the chip key. The update loader never writes the head; it only has to look like a package's.
//! - app area from 0x4000: `app_area_head`, `app.bin` (the app slot), the SDK's `cfg_tool.bin`,
//!   the region descriptors and the nested `cfg` directory with the SDK's `eq_cfg_hw.bin`, all
//!   SFC-encrypted with the chip key.
//!
//! The layout values (offsets, flags, descriptor addresses, header fields) are facts of the FM-1
//! flash layout and of the SDK's file formats.

use crate::layout::{APP_OFFSET, APP_SLOT};

/// Size of `flash.bin`.
pub const FLASH_SIZE: usize = 0x93000;
/// Where the app area starts in flash (XIP 0x02000000).
pub const AREA: usize = 0x4000;
/// The chip key the app area is encrypted with.
pub const KEY: u16 = 0x980F;
// The header values are the defaults of JieLi's AC79 SDK, whose files are Apache-2.0; marked as
// a snippet of it to be on the safe side.
// SPDX-SnippetBegin
// SPDX-SnippetCopyrightText: Zhuhai Jieli Technology Co., Ltd.
// SPDX-License-Identifier: Apache-2.0
/// The flash header: JieLi SDK defaults (burner_size 544, VID "0.01", flash_size 0xFF000, fs_ver
/// 0x10, PID "AC791N_STORY"), plain, without its CRC.
pub const FLASH_HDR: [u8; 30] = [
    0x20, 0x02, 0x2f, 0x10, 0x4c, 0xc9, 0x00, 0xf0, 0x0f, 0x00, 0x10, 0x10, 0xa4, 0xff, 0x4e, 0x5d, 0x0b, 0x41,
    0xe0, 0xcd, 0x78, 0x1d, 0xc8, 0x56, 0x41, 0x7f, 0xb3, 0x67, 0xcf, 0x9f,
];
// SPDX-SnippetEnd
/// The chip the UFW container is for.
pub const UFW_CHIP: &[u8] = b"AC791N";
/// Marker of an unused identity slot in the container's header blocks.
pub const MARKER_UNUSED: u8 = 0x7D;
/// Header blocks of the container, one identity marker each.
pub const MARKER_BLOCKS: usize = 20;

/// The three files from JieLi's AC79 SDK (`cpu/wl82/tools/`) that go into `flash.bin`.
pub struct Sdk {
    /// `uboot.boot`, the SDK's second-stage boot loader.
    pub uboot: Vec<u8>,
    /// `cfg_tool.bin`.
    pub cfg_tool: Vec<u8>,
    /// `cfg/eq_cfg_hw.bin`, the SDK's default EQ table.
    pub eq_cfg: Vec<u8>,
}

/// CRC-16/CCITT (polynomial 0x1021, initial value 0, no reflection), as JieLi's tools use it.
pub fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |crc, &b| {
        let mut c = crc ^ (u16::from(b) << 8);
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
        c
    })
}

/// JieLi's ENC stream cipher, in place (its own inverse).
pub fn enc(buf: &mut [u8], mut key: u16) {
    for b in buf {
        *b ^= key as u8;
        key = (key << 1) ^ if key & 0x8000 != 0 { 0x1021 } else { 0 };
    }
}

/// The SFC cipher: ENC per 32-byte block, the key mixed with the block's offset in the app area
/// (`at` is `buf[0]`'s offset there).
pub fn sfc(buf: &mut [u8], at: usize, key: u16) {
    for (i, blk) in buf.chunks_mut(32).enumerate() {
        enc(blk, key ^ ((at + i * 32) >> 2) as u16);
    }
}

/// The chip key a 32-byte `isd_config` blob decodes to.
pub fn chipkey_decode(d: &[u8]) -> u16 {
    let sum = d[..16].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    let sum = if sum >= 0xE0 {
        0xAA
    } else if sum <= 0x10 {
        0x55
    } else {
        sum
    };
    (0..16).filter(|&i| (d[16 + i] ^ d[15 - i]) < sum).fold(0u16, |k, i| k | 1 << i)
}

/// A JLFS entry's name field: a name (NUL-terminated, 0xFF after), or 16 raw bytes (a name
/// followed by the SDK packer's own fields).
pub enum Name<'a> {
    Str(&'a str),
    Raw([u8; 16]),
}

/// A 32-byte JLFS entry: its CRC, then data CRC, offset, size, flags, a reserved byte, index and
/// name.
pub fn entry(dcrc: u16, offset: u32, size: u32, flags: u8, resvd: u8, index: u16, name: Name) -> [u8; 32] {
    let nm = match name {
        Name::Raw(b) => b,
        Name::Str(s) => {
            let mut b = [0xFF; 16];
            b[..s.len()].copy_from_slice(s.as_bytes());
            b[s.len()] = 0;
            b
        }
    };
    let mut e = [0u8; 32];
    e[2..4].copy_from_slice(&dcrc.to_le_bytes());
    e[4..8].copy_from_slice(&offset.to_le_bytes());
    e[8..12].copy_from_slice(&size.to_le_bytes());
    e[12] = flags;
    e[13] = resvd;
    e[14..16].copy_from_slice(&index.to_le_bytes());
    e[16..32].copy_from_slice(&nm);
    let c = crc16(&e[2..]);
    e[..2].copy_from_slice(&c.to_le_bytes());
    e
}

/// 32 bytes that [`chipkey_decode`] turns into `key`, then their CRC-16.
pub fn key_blob(key: u16) -> [u8; 34] {
    let mut d = [0u8; 34];
    d[..16].fill(0x10); // sum 0 -> threshold 0x55
    for i in 0..16 {
        d[16 + i] = d[15 - i] ^ if key >> i & 1 != 0 { 0x00 } else { 0xFF };
    }
    debug_assert_eq!(chipkey_decode(&d[..32]), key);
    let c = crc16(&d[..32]);
    d[32..].copy_from_slice(&c.to_le_bytes());
    d
}

fn put(f: &mut [u8], at: usize, data: &[u8]) {
    f[at..at + data.len()].copy_from_slice(data);
}

/// `flash.bin` for the app slot contents `slot` (at most [`APP_SLOT`] bytes; padded with 0xFF).
pub fn flash_image(slot: &[u8], key: u16, sdk: &Sdk) -> Result<Vec<u8>, String> {
    let (spl, cfg_tool, eq) = (&sdk.uboot, &sdk.cfg_tool, &sdk.eq_cfg);
    let mut isd = key_blob(key).to_vec();
    isd.extend_from_slice(b"[FELUCCA]\r\n");
    let mut f = vec![0xFFu8; FLASH_SIZE];
    // ---- head
    let spl_off = 0xA0usize;
    let isd_off = spl_off + spl.len();
    if isd_off + isd.len() > AREA {
        return Err(format!("uboot.boot is {} bytes: the flash head has no room for it", spl.len()));
    }
    put(&mut f, spl_off, spl);
    put(&mut f, isd_off, &isd);
    let mut head = Vec::with_capacity(5 * 32);
    head.extend_from_slice(&crc16(&FLASH_HDR).to_le_bytes());
    head.extend_from_slice(&FLASH_HDR);
    head.extend_from_slice(&entry(crc16(spl), spl_off as u32, spl.len() as u32, 0x00, 0x00, 0, Name::Str("uboot.boot")));
    head.extend_from_slice(&entry(crc16(&isd), isd_off as u32, isd.len() as u32, 0x02, 0x80, 0, Name::Str("isd_config.ini")));
    head.extend_from_slice(&entry(0xFFFF, AREA as u32, 0xFFFF_FFFF, 0x81, 0xFF, 0, Name::Str("app_dir_head")));
    head.extend_from_slice(&entry(0xFFFF, 0xFF000, 0x1000, 0x12, 0x01, 0xFFFF, Name::Str("key_mac")));
    for blk in head.chunks_mut(32) {
        // the header and each JLFS entry: ENC from 0xFFFF
        enc(blk, 0xFFFF);
    }
    put(&mut f, 0, &head);
    // ---- app area (plain first, then SFC)
    let (app_off, slot_len) = (APP_OFFSET as usize, APP_SLOT as usize);
    if slot.len() > slot_len {
        return Err(format!("the app slot's contents are {} bytes, the slot is {slot_len} bytes", slot.len()));
    }
    let mut app = slot.to_vec();
    app.resize(slot_len, 0xFF);
    let blk = app_off + slot_len + cfg_tool.len(); // app_area_head block size
    let mut area = vec![0xFFu8; blk];
    put(&mut area, app_off, &app);
    put(&mut area, app_off + slot_len, cfg_tool);
    let raw = |s: &[u8]| -> [u8; 16] { s.try_into().expect("16 bytes") };
    let ents = [
        entry(crc16(&app), app_off as u32, slot_len as u32, 0x82, 0xFF, 0, Name::Str("app.bin")),
        entry(crc16(cfg_tool), (app_off + slot_len) as u32, cfg_tool.len() as u32, 0x82, 0xFF, 0, Name::Str("cfg_tool.bin")),
        // region descriptors; the bytes after the names are SDK packer fields (VM size, "CODE", "AUTO")
        entry(0xFFFF, 0x93000, 352_256, 0x12, 0x80, 0, Name::Raw(raw(b"VM\0\xff\xff\xff\xff\xff\xff\xff\xff\xff\x00\x80\x00\x00"))),
        entry(0xFFFF, 0, FLASH_SIZE as u32, 0x92, 0x82, 0, Name::Raw(raw(b"PRCT\0\xff\xff\xff\xff\xff\xff\xffCODE"))),
        entry(0xFFFF, 0xE9000, 0x1000, 0x92, 0x80, 0, Name::Raw(raw(b"BTIF\0\xff\xff\xff\xff\xff\xff\xffAUTO"))),
        entry(0xFFFF, 0xEA000, 0x12000, 0x92, 0x81, 1, Name::Str("USR")),
    ];
    let ents = ents.concat();
    put(&mut area, 32, &ents);
    let area_head = entry(crc16(&area[32..blk]), 0x0200_0120, blk as u32, 0x83, 0xFF, 0, Name::Str("app_area_head"));
    put(&mut area, 0, &area_head);
    // the nested cfg directory right after the block: the SDK's default EQ table
    let mut cfg_body = entry(crc16(eq), 0x40, eq.len() as u32, 0x82, 0xFF, 1, Name::Str("eq_cfg_hw.bin")).to_vec();
    cfg_body.extend_from_slice(eq);
    let mut region = area;
    region.extend_from_slice(&entry(crc16(&cfg_body), 0x20, (0x20 + cfg_body.len()) as u32, 0x83, 0xFF, 1, Name::Str("cfg")));
    region.extend_from_slice(&cfg_body);
    if AREA + region.len() > FLASH_SIZE {
        return Err(format!("the app area ({} bytes) runs past flash.bin", region.len()));
    }
    sfc(&mut region, 0, key);
    put(&mut f, AREA, &region);
    Ok(f)
}

/// Checks that `product` fits the container's identity markers: at most 20 characters, none
/// encoding to the unused marker.
pub fn check_product(product: &str) -> Result<(), String> {
    let bad = product.bytes().enumerate().any(|(i, c)| c.wrapping_add(i as u8 + 1) == MARKER_UNUSED);
    if product.len() > MARKER_BLOCKS || !product.is_ascii() || bad {
        return Err(format!("product {product:?}: at most 20 characters, none encoding to the unused marker"));
    }
    Ok(())
}

/// The `.fwsc` file: a UFW container (chip `AC791N`) with `flash.bin` (type 0) and `ota.bin`
/// (type 100), one identity marker after each of its first 20 0x2F-byte blocks.
pub fn ufw(flash: &[u8], ota: &[u8], product: &str) -> Result<Vec<u8>, String> {
    check_product(product)?;
    let files: [(u16, &str, &[u8]); 2] = [(0, "flash.bin", flash), (100, "ota.bin", ota)];
    let mut off = 0x400usize;
    let mut ents = Vec::new();
    for (i, (typ, name, d)) in files.iter().enumerate() {
        off = (off + 0xFF) & !0xFF;
        let mut e = [0u8; 0x50];
        e[0..2].copy_from_slice(&typ.to_le_bytes());
        e[2..4].copy_from_slice(&(i as u16).to_le_bytes());
        e[4..6].copy_from_slice(&crc16(d).to_le_bytes());
        e[8..12].copy_from_slice(&(off as u32).to_le_bytes());
        e[12..16].copy_from_slice(&(d.len() as u32).to_le_bytes());
        e[16..20].copy_from_slice(&(((d.len() + 31) & !31) as u32).to_le_bytes());
        e[0x40..0x40 + name.len()].copy_from_slice(name.as_bytes());
        ents.push((e, off, *d));
        off += d.len();
    }
    let total = off;
    let mut hdr = [0u8; 0x40];
    hdr[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    hdr[8..10].copy_from_slice(&(files.len() as u16).to_le_bytes());
    // header fields as written by the SDK packer
    hdr[10..12].copy_from_slice(&0x0004u16.to_le_bytes());
    hdr[12..14].copy_from_slice(&0x0200u16.to_le_bytes());
    hdr[16..16 + UFW_CHIP.len()].copy_from_slice(UFW_CHIP);
    let mut lst = Vec::new();
    for (e, _, _) in &ents {
        let mut x = *e;
        enc(&mut x, 0xFFFF);
        lst.extend_from_slice(&x);
    }
    let c = crc16(&lst);
    hdr[2..4].copy_from_slice(&c.to_le_bytes());
    let c = crc16(&hdr[2..]);
    hdr[0..2].copy_from_slice(&c.to_le_bytes());
    enc(&mut hdr, 0xFFFF);
    let mut logical = vec![0xFFu8; total];
    put(&mut logical, 0, &hdr);
    put(&mut logical, 0x40, &lst);
    for (_, o, d) in &ents {
        put(&mut logical, *o, d);
    }
    let mut out = Vec::with_capacity(total + MARKER_BLOCKS);
    for i in 0..MARKER_BLOCKS {
        let m = product.as_bytes().get(i).map_or(MARKER_UNUSED, |&c| c.wrapping_add(i as u8 + 1));
        out.extend_from_slice(&logical[i * 0x2F..(i + 1) * 0x2F]);
        out.push(m);
    }
    out.extend_from_slice(&logical[MARKER_BLOCKS * 0x2F..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_blob_decodes_to_the_key() {
        for key in [0x0000, 0x980F, 0xFFFF, 0x1234] {
            assert_eq!(chipkey_decode(&key_blob(key)[..32]), key);
        }
    }

    #[test]
    fn entry_names() {
        let e = entry(0, 0, 0, 0, 0, 0, Name::Str("app.bin"));
        assert_eq!(&e[16..24], b"app.bin\0");
        assert!(e[24..].iter().all(|&b| b == 0xFF));
        assert_eq!(u16::from_le_bytes([e[0], e[1]]), crc16(&e[2..]));
    }

    #[test]
    fn crc16_check_value() {
        // CRC-16/XMODEM's check value
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }
}
