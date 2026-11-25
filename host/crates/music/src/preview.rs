// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the FM-1 plays for a game of a personal pack's learning workspace (`fmzx-pack personal
//! ... -o W/all`), rendered through `core/sound` as `music-corpus` renders `_fm_l.wav`: for
//! `music-preview` (BUILDING.md, "Arranged music").
//!
//! The game is found by its id in `W/all/converted/manifest.toml`, its key script and overrides
//! in `W/all/personal-games.toml`, and its music in `W/all/sound-cache/` through
//! [`learn_music`], the personal pack's own lookup (learned and cached on a miss). Its list is
//! the pack's: the takes `W/all/arrange/choices.toml` chooses put in by [`arrange::apply`], each
//! verified by replay as the pack verifies it. A preview of one score has that score play the
//! take asked for instead of its choice.
//!
//! The run is the learner's (its key script, or the run it chose) from its start; a score's is
//! the run that recorded it ([`ScoreRuns`](fmzx_pack::personal::ScoreRuns)), from the frame of the
//! edge that starts it (no earlier: what the run played before it, such as an earlier tune's FM
//! notes ringing out, would sound the same in every take) (its line in the learner's details: every take keeps its
//! trigger, so plays from there too). The FM render is FM: engines (the mixer from the run's
//! start, 3 frames late, the effects follower on, as on the device), the beeper render the beeper
//! alone over the same frames of the run. A preview of the default length is 40 s, or with a score
//! the score (its intro and one pass of its loop) and 2 s when that is shorter.

use crate::{sample_rate, RATE};
use fmzx_pack::arrange::{self, Choice, Of, Status, ARRANGE, CLOCK_HZ, TAKE_FILE};
use fmzx_pack::manifest::{game_path, parse_manifest, parse_personal_manifest};
use fmzx_pack::personal::{cache_path, learn_music, machine, verify_in_run, LearnerRun, GAMES_FILE};
use soundlearn::listen::RunKind;
use soundlearn::{decode_scores, encode_list_scores, score_key, ScoreRecord};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use zx_sys::{Beeper, SndMixer, SND_DELAY_FRAMES, SND_MODE_FM_B, ZX_FRAME_TSTATES};

/// The seconds of a preview when none are given (with a score, at most its length and 3 s).
pub const DEFAULT_SECONDS: f64 = 40.0;
/// The longest preview.
pub const MAX_SECONDS: f64 = 600.0;
/// Frames before a score's first frame in its preview.
const LEAD_FRAMES: usize = 0;
/// Seconds after a score's end in a preview of the default length.
const TAIL_SECONDS: f64 = 2.0;

/// The take a preview plays for its score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// Whatever `choices.toml` chooses.
    Chosen,
    /// The score as recorded (a shadow score: the routine, live).
    Original,
    /// `take-<n>`, `chip-<n>` or `ay-<n>`.
    Of(Choice),
}

impl Take {
    /// A take's name on the command line: `0` or `original`, `<n>` (`take-<n>`), `chip-<n>`,
    /// `ay-<n>`.
    pub fn parse(s: &str) -> Option<Take> {
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if s == "original" {
            return Some(Take::Original);
        }
        if digits(s) {
            return match s.parse::<u32>().ok()? {
                0 => Some(Take::Original),
                n => Some(Take::Of(Choice::Take(n))),
            };
        }
        Choice::own_named(s).map(Take::Of)
    }
}

/// A preview asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub game: String,
    /// A score's key and the take it plays.
    pub score: Option<(String, Take)>,
    /// None: [`DEFAULT_SECONDS`] (with a score, at most 1 s, its length and 2 s).
    pub seconds: Option<f64>,
    pub beeper: bool,
}

/// Why a preview was not made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The take asked for does not fit the score: why (the pack would play the original).
    Refused(String),
    /// Anything else: the workspace, game, score or take is not there, or a file does not read.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Refused(why) => write!(f, "refused: {why}"),
            Error::Other(e) => f.write_str(e),
        }
    }
}

fn other(e: impl Into<String>) -> Error {
    Error::Other(e.into())
}

/// A preview made.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// Mono samples at [`rate`].
    pub samples: Vec<i16>,
    /// Where it starts in its run, in seconds.
    pub from: f64,
    /// What the score plays, when a score was asked for: `original`, `the live version`,
    /// `take 2 (<generator>)`, `C64 take chip-1 (<source>)`, `Spectrum 128 take ay-1 (<source>)`.
    pub plays: Option<String>,
    /// Whether the game's music came from the sound cache (else it was learned just now).
    pub cached: bool,
}

/// The WAV's sample rate: the FM-1's, rounded to a whole Hz as `music-corpus` writes it.
pub fn rate() -> u32 {
    sample_rate().round() as u32
}

