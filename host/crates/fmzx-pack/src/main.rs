// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `fmzx-pack games <manifest.toml> -o <pack.bin>`
//! `fmzx-pack testprogs <dir>` (writes `scroll.sna` and `border.sna`)
//! `fmzx-pack fwsc --app <app.bin> --pack <pack.bin> --loader <loader.bin> -o <out.fwsc>
//!     [--product FM-1_948] [--cache DIR] [--offline]`
//! `fmzx-pack verify <pkg.fwsc> [--product FM-1_948]`

use fmzx_pack::inputs::{check_loader, default_cache, load_sdk, sha256_hex, Pins};
use fmzx_pack::pack::build_pack;
use fmzx_pack::package::{self, PRODUCT};
use fmzx_pack::testprogs::write_all;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: fmzx-pack games <manifest.toml> -o <pack.bin>
       fmzx-pack testprogs <dir>
       fmzx-pack fwsc --app <app.bin> --pack <pack.bin> --loader <loader.bin> -o <out.fwsc>
                      [--product FM-1_948] [--cache DIR] [--offline]
       fmzx-pack verify <pkg.fwsc> [--product FM-1_948]";

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// `--flag value` and `--flag` options after a command.
struct Opts {
    values: Vec<(String, String)>,
    flags: Vec<String>,
    free: Vec<String>,
}

fn opts(args: &[String], with_value: &[&str], flags: &[&str]) -> Result<Opts, String> {
    let mut o = Opts { values: Vec::new(), flags: Vec::new(), free: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if with_value.contains(&a.as_str()) {
            let v = it.next().ok_or(format!("{a} needs a value\n{USAGE}"))?;
            o.values.push((a.clone(), v.clone()));
        } else if flags.contains(&a.as_str()) {
            o.flags.push(a.clone());
        } else if a.starts_with('-') {
            return Err(format!("unknown option {a}\n{USAGE}"));
        } else {
            o.free.push(a.clone());
        }
    }
    Ok(o)
}

impl Opts {
    fn get(&self, k: &str) -> Option<&str> {
        self.values.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    fn need(&self, k: &str) -> Result<&str, String> {
        self.get(k).ok_or(format!("{k} is required\n{USAGE}"))
    }
}

fn fwsc(args: &[String]) -> Result<(), String> {
    let o = opts(args, &["--app", "--pack", "--loader", "-o", "--product", "--cache"], &["--offline"])?;
    if !o.free.is_empty() {
        return Err(USAGE.to_string());
    }
    let pins = Pins::builtin();
    let product = o.get("--product").unwrap_or(PRODUCT);
    let (image, pack, loader) = (read(o.need("--app")?)?, read(o.need("--pack")?)?, read(o.need("--loader")?)?);
    check_loader(&pins, &loader)?;
    let cache = o.get("--cache").map_or_else(default_cache, PathBuf::from);
    let sdk = load_sdk(&pins, &cache, o.flags.iter().any(|f| f == "--offline"))?;
    let pkg = package::build(&image, &pack, &loader, product, &sdk)?;
    let report = package::verify(&pkg, &pins, Some(product));
    if !report.problems.is_empty() {
        return Err(format!("the package does not verify:\n  {}", report.problems.join("\n  ")));
    }
    let out = o.need("-o")?;
    std::fs::write(out, &pkg).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "{out}: {} bytes, identity {product}, image {} bytes, pack {} bytes at app area offset {:#x}, SHA-256 {}",
        pkg.len(),
        image.len(),
        pack.len(),
        fmzx_pack::layout::pack_offset(image.len()),
        sha256_hex(&pkg)
    );
    Ok(())
}

fn verify(args: &[String]) -> Result<(), String> {
    let o = opts(args, &["--product"], &[])?;
    let [path] = &o.free[..] else { return Err(USAGE.to_string()) };
    let pkg = read(path)?;
    let r = package::verify(&pkg, &Pins::builtin(), o.get("--product"));
    println!("{path}: {} bytes, SHA-256 {}", pkg.len(), sha256_hex(&pkg));
    println!("  identity {}", r.identity);
    println!("  image {} bytes", r.image_len);
    if let Some((at, n)) = r.pack {
        println!("  game pack at app area offset {at:#x}, {n} games");
    }
    println!("  loader SHA-256 {}", r.loader_sha256);
    if r.problems.is_empty() {
        println!("  OK");
        Ok(())
    } else {
        Err(format!("{path}: {} problems:\n  {}", r.problems.len(), r.problems.join("\n  ")))
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [cmd, manifest, flag, out] if cmd == "games" && flag == "-o" => {
            let pack = build_pack(&PathBuf::from(manifest))?;
            std::fs::write(out, &pack).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: {} bytes", pack.len());
            Ok(())
        }
        [cmd, dir] if cmd == "testprogs" => {
            for path in write_all(&PathBuf::from(dir))? {
                println!("{}", path.display());
            }
            Ok(())
        }
        [cmd, rest @ ..] if cmd == "fwsc" => fwsc(rest),
        [cmd, rest @ ..] if cmd == "verify" => verify(rest),
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fmzx-pack: {e}");
            ExitCode::FAILURE
        }
    }
}
