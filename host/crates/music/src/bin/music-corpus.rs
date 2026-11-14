// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `music-corpus <folder> [--seconds N] [--wav] [-o build/music-corpus]`: runs every `.tap`,
//! `.tzx`, `.sna` and `.z80` of a folder and reports how much of its sound plays as FM. Reads
//! the folder, writes only under the output directory. See BUILDING.md, "Music corpus".

use music::corpus::{report_txt, run_folder, Options};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: music-corpus <folder> [--seconds N] [--wav] [-o <output dir, default <repo>/build/music-corpus>]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut opts = Options::default();
    let mut folder = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => match args.next().and_then(|v| v.parse().ok()) {
                Some(n) if n > 0 => opts.seconds = n,
                _ => return usage(),
            },
            "--wav" => opts.wav = true,
            "-o" => match args.next() {
                Some(p) => opts.out = PathBuf::from(p),
                None => return usage(),
            },
            _ if a.starts_with('-') || folder.is_some() => return usage(),
            _ => folder = Some(PathBuf::from(a)),
        }
    }
    let Some(folder) = folder else { return usage() };
    match run_folder(&folder, &opts) {
        Ok(rows) => {
            print!("{}", report_txt(&rows, &opts));
            println!("\nreport.txt and report.csv are in {}", opts.out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("music-corpus: {e}");
            ExitCode::FAILURE
        }
    }
}
