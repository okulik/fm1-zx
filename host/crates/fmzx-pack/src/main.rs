// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `fmzx-pack games <manifest.toml> -o <pack.bin>`
//! `fmzx-pack keymaps <manifest.toml> -o <keymaps.json>`
//! `fmzx-pack testprogs <dir>` (writes `scroll.sna`, `border.sna`, `tape.tap` and `tune.sna`)
//! `fmzx-pack menu-art <png> -o <c file>` (the menu art's top band as C: `app/menu_art.c`)
//! `fmzx-pack fwsc --app <app.bin> --pack <pack.bin> --loader <loader.bin> -o <out.fwsc>
//!     [--product FM-1_048] [--cache DIR] [--offline]`
//! `fmzx-pack verify <pkg.fwsc> [--product FM-1_048]`
//! `fmzx-pack personal <folder | games.toml> -o <dir> --app <app.bin> [--app <app.bin>]... [--plan]`
//!     (the personal pack, `tools/pack-studio`; `--app`: the images it goes with, at least
//!     one, which set the room for it; `--plan`: `<dir>/plan.json` instead of the pack, which may
//!     then be over the room)
//! `fmzx-pack chip-notes <file.sid | file.ay> [--subtune N] [--seconds S] -o <notes.toml>` (a C64
//!     SID tune's or a Spectrum 128 `.ay` song's notes for `tools/arrange-music`: the `chipmusic`
//!     crate; `--subtune` (1-based) defaults to the file's start song, `--seconds` to 180)
//! `fmzx-pack chip-notes --info <file.sid | file.ay>` (the file's header)
//! `fmzx-pack engine-notes <W/all> --game ID --score KEY -o <notes.toml>` (a score's music read
//!     exactly from the game's music engine, on the score's timeline, for `tools/arrange-music
//!     engine`: [`fmzx_pack::engine_notes`]; exit 3 when no known engine plays it)
//! `fmzx-pack engine-notes --info <W/all> --game ID` (each score's engine, or why none)

use fmzx_pack::inputs::{check_loader, default_cache, load_sdk, sha256_hex, Pins};
use fmzx_pack::keymaps::keymaps_json;
use fmzx_pack::manifest::parse_manifest;
use fmzx_pack::pack::build_pack;
use fmzx_pack::package::{self, PRODUCT};
use fmzx_pack::personal;
use fmzx_pack::testprogs::write_all;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: fmzx-pack games <manifest.toml> -o <pack.bin>
       fmzx-pack keymaps <manifest.toml> -o <keymaps.json>
       fmzx-pack testprogs <dir>
       fmzx-pack menu-art <png> -o <c file>
       fmzx-pack fwsc --app <app.bin> --pack <pack.bin> --loader <loader.bin> -o <out.fwsc>
                      [--product FM-1_048] [--cache DIR] [--offline]
       fmzx-pack verify <pkg.fwsc> [--product FM-1_048]
       fmzx-pack personal <folder | games.toml> -o <dir> --app <app.bin> [--app <app.bin>]... [--plan]
       fmzx-pack chip-notes <file.sid | file.ay> [--subtune N] [--seconds S (default 180)] -o <notes.toml>
       fmzx-pack chip-notes --info <file.sid | file.ay>
       fmzx-pack engine-notes <W/all> --game ID --score KEY -o <notes.toml>
       fmzx-pack engine-notes --info <W/all> --game ID";

/// The start of the error an `engine-notes` run ends with when no known engine plays the score
/// (exit 3).
const NO_ENGINE: &str = "no engine: ";

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

fn personal(args: &[String]) -> Result<(), String> {
    let o = opts(args, &["-o", "--app"], &["--plan"])?;
    let [input] = &o.free[..] else { return Err(USAGE.to_string()) };
    let out = o.need("-o")?;
    if !o.values.iter().any(|(k, _)| k == "--app") {
        return Err(format!(
            "personal needs --app <app.bin>: the app images the pack goes with set the room for it\n{USAGE}"
        ));
    }
    // the room beside the largest image the pack goes with
    let mut limit = None;
    for (_, app) in o.values.iter().filter(|(k, _)| k == "--app") {
        let len = std::fs::metadata(app).map_err(|e| format!("{app}: {e}"))?.len();
        let room = fmzx_pack::layout::pack_room(usize::try_from(len).unwrap_or(usize::MAX));
        limit = Some(limit.map_or(room, |l: u32| l.min(room)));
    }
    let plan = o.flags.iter().any(|f| f == "--plan");
    let r = if plan { personal::run_plan } else { personal::run_with_limit }(Path::new(input), Path::new(out), limit)?;
    print!("{}", personal::table(&r));
    let games = r.rows.iter().filter(|w| w.packed.is_some()).count();
    if plan {
        let fits = if r.used > r.room { "does not fit" } else { "fits" };
        println!("{}: a pack of {games} games, {} bytes of {}: it {fits}", Path::new(out).join(personal::PLAN).display(), r.used, r.room);
    } else {
        println!("{}: {} bytes, {games} games", Path::new(out).join("games.bin").display(), r.used);
    }
    println!("{}: the learned music's details", Path::new(out).join(personal::SOUND_REPORT).display());
    Ok(())
}

