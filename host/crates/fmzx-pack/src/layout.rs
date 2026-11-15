// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the image and the game pack sit in the FM-1's app area. One definition for the
//! package writer and for emu-gate, which places the pack the same way.

/// XIP address of the app area's start (flash 0x4000).
pub const AREA_XIP: u32 = 0x0200_0000;
/// Offset of the image (`app.bin`) in the app area: the boot loader enters it at 0x02000120.
pub const APP_OFFSET: u32 = 0x120;
/// XIP address the boot loader enters.
pub const APP_XIP: u32 = AREA_XIP + APP_OFFSET;
/// The app slot: image, padding and game pack together, from [`APP_OFFSET`].
pub const APP_SLOT: u32 = 0x8_DFBC;
/// The pack starts on a 4 KiB boundary of the XIP map.
pub const PACK_ALIGN: u32 = 4096;
/// The pack's first bytes (`app/pack.h`).
pub const PACK_MAGIC: &[u8; 8] = b"FMZXPACK";

/// Offset of the game pack in the app area for an image of `image_len` bytes: the first 4 KiB
/// boundary at or after the image's end, `align_up(0x120 + image_len, 4096)`.
pub fn pack_offset(image_len: usize) -> usize {
    (APP_OFFSET as usize + image_len).next_multiple_of(PACK_ALIGN as usize)
}

/// XIP address of the game pack for an image of `image_len` bytes.
pub fn pack_xip(image_len: usize) -> u32 {
    AREA_XIP + pack_offset(image_len) as u32
}

/// The bytes a game pack may take beside an image of `image_len` bytes: the app slot from the
/// pack's offset to its end (0 when the image alone fills it).
pub fn pack_room(image_len: usize) -> u32 {
    (APP_SLOT as usize).saturating_sub(pack_offset(image_len) - APP_OFFSET as usize) as u32
}

/// The app slot's contents (from [`APP_OFFSET`]): the image, 0xFF up to the pack, the pack.
/// Not padded to the slot's end. Errors when they do not fit the slot.
pub fn slot_contents(image: &[u8], pack: &[u8]) -> Result<Vec<u8>, String> {
    let at = pack_offset(image.len()) - APP_OFFSET as usize;
    let end = at + pack.len();
    if end > APP_SLOT as usize {
        return Err(format!(
            "image ({} bytes) and pack ({} bytes, from slot offset {at:#x}) need {end} bytes; the app slot has {} ({:#x})",
            image.len(),
            pack.len(),
            APP_SLOT,
            APP_SLOT
        ));
    }
    let mut s = image.to_vec();
    s.resize(at, 0xFF);
    s.extend_from_slice(pack);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_on_the_next_4k_boundary() {
        assert_eq!(pack_offset(0), 0x1000);
        assert_eq!(pack_offset(0x1000 - 0x120), 0x1000);
        assert_eq!(pack_offset(0x1000 - 0x120 + 1), 0x2000);
        assert_eq!(pack_xip(68_056), 0x0201_1000);
    }

    #[test]
    fn pack_room_is_what_slot_contents_takes() {
        let image = vec![0u8; 70_000];
        let room = pack_room(image.len()) as usize;
        assert!(slot_contents(&image, &vec![0; room]).is_ok());
        assert!(slot_contents(&image, &vec![0; room + 1]).is_err());
        assert_eq!(pack_room(APP_SLOT as usize), 0);
    }

    #[test]
    fn slot_contents_pads_with_ff() {
        let s = slot_contents(&[1, 2, 3], b"FMZXPACK").unwrap();
        assert_eq!(s.len(), 0x1000 - 0x120 + 8);
        assert_eq!(&s[..3], &[1, 2, 3]);
        assert!(s[3..0x1000 - 0x120].iter().all(|&b| b == 0xFF));
        assert_eq!(&s[0x1000 - 0x120..], b"FMZXPACK");
    }
}
