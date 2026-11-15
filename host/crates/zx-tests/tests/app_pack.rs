// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_sys::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn random(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.next() as u8).collect()
}

fn repetitive(n: usize) -> Vec<u8> {
    (0..n).map(|i| b"ABCABCABD\0\0\0\0"[(i / 3) % 13]).collect()
}

#[test]
fn lz4_roundtrip() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for n in [0usize, 1, 15, 4096, 131_072] {
        for data in [random(&mut rng, n), repetitive(n), vec![0u8; n]] {
            let block = lz4_flex::block::compress(&data);
            assert_eq!(lz4_decode(&block, data.len()).unwrap(), data, "n={n}");
            // a larger cap is fine too
            assert_eq!(lz4_decode(&block, data.len() + 7).unwrap(), data);
        }
    }
}

#[test]
fn lz4_known_good() {
    // 1 literal 'a', match offset 1 length 4+0: "aaaaa"; a block that ends with a match
    // (no trailing literals) is non-canonical, but the decoder tolerates it, memory-safely.
    assert_eq!(lz4_decode(&[0x10, b'a', 1, 0], 16).unwrap(), b"aaaaa");
}

#[test]
fn lz4_tolerates_non_canonical_ending() {
    assert_eq!(lz4_decode(&[0x10, b'a', 1, 0], 5).unwrap(), b"aaaaa");
}

#[test]
fn lz4_rejects_huge_cap() {
    assert_eq!(lz4_decode_raw_cap(&[0x10, b'a', 1, 0], 0x8000_0000), Err(ZX_ERR_FORMAT));
}

#[test]
fn lz4_rejects_bad() {
    let bad: &[(&str, &[u8], usize)] = &[
        ("empty", &[], 16),
        ("offset 0", &[0x10, b'a', 0, 0], 16),
        ("offset before start", &[0x10, b'a', 2, 0], 16),
        ("truncated offset", &[0x10, b'a', 1], 16),
        ("literals past input", &[0x50, 1, 2], 16),
        ("literal length ext past input", &[0xF0, 255], 1000),
        ("literals past cap", &[0x50, 1, 2, 3, 4, 5], 4),
        ("match past cap", &[0x1F, b'a', 1, 0, 200], 10),
        ("match length ext past input", &[0x1F, b'a', 1, 0, 255], 1000),
        ("huge literal length", &[0xF0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 1], 100),
    ];
    for (name, block, cap) in bad {
        assert_eq!(lz4_decode(block, *cap), Err(ZX_ERR_FORMAT), "{name}");
    }
}

#[test]
fn lz4_fuzz_mutations() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let mut data = repetitive(3000);
    data.extend(random(&mut rng, 500));
    data.extend(repetitive(2000));
    let good = lz4_flex::block::compress(&data);
    assert_eq!(lz4_decode(&good, data.len()).unwrap(), data);
    let mut ok = 0;
    for i in 0..10_000 {
        let mut b = good.clone();
        for _ in 0..1 + rng.next() % 4 {
            let p = rng.next() as usize % b.len();
            match rng.next() % 3 {
                0 => b[p] = rng.next() as u8,
                1 => b[p] ^= 1 << (rng.next() % 8),
                _ => b.truncate(p.max(1)),
            }
        }
        let cap = match i % 3 {
            0 => data.len(),
            1 => rng.next() as usize % (data.len() + 1),
            _ => 70_000,
        };
        if lz4_decode(&b, cap).is_ok() {
            ok += 1;
        }
    }
    // sanity: the mutations are neither all rejected nor all harmless
    assert!(ok > 0 && ok < 10_000, "{ok} of 10000 mutations decoded");
    eprintln!("lz4 fuzz: {ok} of 10000 mutations still decoded");
}

