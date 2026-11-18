// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The package inputs that are not built here, pinned by SHA-256 in `inputs.toml`: three files
//! from JieLi's AC79 SDK, fetched with curl into a cache directory (`build/ac79/` by default),
//! and felucca-bsp's update loader (built by the `felucca_loader` target).

use crate::fwsc::Sdk;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One pinned SDK file: its path under the SDK's `cpu/wl82/tools/` and its SHA-256.
#[derive(Clone, Debug, Deserialize)]
pub struct SdkFile {
    pub path: String,
    pub sha256: String,
}

/// The pinned SDK files and where they are fetched from.
#[derive(Clone, Debug, Deserialize)]
pub struct SdkPins {
    pub base: String,
    pub file: Vec<SdkFile>,
}

/// The pinned update loader (`loader.bin`).
#[derive(Clone, Debug, Deserialize)]
pub struct LoaderPin {
    pub size: usize,
    pub sha256: String,
}

/// Everything `inputs.toml` pins.
#[derive(Clone, Debug, Deserialize)]
pub struct Pins {
    pub sdk: SdkPins,
    pub loader: LoaderPin,
}

/// The SDK file names, in the order of [`Sdk`]'s fields.
pub const SDK_FILES: [&str; 3] = ["uboot.boot", "cfg_tool.bin", "cfg/eq_cfg_hw.bin"];

impl Pins {
    /// The pins of `inputs.toml` (compiled in).
    pub fn builtin() -> Pins {
        toml::from_str(include_str!("../inputs.toml")).expect("inputs.toml parses")
    }

    /// The pinned SHA-256 of SDK file `path`.
    pub fn sdk_sha256(&self, path: &str) -> Option<&str> {
        self.sdk.file.iter().find(|f| f.path == path).map(|f| f.sha256.as_str())
    }
}

/// The default cache: `build/ac79/` in the repository.
pub fn default_cache() -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).expect("host/crates/fmzx-pack is in the repository");
    repo.join("build/ac79")
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn fetch(url: &str, to: &Path) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let status = Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "2", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e} (is curl installed?)"))?;
    if !status.success() {
        let _ = std::fs::remove_file(to);
        return Err(format!("curl could not fetch {url} ({status})"));
    }
    Ok(())
}

/// One SDK file from the cache, fetched first when it is missing (unless `offline`). A file whose
/// SHA-256 does not match is an error, in the cache or freshly fetched; a cached one is never
/// replaced on its own.
fn sdk_file(pins: &SdkPins, f: &SdkFile, cache: &Path, offline: bool) -> Result<Vec<u8>, String> {
    let path = cache.join(&f.path);
    if !path.exists() {
        if offline {
            return Err(format!(
                "{} is missing and --offline is set: run once without --offline to fetch it from {}{}",
                path.display(),
                pins.base,
                f.path
            ));
        }
        let part = PathBuf::from(format!("{}.part", path.display()));
        fetch(&format!("{}{}", pins.base, f.path), &part)?;
        let got = sha256_hex(&std::fs::read(&part).map_err(|e| format!("{}: {e}", part.display()))?);
        if got != f.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{}{}: SHA-256 {got}, expected {} (not kept)", pins.base, f.path, f.sha256));
        }
        std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let got = sha256_hex(&data);
    if got != f.sha256 {
        return Err(format!(
            "{}: SHA-256 {got}, expected {} (inputs.toml); delete it to fetch it again",
            path.display(),
            f.sha256
        ));
    }
    Ok(data)
}

/// The three SDK files, from `cache` (fetching the missing ones unless `offline`).
pub fn load_sdk(pins: &Pins, cache: &Path, offline: bool) -> Result<Sdk, String> {
    let mut files = Vec::new();
    for name in SDK_FILES {
        let f = pins.sdk.file.iter().find(|f| f.path == name).ok_or(format!("inputs.toml pins no {name}"))?;
        files.push(sdk_file(&pins.sdk, f, cache, offline)?);
    }
    let mut it = files.into_iter();
    let mut next = || it.next().expect("three files");
    Ok(Sdk { uboot: next(), cfg_tool: next(), eq_cfg: next() })
}

/// Checks that `loader` is the pinned loader.
pub fn check_loader(pins: &Pins, loader: &[u8]) -> Result<(), String> {
    let got = sha256_hex(loader);
    if loader.len() != pins.loader.size || got != pins.loader.sha256 {
        return Err(format!(
            "the loader is {} bytes, SHA-256 {got}; expected the pinned felucca-bsp loader, {} bytes, {} \
             (platform/fm1/VENDORED.md)",
            loader.len(),
            pins.loader.size,
            pins.loader.sha256
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_pins_parse() {
        let p = Pins::builtin();
        assert!(p.sdk.base.starts_with("https://"));
        for f in SDK_FILES {
            assert_eq!(p.sdk_sha256(f).map(str::len), Some(64), "{f}");
        }
        assert_eq!(p.loader.size, 15060);
    }

    #[test]
    fn sha256_of_abc() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
