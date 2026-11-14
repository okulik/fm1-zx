// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The local game corpus: run every game of a folder, decode its beeper sound and report how much
//! of it plays as FM. The same C decoder, synth and mixer as the firmware, through `zx-sys`.
//! Games are only read; everything written goes under the output directory.
//!
//! Per game `<name>.<tap|tzx|sna|z80>` an optional `<name>.keys` beside it holds the key script:
//! steps `ACTION@ms[+hold]` separated by commas or line breaks (`#` starts a comment line), where
//! the action is a Spectrum key (`key.ENTER`) or a Kempston direction (`kempston.fire`) as in
//! the `demo` strings of `games/games.toml`; hold is 100 ms when not given.

use crate::score::{score, Score};
use crate::truth::Engine;
use crate::{capture, decode, played_hz, sample_rate, tzx, wav, Frame, RATE};
use fmzx_pack::manifest::parse_action;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use zx_sys::{Beeper, Machine, SndMixer, SND_MODE_FM_A, SND_STYLE_FLIPS, SND_STYLE_PULSES, SND_STYLE_SLOTS};

/// Frame time in ms as the key scripts count it (50 Hz).
const FRAME_MS: u64 = 20;

#[derive(Clone, Debug)]
pub struct Options {
    pub seconds: u32,
    pub wav: bool,
    pub out: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Options { seconds: 40, wav: false, out: crate::repo().join("build/music-corpus") }
    }
}

/// One game's result.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub file: String,
    /// `ok`, or `not loadable: <why>`.
    pub status: String,
    pub frames: usize,
    /// Frames with any speaker edge.
    pub sound_frames: usize,
    /// Of those frames, the percentage decided as FM.
    pub fm_pct: f64,
    /// Of those frames, the percentage in each decoder style: slots, flips, pulses.
    pub style_pct: [f64; 3],
    pub engine: Option<Engine>,
    /// Against the engine's own notes, when its engine was recognised and played.
    pub truth: Option<Score>,
    /// Anything that went wrong beside the measurements (a WAV that could not be written).
    pub note: String,
}

impl Row {
    /// Seconds of the run with sound.
    pub fn sound_seconds(&self) -> f64 {
        self.sound_frames as f64 * FRAME_MS as f64 / 1000.0
    }
}

/// One step of a key script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub action: u8,
    pub at: u64,
    pub hold: u64,
}

/// Parses a key script (see the module docs).
pub fn parse_keys(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        for p in line.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, when) = p.split_once('@').ok_or_else(|| format!("step {p:?}: expected ACTION@ms"))?;
            let (at, hold) = when.split_once('+').map_or((when, "100"), |(a, h)| (a, h));
            let action = parse_action(key.trim())?;
            let num = |s: &str| s.trim().parse::<u64>().map_err(|_| format!("step {p:?}: bad time"));
            steps.push(Step { action, at: num(at)?, hold: num(hold)? });
        }
    }
    Ok(steps)
}

/// The Spectrum key rows and Kempston bits held at `ms`.
fn inputs(steps: &[Step], ms: u64) -> ([u8; 8], u8) {
    let (mut rows, mut kemp) = ([0u8; 8], 0u8);
    for s in steps.iter().filter(|s| s.at <= ms && ms < s.at + s.hold) {
        if s.action & 0x40 != 0 {
            kemp |= 1 << (s.action & 0x3f);
        } else if s.action < 40 {
            rows[s.action as usize / 5] |= 1 << (s.action % 5);
        }
    }
    (rows, kemp)
}

/// A machine with the game file loaded (tapes autostart); `Err` says why it cannot be.
pub fn load(path: &Path) -> Result<Machine, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut m = Machine::new();
    match ext.as_str() {
        "tap" | "tzx" => {
            let tap = if ext == "tzx" { tzx::to_tap(&data)? } else { data };
            m.tape_insert(&tap).map_err(|e| format!("tape rejected ({e})"))?;
            m.tape_autostart();
        }
        "sna" => m.load_sna(&data).map_err(|e| format!("snapshot rejected ({e})"))?,
        "z80" => m.load_z80(&data).map_err(|e| format!("snapshot rejected ({e})"))?,
        _ => return Err("unknown file type".into()),
    }
    Ok(m)
}

/// A music engine's sound loop in memory: `len` bytes from `base`, the operand bytes the engine
/// rewrites while it plays (`wild`, offsets) left out, hashed with [`digest`]. Digests, not the
/// bytes: the games' code does not enter the repository.
struct Sig {
    engine: Engine,
    base: u16,
    len: usize,
    wild: &'static [usize],
    digest: u64,
}

