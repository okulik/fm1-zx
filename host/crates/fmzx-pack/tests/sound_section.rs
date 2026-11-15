// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The pack's sound section (pack version 2): the writer, both readers, and a bad section never
//! breaking the games.

use fmzx_pack::pack::{attach_sound, build_pack, sound_list};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use zx_sys::{pack_crc32, pack_game_at, pack_open, pack_sound_at, ZX_ERR_FORMAT};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// A 3-game pack from the repo's test programs, with no sound section.
fn pack3() -> Vec<u8> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("fmzx-sound-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let mut text = String::new();
    for (id, file) in [("a", "border.sna"), ("b", "scroll.sna"), ("c", "tune.sna")] {
        std::fs::copy(repo().join("games/test").join(file), dir.join(file)).unwrap();
        text += &format!("[[game]]\nid = \"{id}\"\nname = \"Game {id}\"\nfile = \"{file}\"\n\n");
    }
    let manifest = dir.join("games.toml");
    std::fs::write(&manifest, text).unwrap();
    let pack = build_pack(&manifest).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    pack
}

fn list(seed: u8, words: usize) -> Vec<u8> {
    (0..words * 4).map(|i| seed.wrapping_add(i as u8)).collect()
}

fn rd32(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
}

fn put32(p: &mut [u8], at: usize, x: u32) {
    p[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

fn fix_crc(p: &mut [u8]) {
    let c = pack_crc32(&p[32..]);
    put32(p, 16, c);
}

fn sound_at(p: &[u8]) -> usize {
    rd32(p, 20) as usize
}

fn games_still_work(p: &[u8]) {
    assert_eq!(pack_open(p), Ok(3));
    for i in 0..3 {
        let g = pack_game_at(p, i).unwrap();
        assert_eq!(g.extract(131_072).unwrap().len() as u32, g.raw_len());
    }
}

#[test]
fn sound_roundtrip() {
    let mut p = pack3();
    let (a, b) = (list(1, 3), list(9, 5));
    attach_sound(&mut p, &[Some(a.clone()), None, Some(b.clone())]).unwrap();
    assert_eq!(u16::from_le_bytes([p[8], p[9]]), 2);
    assert_eq!(rd32(&p, 12) as usize, p.len());
    assert_eq!(sound_at(&p) % 4, 0);
    assert_eq!(sound_at(&p) + rd32(&p, 24) as usize, p.len());
    assert_eq!(rd32(&p, 28), 0);
    games_still_work(&p);
    assert_eq!(sound_list(&p, 0), Some(&a[..]));
    assert_eq!(sound_list(&p, 1), None);
    assert_eq!(sound_list(&p, 2), Some(&b[..]));
    assert_eq!(sound_list(&p, 3), None);
    assert_eq!(pack_sound_at(&p, 0), Ok(Some(a)));
    assert_eq!(pack_sound_at(&p, 1), Ok(None));
    assert_eq!(pack_sound_at(&p, 2), Ok(Some(b)));
    assert_eq!(pack_sound_at(&p, 3), Err(ZX_ERR_FORMAT));
}

#[test]
fn attach_rejects_misuse_and_all_none_adds_nothing() {
    let plain = pack3();
    let mut p = plain.clone();
    attach_sound(&mut p, &[None, None, None]).unwrap();
    assert_eq!(p, plain);
    assert!(attach_sound(&mut p, &[Some(list(1, 1))]).is_err(), "wrong count");
    assert!(attach_sound(&mut p, &[Some(vec![]), None, None]).is_err(), "empty list");
    attach_sound(&mut p, &[Some(list(1, 1)), None, None]).unwrap();
    assert!(attach_sound(&mut p, &[Some(list(1, 1)), None, None]).is_err(), "second section");
    assert!(attach_sound(&mut vec![0u8; 8], &[]).is_err(), "not a pack");
}

#[test]
fn unaligned_list_is_padded() {
    let mut p = pack3();
    attach_sound(&mut p, &[Some(vec![1, 2, 3, 4, 5]), None, None]).unwrap();
    assert_eq!(sound_list(&p, 0), Some(&[1, 2, 3, 4, 5, 0, 0, 0][..]));
    assert_eq!(rd32(&p, 12) as usize, p.len());
}

#[test]
fn no_section_is_none() {
    let p = pack3();
    assert_eq!(rd32(&p, 20), 0);
    assert_eq!(rd32(&p, 24), 0);
    for i in 0..3 {
        assert_eq!(pack_sound_at(&p, i), Ok(None));
        assert_eq!(sound_list(&p, i), None);
    }
}

/// A 3-game pack with a section, then `f` patches it and the CRC is fixed.
fn bad(f: impl FnOnce(&mut Vec<u8>, usize)) -> Vec<u8> {
    let mut p = pack3();
    attach_sound(&mut p, &[Some(list(1, 3)), Some(list(5, 2)), None]).unwrap();
    let s = sound_at(&p);
    f(&mut p, s);
    fix_crc(&mut p);
    p
}

fn assert_bad(p: &[u8]) {
    games_still_work(p);
    for i in 0..3 {
        assert_eq!(pack_sound_at(p, i), Err(ZX_ERR_FORMAT), "game {i}");
    }
    for i in 0..3 {
        assert_eq!(sound_list(p, i), None, "game {i}");
    }
}

#[test]
fn sound_section_bad_offset_past_total() {
    assert_bad(&bad(|p, _| {
        let total = p.len() as u32;
        put32(p, 20, total + 4);
    }));
}

#[test]
fn sound_section_bad_length_past_total() {
    assert_bad(&bad(|p, _| {
        let total = p.len() as u32;
        put32(p, 24, total);
    }));
}

#[test]
fn sound_section_bad_offset_wraps() {
    assert_bad(&bad(|p, _| {
        put32(p, 20, 0xFFFF_FFFC);
        put32(p, 24, 0x10);
    }));
}

#[test]
fn sound_section_bad_one_header_field_zero() {
    assert_bad(&bad(|p, _| put32(p, 24, 0)));
}

#[test]
fn sound_section_bad_count() {
    assert_bad(&bad(|p, s| p[s + 2] = 2));
}

#[test]
fn sound_section_bad_version() {
    assert_bad(&bad(|p, s| p[s] = 2));
}

#[test]
fn sound_section_bad_list_offset_past_section() {
    let p = bad(|p, s| {
        let len = rd32(p, 24);
        put32(p, s + 4, len);
    });
    games_still_work(&p);
    assert_eq!(pack_sound_at(&p, 0), Err(ZX_ERR_FORMAT));
    assert_eq!(sound_list(&p, 0), None);
    assert_eq!(pack_sound_at(&p, 2), Ok(None));
}

#[test]
fn sound_section_bad_list_offset_unaligned_or_in_table() {
    for off in [21u32, 8] {
        let p = bad(|p, s| put32(p, s + 4, off));
        games_still_work(&p);
        assert_eq!(pack_sound_at(&p, 0), Err(ZX_ERR_FORMAT), "{off}");
        assert_eq!(sound_list(&p, 0), None, "{off}");
    }
}

#[test]
fn sound_section_bad_section_inside_the_entry_table() {
    assert_bad(&bad(|p, _| put32(p, 20, 32)));
}
