// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_sys::*;
use zx_tests::*;

fn entry_crc(pack: &[u8], i: usize) -> u32 {
    let at = 32 + 160 * i + 44;
    u32::from_le_bytes(pack[at..at + 4].try_into().unwrap())
}

fn put32(h: &mut [u8], at: usize, v: u32) {
    h[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Rewrites the header CRC after a field was changed.
fn fix_crc(h: &mut [u8; SAVE_HEADER_SIZE]) {
    let crc = pack_crc32(&h[..60]);
    put32(h, 60, crc);
}

fn good(pack: &[u8]) -> ([u8; SAVE_HEADER_SIZE], SaveInfo) {
    let info = SaveInfo { game: 1, game_crc: entry_crc(pack, 1), tape_pos: 0x1234, z80_crc: 0xDEAD_BEEF };
    (save_header_build(&info), info)
}

#[test]
fn z80_round_trip() {
    let regs = sample_regs();
    let ram = sample_ram();
    let mut f = save_z80_header(&regs, 5).to_vec();
    f.extend_from_slice(&ram);
    assert_eq!(f.len(), SAVE_Z80_LEN);
    let mut m = Machine::new();
    m.load_z80(&f).unwrap();
    assert_eq!(format!("{:?}", m.regs()), format!("{:?}", regs));
    assert_eq!(m.regs().r & 0x80, 0x80);
    assert_eq!(m.border(), 5);
    for (i, b) in ram.iter().enumerate() {
        assert_eq!(m.peek(0x4000 + i as u16), *b, "ram {i}");
    }
    // the same bytes as the test fixture's header, which has border 3
    let mut want = make_z80_v1(&regs, &ram, false);
    let mut got = f.clone();
    want[12] &= !0x0E;
    got[12] &= !0x0E;
    assert_eq!(got, want);
    assert_eq!(f[12] & 0x20, 0, "uncompressed");
}

#[test]
fn header_round_trip() {
    let pack = build_pack(&["Alpha", "Beta"]);
    let (h, info) = good(&pack);
    assert_eq!(&h[..8], b"FMZXSAVE");
    assert_eq!(save_header_check(&h, Some(&pack)), Ok(info));
}

#[test]
fn header_rejects_each_fault() {
    let pack = build_pack(&["Alpha", "Beta"]);
    let (h, _) = good(&pack);
    let bad = |h: &[u8; SAVE_HEADER_SIZE]| assert_eq!(save_header_check(h, Some(&pack)), Err(ZX_ERR_FORMAT));

    let mut x = h;
    x[3] ^= 1;
    fix_crc(&mut x);
    bad(&x);
    let mut x = h;
    x[8] = 2;
    fix_crc(&mut x);
    bad(&x);
    for i in 0..60 {
        let mut x = h;
        x[i] ^= 0x40;
        bad(&x);
    }
    let mut x = h;
    put32(&mut x, 20, SAVE_Z80_LEN as u32 - 1);
    fix_crc(&mut x);
    bad(&x);
    let mut x = h;
    x[10] = 2;
    fix_crc(&mut x);
    bad(&x);
    let mut x = h;
    put32(&mut x, 12, entry_crc(&pack, 1) + 1);
    fix_crc(&mut x);
    bad(&x);
    assert_eq!(save_header_check(&h, None), Err(ZX_ERR_FORMAT));
}

#[test]
fn blank_areas_are_no_save() {
    let pack = build_pack(&["Alpha", "Beta"]);
    assert_eq!(save_header_check(&[0xFF; SAVE_HEADER_SIZE], Some(&pack)), Err(ZX_ERR_FORMAT));
    let mut probe = [0xFFu8; SAVE_HEADER_SIZE];
    probe[..9].copy_from_slice(b"FMZXPROBE");
    assert_eq!(save_header_check(&probe, Some(&pack)), Err(ZX_ERR_FORMAT));
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    let mut r = [0u8; SAVE_HEADER_SIZE];
    for b in r.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s >> 24) as u8;
    }
    assert_eq!(save_header_check(&r, Some(&pack)), Err(ZX_ERR_FORMAT));
}

#[test]
fn tape_pos_boundaries() {
    let mut tap = tap_block(0x00, &[7u8; 17]);
    tap.extend(tap_block(0xFF, &[9u8; 98]));
    assert_eq!(tap.len(), 123);
    for p in [0, 21, 123] {
        assert!(save_tape_pos_ok(&tap, p), "{p}");
    }
    for p in [1, 20, 22, 124] {
        assert!(!save_tape_pos_ok(&tap, p), "{p}");
    }
}

#[test]
fn area_offsets_skip_sector_tails() {
    assert_eq!(SAVE_AREA_LEN, 0xD000);
    assert_eq!(SAVE_LEN, SAVE_HEADER_SIZE + SAVE_Z80_LEN);
    assert_eq!(save_area_offset(0), 0);
    assert_eq!(save_area_offset(3839), 3839);
    assert_eq!(save_area_offset(3840), 0x1000);
    assert_eq!(save_area_offset(SAVE_LEN as u32 - 1), 12 * 0x1000 + 3165);
    let mut prev = None;
    for p in 0..SAVE_LEN as u32 {
        let o = save_area_offset(p);
        assert!(o % 0x1000 < 0xF00);
        assert!((o as usize) < SAVE_AREA_LEN);
        if let Some(q) = prev {
            assert!(o > q);
        }
        prev = Some(o);
    }
}
