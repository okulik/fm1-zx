// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The update package, from synthetic inputs (the SDK files too, so no network is needed): read
//! back with fm1-emu's reader, the identity, the slot limit, the input cache and `verify`.

use fmzx_pack::fwsc::{self, Sdk};
use fmzx_pack::inputs::{load_sdk, sha256_hex, LoaderPin, Pins, SdkFile, SdkPins, SDK_FILES};
use fmzx_pack::layout::{pack_offset, slot_contents, APP_OFFSET, APP_SLOT};
use fmzx_pack::lz4blk;
use fmzx_pack::package::{build, unwrap_loader, verify, PRODUCT};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Deterministic bytes (an LCG's bits 16-23).
fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (x >> 16) as u8
        })
        .collect()
}

fn sdk() -> Sdk {
    Sdk { uboot: noise(0x2000, 1), cfg_tool: noise(0x300, 2), eq_cfg: noise(0x100, 3) }
}

/// An image that carries the identity.
fn image(n: usize) -> Vec<u8> {
    let mut a = noise(n, 4);
    a[100..109].copy_from_slice(b"FM-1_948\0");
    a
}

/// A loader with Felucca's marker that compresses (two LZ4 blocks).
fn loader() -> Vec<u8> {
    let mut l = noise(2, 6);
    l.extend_from_slice(b"FELUCCA-LOADER-1");
    l.extend((0..6000u32).map(|i| (i / 7 % 251) as u8));
    l
}

/// A real game pack: our border test program, through `build_pack` (once: the tests run in
/// parallel).
fn pack() -> Vec<u8> {
    static PACK: OnceLock<Vec<u8>> = OnceLock::new();
    PACK.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fwsc-pack");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("border.sna"), fmzx_pack::testprogs::border_sna()).unwrap();
        std::fs::write(dir.join("games.toml"), "[[game]]\nid = \"border\"\nname = \"Border\"\nfile = \"border.sna\"\n")
            .unwrap();
        fmzx_pack::pack::build_pack(&dir.join("games.toml")).unwrap()
    })
    .clone()
}

/// Pins that match the synthetic SDK files and loader.
fn pins() -> Pins {
    let s = sdk();
    let files = [&s.uboot, &s.cfg_tool, &s.eq_cfg];
    Pins {
        sdk: SdkPins {
            base: "https://example.invalid/".into(),
            file: SDK_FILES.iter().zip(files).map(|(p, d)| SdkFile { path: p.to_string(), sha256: sha256_hex(d) }).collect(),
        },
        loader: LoaderPin { size: loader().len(), sha256: sha256_hex(&loader()) },
    }
}

fn package() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (img, pk) = (image(10_000), pack());
    let pkg = build(&img, &pk, &loader(), PRODUCT, &sdk()).expect("build the package");
    (pkg, img, pk)
}

#[test]
fn roundtrip_with_fm1_soc() {
    let (pkg, img, pk) = package();
    let u = fm1_soc::fwsc::unpack(&pkg).expect("fm1-emu unpacks the package");
    assert_eq!(u.area, 0x4000);
    assert_eq!(u.key, fwsc::KEY);
    let app = u.entry("app.bin").expect("app.bin in the app area");
    assert_eq!((app.offset, app.size), (APP_OFFSET as usize, APP_SLOT as usize));
    assert_eq!(&u.xip[0x120..0x120 + img.len()], &img[..], "the image at 0x120");
    let at = pack_offset(img.len());
    assert_eq!(at % 4096, 0);
    assert!(u.xip[0x120 + img.len()..at].iter().all(|&b| b == 0xFF), "0xFF between image and pack");
    assert_eq!(&u.xip[at..at + pk.len()], &pk[..], "the pack at the 4 KiB boundary");
    assert!(u.xip[at + pk.len()..0x120 + APP_SLOT as usize].iter().all(|&b| b == 0xFF), "0xFF to the slot's end");
    assert_eq!(u.raw.len(), fwsc::FLASH_SIZE);
    // the region descriptors the stock layout has (USR: the user bank)
    for (name, offset) in [("PRCT", 0), ("BTIF", 0xE9000), ("USR", 0xEA000)] {
        assert_eq!(u.entry(name).map(|h| h.offset), Some(offset), "{name}");
    }
    // fm1-emu's boot path for packages starts the image at 0x02000120
    let st = fm1_soc::stock_from_package(&pkg, None).expect("stock_from_package");
    assert_eq!(st.entry, 0x0200_0120);
    assert_eq!(st.board, fm1_soc::Board::Fm1);
    // and the loader comes back out of ota.bin
    let ota = lz4blk::wrap_loader(&loader()).unwrap();
    assert!(pkg.windows(ota.len()).any(|w| w == &ota[..]), "ota.bin in the container");
    assert_eq!(unwrap_loader(&ota).unwrap(), loader());
}

