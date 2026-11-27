// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `music-preview <W/all> --game ID [--score KEY [--take T]] [--seconds N] [--beeper] -o OUT.wav`:
//! what the FM-1 plays for a game of a personal pack's learning workspace, through `core/sound`,
//! in FM: engines with the game's list as the pack has it, or with score KEY playing take T (`0`
//! or `original`, `<n>`, `engine-<n>`, `chip-<n>`, `ay-<n>`); `--beeper` the beeper alone over the same frames. One line
//! on stdout when written; one line on stderr and exit 1 when not (an unknown workspace, game,
//! score or take; 3 when the take is refused: `refused: <why>`), 2 for a usage error. See BUILDING.md, "Arranged music", and
//! [`music::preview`].

use music::preview::{preview, rate, Error, Request, Take, MAX_SECONDS};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage(why: &str) -> ExitCode {
    eprintln!("music-preview: {why}");
    eprintln!("usage: music-preview <W/all> --game ID [--score KEY [--take 0|original|<n>|engine-<n>|chip-<n>|ay-<n>]] [--seconds N] [--beeper] -o OUT.wav");
    ExitCode::from(2)
}

/// One line: a message's lines joined.
fn one_line(s: &str) -> String {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ")
}

fn main() -> ExitCode {
    let (mut all, mut game, mut score, mut take, mut seconds, mut beeper, mut out) = (None, None, None, None, None, false, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let got = match a.as_str() {
            "--game" => value("--game").map(|v| game = Some(v)),
            "--score" => value("--score").map(|v| score = Some(v)),
            "--take" => value("--take").map(|v| take = Some(v)),
            "--seconds" => value("--seconds").and_then(|v| match v.parse::<f64>() {
                Ok(n) if n > 0.0 && n <= MAX_SECONDS => {
                    seconds = Some(n);
                    Ok(())
                }
                _ => Err(format!("--seconds {v}: a number of seconds above 0, at most {MAX_SECONDS}")),
            }),
            "--beeper" => {
                beeper = true;
                Ok(())
            }
            "-o" => value("-o").map(|v| out = Some(PathBuf::from(v))),
            _ if a.starts_with('-') => Err(format!("unknown option {a}")),
            _ if all.is_some() => Err(format!("one workspace only ({a}?)")),
            _ => {
                all = Some(PathBuf::from(a));
                Ok(())
            }
        };
        if let Err(e) = got {
            return usage(&e);
        }
    }
    let (Some(all), Some(game), Some(out)) = (all, game, out) else { return usage("give the workspace, --game and -o") };
    if take.is_some() && score.is_none() {
        return usage("--take needs --score");
    }
    let take = match take.as_deref().map(|t| (t, Take::parse(t))) {
        None => Take::Chosen,
        Some((_, Some(t))) => t,
        Some((t, None)) => {
            eprintln!("unknown take {t:?}: a take is 0 or original, <n> (take-<n>), engine-<n>, chip-<n> or ay-<n>");
            return ExitCode::FAILURE;
        }
    };
    let req = Request { game, score: score.map(|k| (k, take)), seconds, beeper };
    let p = match preview(&all, &req) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", one_line(&e.to_string()));
            return ExitCode::from(if matches!(e, Error::Refused(_)) { 3 } else { 1 });
        }
    };
    if let Err(e) = write(&out, &p.samples) {
        eprintln!("{}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    let what = match (&req.score, &p.plays) {
        (Some((key, _)), Some(plays)) => format!("score {key} playing {plays}"),
        _ => "its music".into(),
    };
    let sound = if req.beeper { "the beeper" } else { "FM: engines" };
    println!(
        "{}: {:.1} s of {} ({what}, {sound}), from {:.1} s into its run{}",
        out.display(),
        p.samples.len() as f64 / f64::from(rate()),
        req.game,
        p.from,
        if p.cached { "" } else { "; its music learned and cached" }
    );
    ExitCode::SUCCESS
}

/// Writes the WAV whole, then renames it into place: a half-written preview is never there.
fn write(out: &Path, samples: &[i16]) -> std::io::Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = out.with_extension("wav.tmp");
    music::wav::write(&tmp, samples, rate())?;
    std::fs::rename(&tmp, out)
}
