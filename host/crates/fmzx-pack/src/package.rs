// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The FM-1 ZX update package: the image and the game pack in the app slot ([`crate::layout`]),
//! written by [`crate::fwsc`] with Felucca's loader wrapped by [`crate::lz4blk`]; and the checks
//! of `fmzx-pack verify`, which read a package back the way fm1-emu's `fwsc::unpack` (MIT)
//! documents the format.

use crate::fwsc::{self, crc16, enc, Sdk, AREA, FLASH_SIZE, MARKER_BLOCKS, MARKER_UNUSED};
use crate::inputs::{sha256_hex, Pins, SDK_FILES};
use crate::layout::{self, APP_OFFSET, APP_SLOT, PACK_ALIGN, PACK_MAGIC};
use crate::lz4blk::{self, LOADER_LOAD, LOADER_NAME};

/// The product identity FM-1 ZX packages carry by default.
pub const PRODUCT: &str = "FM-1_948";
/// The marker at the start of Felucca's update loader.
pub const LOADER_MARKER: &[u8; 16] = b"FELUCCA-LOADER-1";

/// Checks that `product` is `FM-1_` and three digits.
pub fn check_product(product: &str) -> Result<(), String> {
    let digits = product.strip_prefix("FM-1_").filter(|d| d.len() == 3 && d.bytes().all(|c| c.is_ascii_digit()));
    match digits {
        Some(_) => Ok(()),
        None => Err(format!("product identity {product:?}: expected FM-1_ and three digits (FM-1 ZX uses {PRODUCT})")),
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Whether `image` carries `product` as a NUL-terminated string (`fmzx_product`).
pub fn image_has_identity(image: &[u8], product: &str) -> bool {
    let mut s = product.as_bytes().to_vec();
    s.push(0);
    find(image, &s).is_some()
}

/// The `.fwsc` package: `image` (`fmzx_app.bin`) at 0x120 in the app area, `pack` at the next
/// 4 KiB boundary, the slot padded with 0xFF; `loader` (`loader.bin`) as `ota.bin`.
pub fn build(image: &[u8], pack: &[u8], loader: &[u8], product: &str, sdk: &Sdk) -> Result<Vec<u8>, String> {
    check_product(product)?;
    if !image_has_identity(image, product) {
        return Err(format!("the image does not carry the identity {product} (FMZX_PRODUCT at build time)"));
    }
    if !pack.starts_with(PACK_MAGIC) {
        return Err("the game pack does not start with FMZXPACK".into());
    }
    if find(&loader[..loader.len().min(64)], LOADER_MARKER).is_none() {
        return Err("the loader has no FELUCCA-LOADER-1 marker".into());
    }
    let slot = layout::slot_contents(image, pack)?;
    let flash = fwsc::flash_image(&slot, fwsc::KEY, sdk)?;
    let ota = lz4blk::wrap_loader(loader)?;
    fwsc::ufw(&flash, &ota, product)
}

/// What `verify` found in a package.
#[derive(Debug, Default)]
pub struct Report {
    /// The identity the container's markers spell.
    pub identity: String,
    /// Length of the image (the app slot up to the pack, trailing 0xFF trimmed).
    pub image_len: usize,
    /// The game pack's offset in the app area and its game count.
    pub pack: Option<(usize, u32)>,
    /// SHA-256 of the update loader, unwrapped.
    pub loader_sha256: String,
    /// Everything that is wrong; empty for a good package.
    pub problems: Vec<String>,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
}

/// A 32-byte JLFS head (already unmasked): its CRC checks, data CRC, offset, size, flags, index,
/// name.
struct Head {
    ok: bool,
    dcrc: u16,
    offset: usize,
    size: usize,
    flags: u8,
    index: u16,
    name: String,
}

fn head(b: &[u8]) -> Head {
    Head {
        ok: crc16(&b[2..32]) == u16le(b, 0),
        dcrc: u16le(b, 2),
        offset: u32le(b, 4) as usize,
        size: u32le(b, 8) as usize,
        flags: b[12],
        index: u16le(b, 14),
        name: b[16..32].iter().take_while(|&&c| c != 0 && c != 0xFF).map(|&c| c as char).collect(),
    }
}

/// Checks a package. `pins` gives the SDK files' and the loader's expected SHA-256 (normally
/// [`Pins::builtin`]); `product`, when given, the identity it must carry.
pub fn verify(pkg: &[u8], pins: &Pins, product: Option<&str>) -> Report {
    let mut r = Report::default();
    if let Err(e) = verify_into(pkg, pins, product, &mut r) {
        r.problems.push(e);
    }
    r
}

fn verify_into(pkg: &[u8], pins: &Pins, product: Option<&str>, r: &mut Report) -> Result<(), String> {
    let p = &mut r.problems;
    if pkg.len() < MARKER_BLOCKS * 0x30 + 0x100 {
        return Err(format!("{} bytes: too short for an update package", pkg.len()));
    }
    // ---- identity markers, then the container without them
    let markers: Vec<u8> = (0..MARKER_BLOCKS).map(|i| pkg[i * 0x30 + 0x2F]).collect();
    let used = markers.iter().position(|&m| m == MARKER_UNUSED).unwrap_or(MARKER_BLOCKS);
    if markers[used..].iter().any(|&m| m != MARKER_UNUSED) {
        p.push("identity markers: a used slot after an unused one".into());
    }
    r.identity = markers[..used].iter().enumerate().map(|(i, &m)| m.wrapping_sub(i as u8 + 1) as char).collect();
    if let Err(e) = check_product(&r.identity) {
        p.push(e);
    }
    if let Some(want) = product.filter(|&w| w != r.identity) {
        p.push(format!("identity {}, expected {want}", r.identity));
    }
    let mut logical = Vec::with_capacity(pkg.len());
    for i in 0..MARKER_BLOCKS {
        logical.extend_from_slice(&pkg[i * 0x30..i * 0x30 + 0x2F]);
    }
    logical.extend_from_slice(&pkg[MARKER_BLOCKS * 0x30..]);
    // ---- UFW header and file list
    let mut hdr = logical[..0x40].to_vec();
    enc(&mut hdr, 0xFFFF);
    if crc16(&hdr[2..]) != u16le(&hdr, 0) {
        return Err("UFW header: CRC does not match (not an FM-1 update package)".into());
    }
    if &hdr[16..22] != fwsc::UFW_CHIP {
        p.push(format!("UFW header: chip {:?}, expected AC791N", String::from_utf8_lossy(&hdr[16..22])));
    }
    if u32le(&hdr, 4) as usize != logical.len() {
        p.push(format!("UFW header: total {} bytes, the container has {}", u32le(&hdr, 4), logical.len()));
    }
    let n = usize::from(u16le(&hdr, 8));
    let lst = logical.get(0x40..0x40 + n * 0x50).ok_or("UFW file list runs past the end")?;
    if crc16(lst) != u16le(&hdr, 2) {
        p.push("UFW file list: CRC does not match".into());
    }
    let (mut flash, mut ota) = (None, None);
    for e in lst.chunks(0x50) {
        let mut e = e.to_vec();
        enc(&mut e, 0xFFFF);
        let (typ, dcrc, off, len) = (u16le(&e, 0), u16le(&e, 4), u32le(&e, 8) as usize, u32le(&e, 12) as usize);
        let name: String = e[0x40..].iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        let Some(d) = logical.get(off..off + len) else {
            p.push(format!("{name}: runs past the end of the package"));
            continue;
        };
        if crc16(d) != dcrc {
            p.push(format!("{name}: CRC does not match"));
        }
        match (typ, name.as_str()) {
            (0, "flash.bin") => flash = Some(d),
            (100, "ota.bin") => ota = Some(d),
            _ => p.push(format!("unexpected file {name} (type {typ})")),
        }
    }
    match ota {
        Some(o) => check_loader(o, pins, r),
        None => r.problems.push("no ota.bin (type 100)".into()),
    }
    let flash = flash.ok_or("no flash.bin (type 0)")?;
    check_flash(flash, pins, product.unwrap_or(&r.identity.clone()), r)
}

fn check_sdk(p: &mut Vec<String>, pins: &Pins, name: &str, data: &[u8]) {
    match pins.sdk_sha256(name) {
        Some(want) if sha256_hex(data) == want => {}
        Some(want) => p.push(format!("{name}: SHA-256 {}, expected {want}", sha256_hex(data))),
        None => p.push(format!("{name}: no pin")),
    }
}

fn check_flash(f: &[u8], pins: &Pins, product: &str, r: &mut Report) -> Result<(), String> {
    let p = &mut r.problems;
    if f.len() != FLASH_SIZE {
        return Err(format!("flash.bin: {} bytes, expected {FLASH_SIZE:#x}", f.len()));
    }
    // ---- the head: flash header, then the JLFS entries
    let mut h = f[..32].to_vec();
    enc(&mut h, 0xFFFF);
    if crc16(&h[2..32]) != u16le(&h, 0) {
        p.push("flash header: CRC does not match".into());
    }
    let (mut key, mut area) = (None, None);
    for o in (32..AREA).step_by(32) {
        let mut b = f[o..o + 32].to_vec();
        enc(&mut b, 0xFFFF);
        let e = head(&b);
        if !e.ok {
            return Err(format!("flash head: the entry at {o:#x} does not check"));
        }
        let data = f.get(e.offset..e.offset.saturating_add(e.size));
        match e.name.as_str() {
            "uboot.boot" | "isd_config.ini" => {
                let d = data.filter(|_| e.offset + e.size <= AREA).ok_or(format!("{}: outside the head", e.name))?;
                if crc16(d) != e.dcrc {
                    p.push(format!("{}: CRC does not match", e.name));
                }
                if e.name == "uboot.boot" {
                    check_sdk(p, pins, "uboot.boot", d);
                } else if d.len() >= 34 && crc16(&d[..32]) == u16le(d, 32) {
                    key = Some(fwsc::chipkey_decode(&d[..32]));
                } else {
                    p.push("isd_config.ini: no chip key".into());
                }
            }
            "app_dir_head" if e.flags == 0x81 => area = Some(e.offset),
            _ => {}
        }
        if e.index != 0 {
            break; // the last entry
        }
    }
    let key = key.ok_or("flash head: no chip key")?;
    if area != Some(AREA) {
        return Err(format!("flash head: app area at {area:?}, expected {AREA:#x}"));
    }
    // ---- the app area, decrypted
    let mut a = f[AREA..].to_vec();
    fwsc::sfc(&mut a, 0, key);
    let ah = head(&a[..32]);
    let blk = ah.size;
    if !ah.ok || ah.name != "app_area_head" || ah.offset != 0x0200_0120 || blk > a.len() || blk < 64 {
        return Err("app area: no valid app_area_head".into());
    }
    if crc16(&a[32..blk]) != ah.dcrc {
        p.push("app area: CRC does not match".into());
    }
    let (app_off, slot_len) = (APP_OFFSET as usize, APP_SLOT as usize);
    let mut have_app = false;
    for o in (32..app_off).step_by(32) {
        let e = head(&a[o..o + 32]);
        if !e.ok {
            p.push(format!("app area: the entry at {o:#x} does not check"));
            continue;
        }
        let last = e.index != 0; // USR, the last entry
        let Some(d) = a.get(e.offset..e.offset.saturating_add(e.size)).filter(|_| e.flags == 0x82) else {
            if last {
                break;
            }
            continue; // a region descriptor: flash offsets, not in the area
        };
        match e.name.as_str() {
            "app.bin" => {
                have_app = true;
                if (e.offset, e.size) != (app_off, slot_len) {
                    p.push(format!("app.bin at {:#x}, {:#x} bytes; expected {app_off:#x}, {slot_len:#x}", e.offset, e.size));
                }
                if crc16(d) != e.dcrc {
                    p.push("app.bin: CRC does not match".into());
                }
            }
            "cfg_tool.bin" => {
                if crc16(d) != e.dcrc {
                    p.push("cfg_tool.bin: CRC does not match".into());
                }
                check_sdk(p, pins, "cfg_tool.bin", d);
            }
            _ => {}
        }
        if last {
            break;
        }
    }
    if !have_app {
        return Err("app area: no app.bin".into());
    }
    // the nested cfg directory after the block
    let cfg = a.get(blk..blk + 64).ok_or("app area: no cfg directory")?;
    let (ch, eh) = (head(&cfg[..32]), head(&cfg[32..]));
    let eq = a.get(blk + eh.offset..blk + eh.offset + eh.size);
    match eq {
        Some(eq) if ch.ok && eh.ok && ch.name == "cfg" && eh.name == "eq_cfg_hw.bin" => {
            if a.get(blk + 32..blk + ch.size).map(crc16) != Some(ch.dcrc) || crc16(eq) != eh.dcrc {
                p.push("cfg directory: CRC does not match".into());
            }
            check_sdk(p, pins, SDK_FILES[2], eq);
        }
        _ => p.push("app area: no valid cfg/eq_cfg_hw.bin".into()),
    }
    // ---- the image and the game pack in the slot
    let slot_end = app_off + slot_len;
    let pack_at = (1..)
        .map(|k| k * PACK_ALIGN as usize)
        .take_while(|&o| o + PACK_MAGIC.len() <= slot_end)
        .find(|&o| a[o..o + PACK_MAGIC.len()] == PACK_MAGIC[..]);
    let image_end = pack_at.unwrap_or(slot_end);
    let image = &a[app_off..image_end];
    r.image_len = image.iter().rposition(|&b| b != 0xFF).map_or(0, |i| i + 1);
    let image = &image[..r.image_len];
    if r.image_len == 0 {
        p.push("app.bin: no image".into());
    }
    if !image_has_identity(image, product) {
        p.push(format!("the image does not carry the identity {product}"));
    }
    match pack_at {
        None => p.push("no game pack (FMZXPACK) at a 4 KiB boundary in the app slot".into()),
        Some(at) => {
            if at != layout::pack_offset(r.image_len) {
                p.push(format!(
                    "game pack at {at:#x}, the image ({} bytes) puts it at {:#x}",
                    r.image_len,
                    layout::pack_offset(r.image_len)
                ));
            }
            let pack = &a[at..slot_end];
            match zx_sys::pack_open(pack) {
                Err(e) => p.push(format!("game pack at {at:#x} does not open ({e})")),
                Ok(n) => {
                    r.pack = Some((at, n));
                    for i in 0..n {
                        let ok = zx_sys::pack_game_at(pack, i)
                            .and_then(|g| g.extract(zx_sys::PACK_MAX_RAW as usize).map(|_| ()));
                        if let Err(e) = ok {
                            p.push(format!("game pack: game {i} does not extract ({e})"));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_loader(ota: &[u8], pins: &Pins, r: &mut Report) {
    let p = &mut r.problems;
    match unwrap_loader(ota) {
        Err(e) => p.push(format!("ota.bin: {e}")),
        Ok(image) => {
            r.loader_sha256 = sha256_hex(&image);
            if find(&image[..image.len().min(64)], LOADER_MARKER).is_none()
                || find(&ota[0x48.min(ota.len())..0x88.min(ota.len())], LOADER_MARKER).is_none()
            {
                p.push("ota.bin: no FELUCCA-LOADER-1 marker at the loader's start".into());
            }
            if image.len() != pins.loader.size || r.loader_sha256 != pins.loader.sha256 {
                p.push(format!(
                    "ota.bin: the loader ({} bytes, SHA-256 {}) is not the pinned one ({} bytes, {})",
                    image.len(),
                    r.loader_sha256,
                    pins.loader.size,
                    pins.loader.sha256
                ));
            }
        }
    }
}

/// The loader image from `ota.bin`, every head and CRC checked.
pub fn unwrap_loader(ota: &[u8]) -> Result<Vec<u8>, String> {
    if ota.len() < 72 {
        return Err("too short".into());
    }
    let (outer, inner) = (head(&ota[..32]), head(&ota[32..64]));
    let body = &ota[32..];
    let name = String::from_utf8_lossy(LOADER_NAME);
    if !outer.ok || outer.offset != 0x20 || outer.size != body.len() || outer.flags != 0x41 || outer.name != name {
        return Err("the outer head does not check".into());
    }
    if crc16(body) != outer.dcrc {
        return Err("CRC does not match".into());
    }
    if !inner.ok || inner.size != LOADER_LOAD as usize || inner.name != name {
        return Err("the inner head does not check".into());
    }
    let (len, mut image, mut o) = (inner.offset, Vec::new(), 64);
    while o < ota.len() {
        if o + 8 > ota.len() {
            return Err("a block header runs past the end".into());
        }
        let (clen, dlen) = (u32le(ota, o) as usize, u32le(ota, o + 4) as usize);
        let blk = ota.get(o + 8..o + 8 + clen).ok_or("a block runs past the end")?;
        let d = lz4blk::decompress(blk)?;
        if d.len() != dlen || dlen > lz4blk::BLOCK {
            return Err("a block's length does not match".into());
        }
        image.extend_from_slice(&d);
        o += 8 + clen;
    }
    if image.len() != len || crc16(&image) != inner.dcrc {
        return Err("the loader's length or CRC does not match its head".into());
    }
    Ok(image)
}