#[test]
fn identity_in_markers() {
    let (pkg, _, _) = package();
    assert_eq!(fm1_soc::fwsc::container(&pkg), (20, "FM-1_948".to_string()));
    for i in 0..20 {
        let want = b"FM-1_948".get(i).map_or(0x7D, |&c| c + i as u8 + 1);
        assert_eq!(pkg[i * 0x30 + 0x2F], want, "marker {i}");
    }
    // another identity, when the image carries it
    let mut img = image(10_000);
    img[100..109].copy_from_slice(b"FM-1_123\0");
    let pkg = build(&img, &pack(), &loader(), "FM-1_123", &sdk()).unwrap();
    assert_eq!(fm1_soc::fwsc::container(&pkg).1, "FM-1_123");
    // refused: not FM-1_ and three digits, or not in the image
    assert!(build(&img, &pack(), &loader(), "FM-1_12", &sdk()).is_err());
    assert!(build(&img, &pack(), &loader(), "FM-2_123", &sdk()).is_err());
    assert!(build(&img, &pack(), &loader(), PRODUCT, &sdk()).unwrap_err().contains("identity"));
}

#[test]
fn refuses_oversize() {
    let pk = pack();
    let slot = APP_SLOT as usize;
    // the largest image whose pack still fits ends the pack exactly at the slot's end
    let fits = (slot + 0x120 - pk.len()) / 4096 * 4096 - 0x120;
    assert!(build(&image(fits), &pk, &loader(), PRODUCT, &sdk()).is_ok());
    let err = build(&image(fits + 1), &pk, &loader(), PRODUCT, &sdk()).unwrap_err();
    assert!(err.contains("app slot"), "{err}");
    assert!(slot_contents(&image(slot), b"FMZXPACK").is_err());
    assert!(fwsc::flash_image(&vec![0; slot + 1], fwsc::KEY, &sdk()).is_err());
}

fn cache_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cfg")).unwrap();
    dir
}

fn fill_cache(dir: &Path, s: &Sdk) {
    std::fs::write(dir.join("uboot.boot"), &s.uboot).unwrap();
    std::fs::write(dir.join("cfg_tool.bin"), &s.cfg_tool).unwrap();
    std::fs::write(dir.join("cfg/eq_cfg_hw.bin"), &s.eq_cfg).unwrap();
}

#[test]
fn refuses_wrong_sha() {
    let dir = cache_dir("fwsc-cache-good");
    fill_cache(&dir, &sdk());
    let got = load_sdk(&pins(), &dir, true).expect("the cache matches the pins");
    assert_eq!(got.uboot, sdk().uboot);
    // one cached file with a wrong hash: an error, and the file is left alone
    let mut bad = sdk();
    bad.cfg_tool[7] ^= 1;
    let dir = cache_dir("fwsc-cache-bad");
    fill_cache(&dir, &bad);
    let err = load_sdk(&pins(), &dir, true).err().expect("a wrong SHA-256 is refused");
    assert!(err.contains("cfg_tool.bin") && err.contains("SHA-256"), "{err}");
    assert_eq!(std::fs::read(dir.join("cfg_tool.bin")).unwrap(), bad.cfg_tool);
    // the real pins refuse the synthetic files too (also when online: cached files are not refetched)
    let err = load_sdk(&Pins::builtin(), &dir, false).err().expect("synthetic files are not the SDK's");
    assert!(err.contains("uboot.boot"), "{err}");
    // missing and offline: an error that says how to fetch
    let dir = cache_dir("fwsc-cache-empty");
    let err = load_sdk(&pins(), &dir, true).err().expect("nothing cached");
    assert!(err.contains("--offline"), "{err}");
}

fn problems(pkg: &[u8], product: Option<&str>) -> Vec<String> {
    verify(pkg, &pins(), product).problems
}

