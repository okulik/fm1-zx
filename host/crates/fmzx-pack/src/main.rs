// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `fmzx-pack games <manifest.toml> -o <pack.bin>`
//! `fmzx-pack testprogs <dir>` (writes `scroll.sna` and `border.sna`)

use fmzx_pack::pack::build_pack;
use fmzx_pack::testprogs::write_all;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: fmzx-pack games <manifest.toml> -o <pack.bin>\n       fmzx-pack testprogs <dir>";

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