/// The game of a workspace: its converted file's name and bytes, and where its key script would
/// be and its overrides.
struct GameFiles {
    file: String,
    data: Vec<u8>,
    keys: PathBuf,
    instruments: BTreeMap<u16, Vec<u8>>,
}

fn game_files(all: &Path, id: &str) -> Result<GameFiles, Error> {
    let conv = all.join("converted");
    let manifest = conv.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).map_err(|_| {
        other(format!("{}: no converted/manifest.toml: not a personal pack's workspace (make it with fmzx-pack personal <games> -o {0})", all.display()))
    })?;
    let games = parse_manifest(&text).map_err(|e| other(format!("{}: {e}", manifest.display())))?.game;
    let Some(g) = games.iter().find(|g| g.id == id) else {
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        return Err(other(format!("no game {id:?} in {} (its games: {})", all.display(), ids.join(", "))));
    };
    let at = conv.join(&g.file);
    let data = std::fs::read(&at).map_err(|e| other(format!("{}: {e}", at.display())))?;
    // the key script and overrides as the run had them: the games file's entry (a workspace made
    // from a games file elsewhere has none here, and its game is learned again if that matters)
    let entry = std::fs::read_to_string(all.join(GAMES_FILE))
        .ok()
        .and_then(|t| parse_personal_manifest(&t).ok())
        .and_then(|m| m.game.into_iter().find(|e| e.id == id));
    let (keys, instruments) = match entry {
        Some(e) => (game_path(all, &e).with_extension("keys"), e.instruments),
        None => (conv.join(format!("{id}.no-key-script")), BTreeMap::new()),
    };
    Ok(GameFiles { file: g.file.clone(), data, keys, instruments })
}