#[test]
fn crc32_check_value() {
    assert_eq!(pack_crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(pack_crc32(b""), 0);
}

// ---- a pack builder independent of the packer ----

struct Spec {
    name: &'static str,
    kind: u8,
    codec: u8,
    crop: u8,
    raw: Vec<u8>,
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

fn fix_crc(p: &mut [u8]) {
    let total = u32::from_le_bytes(p[12..16].try_into().unwrap()) as usize;
    let end = total.min(p.len()).max(32);
    let c = pack_crc32(&p[32..end]);
    put32(p, 16, c);
}

fn build(specs: &[Spec]) -> Vec<u8> {
    let mut p = vec![0u8; 32 + 160 * specs.len()];
    p[..8].copy_from_slice(b"FMZXPACK");
    p[8..10].copy_from_slice(&2u16.to_le_bytes());
    p[10..12].copy_from_slice(&(specs.len() as u16).to_le_bytes());
    for (i, s) in specs.iter().enumerate() {
        let e = 32 + 160 * i;
        let stored = if s.codec == 1 { lz4_flex::block::compress(&s.raw) } else { s.raw.clone() };
        let off = p.len();
        p.extend_from_slice(&stored);
        while p.len() % 4 != 0 {
            p.push(0);
        }
        p[e..e + s.name.len()].copy_from_slice(s.name.as_bytes());
        p[e + 29] = s.kind;
        p[e + 30] = s.codec;
        p[e + 31] = s.crop;
        put32(&mut p, e + 32, off as u32);
        put32(&mut p, e + 36, stored.len() as u32);
        put32(&mut p, e + 40, s.raw.len() as u32);
        put32(&mut p, e + 44, pack_crc32(&s.raw));
        for k in 0..55 {
            p[e + 48 + 2 * k] = k as u8 % 40;
            p[e + 49 + 2 * k] = 0xFF;
        }
    }
    let total = p.len() as u32;
    put32(&mut p, 12, total);
    fix_crc(&mut p);
    p
}

fn two() -> (Vec<Spec>, Vec<u8>) {
    let mut rng = Rng(77);
    let specs = vec![
        Spec { name: "Alpha", kind: 1, codec: 1, crop: 8, raw: repetitive(5000) },
        Spec { name: "Beta", kind: 3, codec: 0, crop: 16, raw: random(&mut rng, 333) },
    ];
    let p = build(&specs);
    (specs, p)
}

#[test]
fn pack_open_ok() {
    let (specs, p) = two();
    assert_eq!(pack_open(&p), Ok(2));
    for (i, s) in specs.iter().enumerate() {
        let g = pack_game_at(&p, i as u32).unwrap();
        assert_eq!(g.name(), s.name);
        assert_eq!((g.kind(), g.codec(), g.crop_x()), (s.kind, s.codec, s.crop));
        assert_eq!(g.raw_len() as usize, s.raw.len());
        assert_eq!(g.keys()[3], [3, 0xFF]);
        assert_eq!(g.extract(s.raw.len()).unwrap(), s.raw);
    }
    // extra room after the pack is fine
    let mut q = p.clone();
    q.extend([0u8; 100]);
    assert_eq!(pack_open(&q), Ok(2));
    // an index past the count is bad
    assert!(pack_game_at(&p, 2).is_err());
}

#[test]
fn pack_bad_magic() {
    let (_, mut p) = two();
    p[0] = b'X';
    fix_crc(&mut p);
    assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT));
}

#[test]
fn pack_bad_version() {
    let (_, mut p) = two();
    for v in [1u8, 3] {
        p[8] = v;
        fix_crc(&mut p);
        assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT), "version {v}");
    }
}

#[test]
fn pack_bad_crc() {
    let (_, mut p) = two();
    let n = p.len();
    p[n - 5] ^= 1;
    assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT));
}

#[test]
fn pack_total_len_exceeds_avail() {
    let (_, p) = two();
    assert_eq!(pack_open(&p[..p.len() - 1]), Err(ZX_ERR_FORMAT));
    assert_eq!(pack_open(&p[..10]), Err(ZX_ERR_FORMAT));
    assert_eq!(pack_open(&[]), Err(ZX_ERR_FORMAT));
}

#[test]
fn pack_entry_table_past_total() {
    let (_, mut p) = two();
    p[10] = 50; // 50 entries do not fit
    fix_crc(&mut p);
    assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT));
}

fn entry_case(edit: impl Fn(&mut Vec<u8>)) {
    let (_, mut p) = two();
    edit(&mut p);
    fix_crc(&mut p);
    assert_eq!(pack_open(&p), Ok(2), "the header stays valid");
    assert_eq!(pack_game_at(&p, 0).err(), Some(ZX_ERR_FORMAT));
}

#[test]
fn entry_out_of_bounds() {
    entry_case(|p| {
        let len = p.len() as u32;
        put32(p, 32 + 36, len); // stored_len reaches past total_len
    });
    entry_case(|p| put32(p, 32 + 32, 0xFFFF_FFF0)); // offset + stored_len overflows 32 bits
    entry_case(|p| put32(p, 32 + 32, 32)); // data inside the header/entry table
    entry_case(|p| {
        let off = u32::from_le_bytes(p[64..68].try_into().unwrap());
        put32(p, 32 + 32, off + 1); // unaligned
    });
}

#[test]
fn raw_len_too_big() {
    entry_case(|p| put32(p, 32 + 40, PACK_MAX_RAW + 1));
}

#[test]
fn stored_len_mismatch() {
    // a stored game whose stored_len differs from raw_len
    let (_, mut p) = two();
    let e = 32 + 160;
    let raw = u32::from_le_bytes(p[e + 40..e + 44].try_into().unwrap());
    put32(&mut p, e + 36, raw - 1);
    fix_crc(&mut p);
    assert_eq!(pack_game_at(&p, 1).err(), Some(ZX_ERR_FORMAT));
}