fn assert_flags(pkg: &[u8], what: &str) {
    let p = problems(pkg, None);
    assert!(p.iter().any(|s| s.contains(what)), "expected a problem with {what:?}, got {p:?}");
}

/// A package from parts, bypassing `build`'s checks.
fn raw_package(slot: &[u8], ota: &[u8], product: &str) -> Vec<u8> {
    fwsc::ufw(&fwsc::flash_image(slot, fwsc::KEY, &sdk()).unwrap(), ota, product).unwrap()
}

#[test]
fn verify_flags_problems() {
    let (pkg, img, pk) = package();
    let r = verify(&pkg, &pins(), Some(PRODUCT));
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.identity, PRODUCT);
    assert_eq!(r.image_len, img.len());
    assert_eq!(r.pack, Some((pack_offset(img.len()), 1)));
    assert_eq!(r.loader_sha256, sha256_hex(&loader()));

    // the identity asked for
    assert!(problems(&pkg, Some("FM-1_123")).iter().any(|s| s.contains("identity FM-1_948, expected FM-1_123")));
    // the SDK files and the loader are not the pinned ones
    let p = verify(&pkg, &Pins::builtin(), None).problems;
    for f in ["uboot.boot", "cfg_tool.bin", "eq_cfg_hw.bin", "not the pinned one"] {
        assert!(p.iter().any(|s| s.contains(f)), "{f}: {p:?}");
    }
    let ota = lz4blk::wrap_loader(&loader()).unwrap();
    let slot = slot_contents(&img, &pk).unwrap();
    // an image without the identity
    let mut other = slot.clone();
    other[100..109].copy_from_slice(b"FM-1_000\0");
    assert_flags(&raw_package(&other, &ota, PRODUCT), "does not carry the identity FM-1_948");
    // a loader without its marker
    let mut l = loader();
    l[2] = b'X';
    assert_flags(&raw_package(&slot, &lz4blk::wrap_loader(&l).unwrap(), PRODUCT), "FELUCCA-LOADER-1");
    // the pack one 4 KiB boundary too far
    let mut late = img.clone();
    late.resize(pack_offset(img.len()) + 4096 - 0x120, 0xFF);
    late.extend_from_slice(&pk);
    assert_flags(&raw_package(&late, &ota, PRODUCT), "the image (10000 bytes) puts it at");
    // a pack that does not open, and no pack at all
    let mut bad = slot.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x55;
    assert_flags(&raw_package(&bad, &ota, PRODUCT), "does not open");
    assert_flags(&raw_package(&img, &ota, PRODUCT), "no game pack");
    // a byte changed in the container: the CRCs
    let mut flip = pkg.clone();
    let mid = flip.len() / 2;
    flip[mid] ^= 1;
    assert_flags(&flip, "CRC");
    let mut flip = pkg.clone();
    flip[0x10] ^= 1;
    assert_flags(&flip, "UFW header");
    // a damaged ota.bin
    let at = pkg.windows(ota.len()).position(|w| w == &ota[..]).unwrap();
    let mut flip = pkg.clone();
    flip[at + 0x50] ^= 1;
    assert_flags(&flip, "ota.bin");
    // truncated
    assert_flags(&pkg[..pkg.len() - 100], "past the end");
    assert_flags(&pkg[..500], "too short");
}

/// The package from fixed synthetic inputs. The same inputs through Felucca 1.6.1's own
/// tools/fm1pkg_make.py (and build.py's ldr_wrap for ota.bin) give the same SHA-256.
#[test]
fn golden_package() {
    let mut pk = b"FMZXPACK".to_vec();
    pk.extend(noise(3000, 5));
    let pkg = build(&image(5000), &pk, &loader(), PRODUCT, &sdk()).unwrap();
    assert_eq!(pkg.len(), 0x400 + fwsc::FLASH_SIZE + lz4blk::wrap_loader(&loader()).unwrap().len() + 20);
    assert_eq!(sha256_hex(&pkg), "bde07cc0e2d58b6f25ed89227747c0e888efa263c2b2fe773c4fe944f1963aa4");
}

#[test]
fn verify_never_panics() {
    let (pkg, _, _) = package();
    let mut x = 7u32;
    for _ in 0..150 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let mut bad = pkg.clone();
        let at = x as usize % bad.len();
        bad[at] ^= 1 << (x >> 29);
        let _ = verify(&bad, &pins(), None);
        let _ = verify(&pkg[..at], &pins(), None);
    }
}