const DIGEST_CASTAWAY: u64 = 0xd9ca_bc62_66a8_0034;
const DIGEST_COLONY8: u64 = 0x46ee_814f_9cab_12d7;
const DIGEST_MM_TITLE: u64 = 0xe6cf_0316_1ab2_4a00;
const DIGEST_MM_GAME: u64 = 0x5b1e_e7bd_a322_b262;

const SIGS: [Sig; 4] = [
    Sig { engine: Engine::Castaway, base: 0xc868, len: 36, wild: &[5, 9, 13, 18, 29], digest: DIGEST_CASTAWAY },
    Sig { engine: Engine::Colony8, base: 0x94fc, len: 36, wild: &[4, 14, 22], digest: DIGEST_COLONY8 },
    // Cave Runner's title tune loop and its in-game burst loop (no operands change)
    Sig { engine: Engine::CaveRunner, base: 0x92f8, len: 22, wild: &[], digest: DIGEST_MM_TITLE },
    Sig { engine: Engine::CaveRunner, base: 0x8852, len: 40, wild: &[], digest: DIGEST_MM_GAME },
];

/// 64-bit FNV-1a of the bytes `len` from `base`, the `wild` offsets skipped.
pub fn digest(m: &Machine, base: u16, len: usize, wild: &[usize]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for i in (0..len).filter(|i| !wild.contains(i)) {
        h = (h ^ m.peek(base.wrapping_add(i as u16)) as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The music engine whose sound loop is in the machine's memory now, recognised by the digest of
/// its fixed loop bytes at the known addresses (not by file name).
pub fn detect_engine(m: &Machine) -> Option<Engine> {
    SIGS.iter().find(|g| digest(m, g.base, g.len, g.wild) == g.digest).map(|g| g.engine)
}

/// Runs one game for `opts.seconds`; with `opts.wav` writes its two WAV files under `opts.out`.
pub fn run_game(path: &Path, opts: &Options) -> Row {
    let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut row = Row { file: file.clone(), ..Row::default() };
    let mut m = match load(path) {
        Ok(m) => m,
        Err(e) => {
            row.status = format!("not loadable: {e}");
            return row;
        }
    };
    let steps = match std::fs::read_to_string(path.with_extension("keys")) {
        Ok(t) => match parse_keys(&t) {
            Ok(s) => s,
            Err(e) => {
                row.status = format!("not loadable: key script: {e}");
                return row;
            }
        },
        Err(_) => Vec::new(),
    };
    let frames = opts.seconds as usize * 1000 / FRAME_MS as usize;
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut run: Vec<Frame> = Vec::with_capacity(frames);
    let mut truth: Vec<Option<Vec<f64>>> = Vec::with_capacity(frames);
    let mut engine = None;
    for k in 0..frames {
        let (rows, kemp) = inputs(&steps, k as u64 * FRAME_MS);
        m.set_keys(rows);
        m.set_kempston(kemp);
        run.extend(capture(&mut m, &mut beeper, 1));
        if engine.is_none() {
            engine = detect_engine(&m);
        }
        truth.push(engine.and_then(|e| e.read(&m)));
    }
    let dec = decode(run.iter().map(|f| (f.edges.as_slice(), f.start_level)));
    // frames with sound: a speaker level change in the frame
    let sounding: Vec<usize> = (0..run.len())
        .filter(|&i| {
            let mut level = run[i].start_level & 2;
            run[i].edges.iter().any(|e| std::mem::replace(&mut level, e.level & 2) != e.level & 2)
        })
        .collect();
    let pct = |n: usize| 100.0 * n as f64 / sounding.len().max(1) as f64;
    row.status = "ok".into();
    row.frames = run.len();
    row.sound_frames = sounding.len();
    row.fm_pct = pct(sounding.iter().filter(|&&i| dec[i].confident != 0).count());
    for (k, style) in [SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES].into_iter().enumerate() {
        row.style_pct[k] = pct(sounding.iter().filter(|&&i| dec[i].style == style).count());
    }
    row.engine = engine;
    if truth.iter().any(Option::is_some) {
        row.truth = Some(score(&truth, &played_hz(&dec)));
    }
    if opts.wav {
        let stem = path.file_stem().map_or_else(|| "game".into(), |s| s.to_string_lossy().into_owned());
        let mut mixer = SndMixer::new(RATE.0, RATE.1);
        let mut fm = Vec::new();
        let mut bp = Vec::new();
        for f in &run {
            fm.extend(mixer.frame(SND_MODE_FM_A, &f.edges, f.start_level, &f.beeper));
            bp.extend_from_slice(&f.beeper);
        }
        let rate = sample_rate().round() as u32;
        for (suffix, s) in [("beeper", &bp), ("fm", &fm)] {
            if let Err(e) = wav::write(&opts.out.join(format!("{stem}_{suffix}.wav")), s, rate) {
                row.note = format!("wav not written: {e}");
            }
        }
    }
    row
}

/// The game files of `folder` (`.tap`, `.tzx`, `.sna`, `.z80`), by name.
pub fn game_files(folder: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let e = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            ["tap", "tzx", "sna", "z80"].contains(&e.as_str())
        })
        .collect();
    v.sort();
    Ok(v)
}

/// Runs every game of `folder` and writes `report.txt` and `report.csv` under `opts.out`.
pub fn run_folder(folder: &Path, opts: &Options) -> Result<Vec<Row>, String> {
    std::fs::create_dir_all(&opts.out).map_err(|e| format!("{}: {e}", opts.out.display()))?;
    let files = game_files(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let rows: Vec<Row> = files
        .iter()
        .map(|p| {
            let r = run_game(p, opts);
            eprintln!("{}: {}", r.file, r.status);
            r
        })
        .collect();
    for (name, text) in [("report.txt", report_txt(&rows, opts)), ("report.csv", report_csv(&rows))] {
        std::fs::write(opts.out.join(name), text).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(rows)
}

fn engine_name(e: Option<Engine>) -> &'static str {
    match e {
        Some(Engine::Castaway) => "castaway",
        Some(Engine::Colony8) => "colony8",
        Some(Engine::CaveRunner) => "cave-runner",
        None => "",
    }
}

/// The human-readable report.
pub fn report_txt(rows: &[Row], opts: &Options) -> String {
    let mut s = format!(
        "music-corpus: {} games, {} s each; FM = frames the decoder is confident about, of the frames with sound.\n\
         styles: slots / flips / pulses, % of the frames with sound. Truth (engines read from RAM): frame precision and\n\
         recall, notes found of true notes, detected notes matching no true note (wrong).\n\n",
        rows.len(),
        opts.seconds
    );
    let _ = writeln!(s, "{:<28} {:>7} {:>6}  {:<17}  truth", "game", "sound s", "FM %", "styles s/f/p %");
    for r in rows {
        if r.status != "ok" {
            let _ = writeln!(s, "{:<28} {}", r.file, r.status);
            continue;
        }
        let truth = r.truth.as_ref().map_or(String::new(), |t| {
            format!(
                "{}: precision {:.1}% recall {:.1}% notes {}/{} wrong {}",
                engine_name(r.engine),
                100.0 * t.precision(),
                100.0 * t.recall(),
                t.notes_found,
                t.true_notes,
                t.wrong.len()
            )
        });
        let truth = if r.note.is_empty() { truth } else { format!("{truth} ({})", r.note) };
        let _ = writeln!(
            s,
            "{:<28} {:>7.1} {:>6.1}  {:>5.1}/{:>5.1}/{:>5.1}  {}",
            r.file,
            r.sound_seconds(),
            r.fm_pct,
            r.style_pct[0],
            r.style_pct[1],
            r.style_pct[2],
            truth
        );
    }
    s
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The CSV report: one line per game.
pub fn report_csv(rows: &[Row]) -> String {
    let mut s = String::from(
        "file,status,sound_seconds,fm_percent,slots_percent,flips_percent,pulses_percent,engine,truth_frames,precision_percent,recall_percent,notes_true,notes_found,wrong_notes,note\n",
    );
    for r in rows {
        let _ = write!(s, "{},{}", csv_field(&r.file), csv_field(&r.status));
        if r.status == "ok" {
            let _ = write!(
                s,
                ",{:.2},{:.1},{:.1},{:.1},{:.1},{}",
                r.sound_seconds(),
                r.fm_pct,
                r.style_pct[0],
                r.style_pct[1],
                r.style_pct[2],
                engine_name(r.engine)
            );
            match &r.truth {
                Some(t) => {
                    let _ = write!(
                        s,
                        ",{},{:.1},{:.1},{},{},{}",
                        t.frames,
                        100.0 * t.precision(),
                        100.0 * t.recall(),
                        t.true_notes,
                        t.notes_found,
                        t.wrong.len()
                    );
                }
                None => s.push_str(",,,,,,"),
            }
        } else {
            s.push_str(",,,,,,,,,,,,");
        }
        let _ = write!(s, ",{}", csv_field(&r.note));
        s.push('\n');
    }
    s
}