#[test]
fn crop_over_16() {
    entry_case(|p| p[32 + 31] = 17);
}

#[test]
fn bad_kind_codec() {
    entry_case(|p| p[32 + 29] = 0);
    entry_case(|p| p[32 + 29] = 4);
    entry_case(|p| p[32 + 30] = 2);
}

#[test]
fn name_unterminated() {
    entry_case(|p| p[32..32 + 29].fill(b'A'));
}

#[test]
fn name_not_printable() {
    entry_case(|p| p[32 + 2] = 0x07);
    entry_case(|p| p[32 + 2] = 0xC3);
}

#[test]
fn extract_crc_mismatch() {
    let (specs, mut p) = two();
    put32(&mut p, 32 + 44, 0x1234_5678);
    fix_crc(&mut p);
    let g = pack_game_at(&p, 0).unwrap();
    assert_eq!(g.extract(specs[0].raw.len()), Err(ZX_ERR_FORMAT));
}

#[test]
fn extract_cap_too_small() {
    let (specs, p) = two();
    let g = pack_game_at(&p, 0).unwrap();
    assert_eq!(g.extract(specs[0].raw.len() - 1), Err(ZX_ERR_FORMAT));
}

#[test]
fn extract_lz4_length_mismatch() {
    // raw_len says more than the block decodes to
    let (specs, mut p) = two();
    put32(&mut p, 32 + 40, specs[0].raw.len() as u32 + 1);
    fix_crc(&mut p);
    let g = pack_game_at(&p, 0).unwrap();
    assert_eq!(g.extract(specs[0].raw.len() + 1), Err(ZX_ERR_FORMAT));
}

#[test]
fn entry_stored_len_near_u32_max() {
    // a valid in-table offset with stored_len = 0xFFFFFFFC: `stored > total - offset`
    entry_case(|p| put32(p, 32 + 36, 0xFFFF_FFFC));
}

#[test]
fn header_needs_games_and_alignment() {
    let (_, mut p) = two();
    p[10] = 0;
    p[11] = 0;
    fix_crc(&mut p);
    assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT), "count 0");
    let (_, mut p) = two();
    let n = p.len();
    p.push(0);
    put32(&mut p, 12, n as u32 + 1);
    fix_crc(&mut p);
    assert_eq!(pack_open(&p), Err(ZX_ERR_FORMAT), "total_len not a multiple of 4");
}

#[test]
fn pack_fuzz_mutations() {
    let mut rng = Rng(0x0bad_cafe_1234_5678);
    let mut raws = vec![repetitive(6000), random(&mut rng, 700), vec![0x55; 9000]];
    raws.push(Vec::new());
    let specs: Vec<Spec> = raws
        .into_iter()
        .zip([1u8, 3, 2, 1])
        .enumerate()
        .map(|(i, (raw, kind))| Spec { name: ["One", "Two", "Three", "Four"][i], kind, codec: (i % 2) as u8, crop: 8, raw })
        .collect();
    let good = build(&specs[..3]);
    assert_eq!(pack_open(&good), Ok(3));
    let (mut opened, mut extracted) = (0, 0);
    for i in 0..5000 {
        let mut p = good.clone();
        for _ in 0..1 + rng.next() % 4 {
            let at = rng.next() as usize % p.len();
            match rng.next() % 4 {
                0 => p[at] ^= 1 << (rng.next() % 8),
                1 => p[at] = rng.next() as u8,
                2 => p.truncate(at.max(1)),
                _ => {
                    // length-field edits: header total/count, entry offset/stored/raw
                    let f = [10usize, 12, 32 + 32, 32 + 36, 32 + 40, 32 + 160 + 32, 32 + 160 + 36, 32 + 160 + 40][rng.next() as usize % 8];
                    if f + 4 <= p.len() {
                        let v = [0u32, 1, 4, 0xFFFF_FFFC, 0xFFFF_FFFF, rng.next(), p.len() as u32][rng.next() as usize % 7];
                        put32(&mut p, f, v);
                    }
                }
            }
        }
        if i % 2 == 0 && p.len() >= 32 {
            fix_crc(&mut p);
        }
        // an exact-size allocation, so ASan sees any read past the (possibly truncated) end
        let p: Box<[u8]> = p.into_boxed_slice();
        let Ok(count) = pack_open(&p) else { continue };
        opened += 1;
        for idx in 0..count {
            let Ok(g) = pack_game_at(&p, idx) else { continue };
            if let Ok(out) = g.extract(PACK_MAX_RAW as usize) {
                assert_eq!(out.len(), g.raw_len() as usize);
                extracted += 1;
            }
        }
    }
    assert!(opened > 100 && extracted > 100, "opened {opened}, extracted {extracted}");
    eprintln!("pack fuzz: {opened} of 5000 mutated packs opened, {extracted} games extracted");
}