/// The takes in a score's folder: `take-<n>`, `chip-<n>` and `ay-<n>` folders with an `arranged.toml`.
fn takes_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join(TAKE_FILE).is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.starts_with("take-") || n.starts_with("chip-") || n.starts_with("ay-"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Makes the preview `req` asks for of the workspace `all` (see the module docs).
pub fn preview(all: &Path, req: &Request) -> Result<Preview, Error> {
    if !all.is_dir() {
        return Err(other(format!("{}: no such folder", all.display())));
    }
    let g = game_files(all, &req.game)?;
    let cache = all.join("sound-cache");
    std::fs::create_dir_all(&cache).map_err(|e| other(format!("{}: {e}", cache.display())))?;
    let script = std::fs::read_to_string(&g.keys).ok();
    let cached = cache_path(&g.data, script.as_deref(), &g.instruments, &cache).is_file();
    if !cached {
        eprintln!("{}: not in the sound cache: learning its music (this takes a while)", req.game);
    }
    let s = learn_music(&g.file, &g.data, &g.keys, &g.instruments, &cache);

    // the list as arrange_all makes it, with the score's take asked for
    let learned = s.list.clone().or_else(|| (!s.shadows.is_empty()).then(|| encode_list_scores(&[], &[], &[])));
    let scores = learned.as_deref().map(decode_scores).transpose().map_err(|e| other(format!("{}: its learned list: {e}", req.game)))?.unwrap_or_default();
    let dir = all.join(ARRANGE);
    let mut choices = arrange::read_choices(&dir).map_err(other)?;
    let mut asked: Option<(Of, &ScoreRecord, Take)> = None;
    if let Some((key, take)) = &req.score {
        let of = scores.iter().position(|r| score_key(r) == *key).map(Of::Score).or_else(|| s.shadows.iter().position(|r| score_key(r) == *key).map(Of::Shadow));
        let Some(of) = of else {
            let keys: Vec<String> = scores.iter().chain(&s.shadows).map(score_key).collect();
            let has = if keys.is_empty() { "it has none".to_string() } else { format!("its scores: {}", keys.join(", ")) };
            return Err(other(format!("game {:?} has no score {key} ({has})", req.game)));
        };
        let rec = match of {
            Of::Score(i) => &scores[i],
            Of::Shadow(i) => &s.shadows[i],
        };
        let name = format!("{}/{key}", req.game);
        match take {
            Take::Chosen => {}
            Take::Original => {
                choices.insert(name, Choice::Take(0));
            }
            Take::Of(c) => {
                let folder = dir.join(&req.game).join(key);
                if !folder.join(c.dir()).join(TAKE_FILE).is_file() {
                    let have = takes_in(&folder);
                    let have = if have.is_empty() { "it has none".to_string() } else { format!("its takes: {}", have.join(", ")) };
                    return Err(other(format!("score {key} has no take {} ({have})", c.dir())));
                }
                choices.insert(name, *c);
            }
        }
        asked = Some((of, rec, *take));
    }
    let mut list = s.list.clone();
    let mut plays = None;
    if let Some(l) = &learned {
        let runs = s.runs.clone();
        let verify = |lst: &[u8], i: usize, of: Of| verify_in_run(&g.file, &g.data, runs.as_ref(), lst, i, of);
        let got = arrange::apply(&dir, &req.game, l, &scores, &s.shadows, &choices, &verify);
        if let Some((of, _, take)) = asked {
            let (st, shadow) = match of {
                Of::Score(i) => (&got.status[i], false),
                Of::Shadow(i) => (&got.shadow_status[i], true),
            };
            plays = Some(match st {
                Some(Status::Refused { why, .. }) if take != Take::Chosen => return Err(Error::Refused(why.clone())),
                Some(Status::Take { take, generator, .. }) => format!("{take} ({generator})"),
                _ if shadow => "the live version".into(),
                _ => "original".into(),
            });
        }
        if s.list.is_some() || got.list != *l {
            list = Some(got.list);
        }
    }

    // the run, and where the preview starts in it
    let learner = s.runs.as_ref().map(|r| r.learner.clone()).unwrap_or_else(|| match script.as_deref().map(fmzx_pack::keys::parse_keys) {
        Some(Ok(steps)) => LearnerRun::Script(steps),
        _ => LearnerRun::Kind(RunKind::Generic, u64::from(crc32(&g.data))),
    });
    let (inputs, score_at) = match asked {
        None => (learner.inputs(), None),
        Some((of, rec, _)) => {
            let key = score_key(rec);
            let inputs = s.runs.as_ref().and_then(|r| r.inputs(of)).ok_or_else(|| other(format!("score {key}: the run that recorded it is not known")))?;
            let frame = score_frame(&s.details, &key).ok_or_else(|| other(format!("score {key}: the learner's details do not say where it starts")))?;
            (inputs, Some((frame, rec)))
        }
    };
    let seconds = req.seconds.unwrap_or_else(|| match score_at {
        Some((_, rec)) => {
            let len = (f64::from(rec.intro_t) + f64::from(rec.loop_t)) / f64::from(CLOCK_HZ);
            DEFAULT_SECONDS.min(secs(LEAD_FRAMES) + len + TAIL_SECONDS)
        }
        None => DEFAULT_SECONDS,
    });
    // whole frames (69,888 T, a little under 20 ms) to cover the seconds, then cut to them
    let frames = (seconds * f64::from(CLOCK_HZ) / f64::from(ZX_FRAME_TSTATES)).ceil().max(1.0) as usize;
    let want = (seconds * f64::from(rate())).round() as usize;
    // the run's first frame in the preview
    let first = score_at.map_or(0, |(f, _)| f.saturating_sub(LEAD_FRAMES));

    let mut m = machine(&g.file, &g.data).map_err(|e| other(format!("{}: {e}", g.file)))?;
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    if let Some(l) = &list {
        mixer.set_learned(l).map_err(|e| other(format!("{}: method B's decoder refuses its list (error {e})", req.game)))?;
    }
    // the mixer plays frame k when given frame k + delay: it runs from the run's start (its
    // methods follow the game from there), and the beeper's frames are kept as they come
    let delay = if req.beeper { 0 } else { SND_DELAY_FRAMES as usize };
    let (from, to) = (first + delay, first + delay + frames);
    let mut samples: Vec<i16> = Vec::with_capacity(want + 1024);
    for k in 0..to {
        let (rows, kemp) = inputs(k as u32);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        let level = m.edges_start_level();
        beeper.set_level(level);
        let edges = m.edges();
        let bp = beeper.render(edges);
        let got = if req.beeper { bp } else { mixer.frame(SND_MODE_FM_B, Some(&m), edges, level, &bp) };
        if k >= from {
            samples.extend(got);
        }
    }
    samples.truncate(want);
    let from = secs(first);
    Ok(Preview { samples, from, plays, cached })
}

/// The frame a score starts in, in the run that recorded it: from its line in the learner's
/// details (`recorded score: 20.7 s from frame 6378, ... (score C21A-62aabd0d)`, a shadow
/// score's `shadow score ...`), the frame of the edge that starts it.
fn score_frame(details: &str, key: &str) -> Option<usize> {
    let tag = format!("(score {key}");
    let line = details.lines().find(|l| {
        let t = l.trim_start();
        (t.starts_with("recorded score") || t.starts_with("shadow score")) && l.contains(&tag)
    })?;
    let rest = &line[line.find(" from frame ")? + " from frame ".len()..];
    rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())].parse().ok()
}

/// The seconds of `frames` frames.
fn secs(frames: usize) -> f64 {
    frames as f64 * f64::from(ZX_FRAME_TSTATES) / f64::from(CLOCK_HZ)
}

/// CRC-32 (IEEE), as the personal pack seeds a game's generic run.
fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |mut c, &b| {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & (c & 1).wrapping_neg());
        }
        c
    })
}