fn chip_notes(args: &[String]) -> Result<(), String> {
    let o = opts(args, &["--subtune", "--seconds", "-o"], &["--info"])?;
    let [path] = &o.free[..] else { return Err(USAGE.to_string()) };
    if o.flags.iter().any(|f| f == "--info") {
        let info = chipmusic::info(&read(path)?).map_err(|e| format!("{path}: {e}"))?;
        print!("{path}: {info}");
        return Ok(());
    }
    let subtune = match o.get("--subtune") {
        Some(s) => Some(s.parse::<u16>().map_err(|_| format!("--subtune {s}: not a number\n{USAGE}"))?),
        None => None,
    };
    let seconds = match o.get("--seconds") {
        Some(s) => s
            .parse::<f64>()
            .ok()
            .filter(|&x| x > 0.0 && x <= 3600.0)
            .ok_or(format!("--seconds {s}: not a number of seconds from 0 to 3600\n{USAGE}"))?,
        None => chipmusic::DEFAULT_SECONDS,
    };
    let out = o.need("-o")?;
    let made = chipmusic::chip_notes(&read(path)?, subtune, seconds).map_err(|e| format!("{path}: cannot be played: {e}"))?;
    std::fs::write(out, made.toml(path)).map_err(|e| format!("{out}: {e}"))?;
    println!("{out}: {}", made.summary());
    Ok(())
}

fn engine_notes(args: &[String]) -> Result<(), String> {
    use fmzx_pack::engine_notes::{notes_toml, read_scores, summary};
    let o = opts(args, &["--game", "--score", "-o"], &["--info"])?;
    let [all] = &o.free[..] else { return Err(USAGE.to_string()) };
    let game = o.need("--game")?;
    if o.flags.iter().any(|f| f == "--info") {
        let got = read_scores(Path::new(all), game, o.get("--score"))?;
        if got.is_empty() {
            println!("{game}: no scores");
        }
        for se in &got {
            let shadow = if se.shadow { " (shadow score)" } else { "" };
            println!("{game}/{}{shadow}: {}", se.key, summary(se));
        }
        return Ok(());
    }
    let key = o.need("--score")?;
    let out = o.need("-o")?;
    let got = read_scores(Path::new(all), game, Some(key))?;
    let se = got.first().ok_or(format!("game {game:?} has no score {key}"))?;
    match &se.found {
        Err(why) => Err(format!("{NO_ENGINE}{game}/{key}: {why}")),
        Ok(f) => {
            std::fs::write(out, notes_toml(game, se, f)).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: {}", summary(se));
            Ok(())
        }
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
        [cmd, manifest, flag, out] if cmd == "keymaps" && flag == "-o" => {
            let path = PathBuf::from(manifest);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{manifest}: {e}"))?;
            let m = parse_manifest(&text)?;
            std::fs::write(out, keymaps_json(&m)?).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: {} games", m.game.len());
            Ok(())
        }
        [cmd, dir] if cmd == "testprogs" => {
            for path in write_all(&PathBuf::from(dir))? {
                println!("{}", path.display());
            }
            Ok(())
        }
        [cmd, png, flag, out] if cmd == "menu-art" && flag == "-o" => {
            std::fs::write(out, fmzx_pack::menu_art::generate(&read(png)?)?).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}");
            Ok(())
        }
        [cmd, rest @ ..] if cmd == "fwsc" => fwsc(rest),
        [cmd, rest @ ..] if cmd == "verify" => verify(rest),
        [cmd, rest @ ..] if cmd == "personal" => personal(rest),
        [cmd, rest @ ..] if cmd == "chip-notes" => chip_notes(rest),
        [cmd, rest @ ..] if cmd == "engine-notes" => engine_notes(rest),
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fmzx-pack: {e}");
            if e.starts_with(NO_ENGINE) {
                ExitCode::from(3)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// chip-notes caps a frame's events as the firmware plays them.
    #[test]
    fn chip_notes_frames_are_the_firmwares() {
        assert_eq!(chipmusic::notes::EVENTS_PER_FRAME, zx_sys::SND_EVENTS);
        assert_eq!(chipmusic::notes::FRAME_T, u64::from(zx_sys::ZX_FRAME_TSTATES));
    }

    #[test]
    fn chip_notes_arguments() {
        let run = |a: &[&str]| super::run(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_err();
        assert!(run(&["chip-notes"]).starts_with("usage:"));
        assert!(run(&["chip-notes", "x.sid", "--subtune", "one", "-o", "n.toml"]).starts_with("--subtune one: not a number"));
        assert!(run(&["chip-notes", "x.sid", "--seconds", "0", "-o", "n.toml"]).starts_with("--seconds 0: not a number"));
        assert!(run(&["chip-notes", "x.sid"]).starts_with("-o is required"));
        assert!(run(&["chip-notes", "/nonexistent/x.sid", "-o", "n.toml"]).starts_with("/nonexistent/x.sid: "));
        assert!(run(&["chip-notes", "--bad", "x.sid"]).starts_with("unknown option --bad"));
    }

    #[test]
    fn engine_notes_arguments() {
        let run = |a: &[&str]| super::run(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_err();
        assert!(run(&["engine-notes"]).starts_with("usage:"));
        assert!(run(&["engine-notes", "W/all"]).starts_with("--game is required"));
        assert!(run(&["engine-notes", "W/all", "--game", "g", "-o", "n.toml"]).starts_with("--score is required"));
        assert!(run(&["engine-notes", "W/all", "--game", "g", "--score", "k"]).starts_with("-o is required"));
        assert!(run(&["engine-notes", "/nonexistent/all", "--game", "g", "--score", "k", "-o", "n.toml"]).starts_with("/nonexistent/all: no such folder"));
        assert!(run(&["engine-notes", "--info", "/nonexistent/all", "--game", "g"]).starts_with("/nonexistent/all: no such folder"));
    }
}
