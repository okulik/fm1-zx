// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game pack writer; the layout is in `docs/pack-format.md`.

use crate::manifest::{encode_keymap, game_path, parse_manifest, INPUTS};
use std::path::Path;
use zx_sys::{Machine, PACK_ENTRY_SIZE, PACK_HEADER_SIZE, PACK_LZ4, PACK_MAX_RAW, PACK_SNA, PACK_STORED, PACK_TAP, PACK_Z80};

const MAGIC: &[u8; 8] = b"FMZXPACK";
const VERSION: u16 = 1;
// header field offsets
const H_VERSION: usize = 8;
const H_COUNT: usize = 10;
const H_TOTAL: usize = 12;
const H_CRC: usize = 16;
// entry field offsets
const E_KIND: usize = 29;
const E_CODEC: usize = 30;
const E_CROP: usize = 31;
const E_OFFSET: usize = 32;
const E_STORED: usize = 36;
const E_RAW: usize = 40;
const E_CRC: usize = 44;
const E_KEYS: usize = 48;

const SNAP_FRAMES: u32 = 50;
const TAPE_FRAMES: u32 = 500;

fn kind_of(file: &str) -> Result<u8, String> {
    let ext = Path::new(file).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("sna") => Ok(PACK_SNA),
        Some("z80") => Ok(PACK_Z80),
        Some("tap") => Ok(PACK_TAP),
        _ => Err("unknown file type (expected .sna, .z80 or .tap)".to_string()),
    }
}

/// Loads the game in the host core and runs it for a while, as the firmware will.
fn validate(kind: u8, data: &[u8]) -> Result<(), String> {
    let mut m = Machine::new();
    let frames = match kind {
        PACK_SNA => {
            m.load_sna(data).map_err(|e| format!("not a loadable .sna (error {e})"))?;
            SNAP_FRAMES
        }
        PACK_Z80 => {
            m.load_z80(data).map_err(|e| format!("not a loadable .z80 (error {e})"))?;
            SNAP_FRAMES
        }
        _ => {
            m.tape_insert(data).map_err(|e| format!("not a valid .tap (error {e})"))?;
            m.tape_autostart();
            TAPE_FRAMES
        }
    };
    for _ in 0..frames {
        m.run_frame();
    }
    Ok(())
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// Builds the pack image from a manifest file; game files are resolved against its directory.
pub fn build_pack(manifest_path: &Path) -> Result<Vec<u8>, String> {
    let text = std::fs::read_to_string(manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let dir = manifest_path.parent().unwrap_or(Path::new("."));
    let count = u16::try_from(manifest.game.len()).map_err(|_| "too many games".to_string())?;

    let mut pack = vec![0u8; PACK_HEADER_SIZE + PACK_ENTRY_SIZE * manifest.game.len()];
    pack[..8].copy_from_slice(MAGIC);
    pack[H_VERSION..H_VERSION + 2].copy_from_slice(&VERSION.to_le_bytes());
    pack[H_COUNT..H_COUNT + 2].copy_from_slice(&count.to_le_bytes());

    for (i, g) in manifest.game.iter().enumerate() {
        let fail = |e: String| format!("{}: {e}", g.file);
        let kind = kind_of(&g.file).map_err(fail)?;
        let path = game_path(dir, g);
        let raw = std::fs::read(&path).map_err(|e| format!("{}: {e}", g.file))?;
        if raw.len() > PACK_MAX_RAW as usize {
            return Err(fail(format!("{} bytes exceeds the {PACK_MAX_RAW}-byte limit", raw.len())));
        }
        validate(kind, &raw).map_err(fail)?;

        let packed = lz4_flex::block::compress(&raw);
        let (codec, stored): (u8, &[u8]) = if packed.len() < raw.len() { (PACK_LZ4, &packed) } else { (PACK_STORED, &raw) };
        let keys = encode_keymap(&g.keys).map_err(|e| format!("game {}: {e}", g.id))?;

        while pack.len() % 4 != 0 {
            pack.push(0);
        }
        let offset = u32::try_from(pack.len()).map_err(|_| fail("pack grows past 4 GiB".to_string()))?;
        pack.extend_from_slice(stored);

        let e = PACK_HEADER_SIZE + PACK_ENTRY_SIZE * i;
        pack[e..e + g.name.len()].copy_from_slice(g.name.as_bytes());
        pack[e + E_KIND] = kind;
        pack[e + E_CODEC] = codec;
        pack[e + E_CROP] = g.crop;
        put32(&mut pack, e + E_OFFSET, offset);
        // both lengths are at most PACK_MAX_RAW-ish (raw checked above; LZ4 output is smaller than raw)
        put32(&mut pack, e + E_STORED, stored.len() as u32);
        put32(&mut pack, e + E_RAW, raw.len() as u32);
        put32(&mut pack, e + E_CRC, crc32fast::hash(&raw));
        for (k, act) in keys.iter().enumerate().take(INPUTS) {
            pack[e + E_KEYS + 2 * k] = act[0];
            pack[e + E_KEYS + 1 + 2 * k] = act[1];
        }
    }

    while pack.len() % 4 != 0 {
        pack.push(0);
    }
    let total = u32::try_from(pack.len()).map_err(|_| "pack grows past 4 GiB".to_string())?;
    put32(&mut pack, H_TOTAL, total);
    let crc = crc32fast::hash(&pack[PACK_HEADER_SIZE..]);
    put32(&mut pack, H_CRC, crc);
    Ok(pack)
}
