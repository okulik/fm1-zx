// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The local game corpus: run every game of a folder, decode its beeper sound and report how much
//! of it plays as FM in each FM choice (method A, the decoder; method B, the known engines). The
//! same C decoder, engine table, synth and mixer as the firmware, through `zx-sys`.
//! Games are only read; everything written goes under the output directory. Tapes with a loader
//! of their own are loaded through the tape signal first ([`load`]).
//!
//! Per game `<name>.<tap|tzx|sna|z80>` an optional `<name>.keys` beside it holds the key script:
//! steps `ACTION@ms[+hold]` separated by commas or line breaks (`#` starts a comment line), where
//! the action is a Spectrum key (`key.ENTER`) or a Kempston direction (`kempston.fire`) as in
//! the `demo` strings of `games/games.toml`; hold is 100 ms when not given.

use crate::score::{score, Score};
use crate::truth::Engine;
use crate::{capture, decode, hz_of_period_q8, played_hz, sample_rate, tzx, wav, Frame, RATE};
use fmzx_pack::manifest::parse_action;
use fmzx_pack::tapeload::{is_128k, load_signal, trap_trial};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use zx_sys::{
    Beeper, Machine, SndEngines, SndFrame, SndMixer, SND_DELAY_FRAMES, SND_MODE_FM_A, SND_MODE_FM_B, SND_STYLE_FLIPS, SND_STYLE_PULSES,
    SND_STYLE_SLOTS,
};

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
    /// Of those frames, the percentage the mixer plays as FM in FM: sound (method A): the decoder's
    /// confident decisions through the mixer's hysteresis (FM from the second confident decision
    /// in a row), as the firmware plays them.
    pub fm_pct: f64,
    /// Of those frames, the percentage in each decoder style: slots, flips, pulses.
    pub style_pct: [f64; 3],
    pub engine: Option<Engine>,
    /// Against the engine's own notes, when its engine was recognised and played.
    pub truth: Option<Score>,
    /// The same in FM: engines (method B, the built-in engine table, fed the machine each frame).
    pub b_fm_pct: f64,
    /// The engines B was confident about, by name, in the order first heard.
    pub b_engines: Vec<String>,
    /// B against the engine's own notes: in each frame B is confident about, per voice the last
    /// pitch it sounded in the frame.
    pub b_truth: Option<Score>,
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

/// A machine with the game file loaded; `Err` says why it cannot be. Time 0 of the run is the
/// machine returned:
///
/// - a standard tape (`.tap`, or a `.tzx` of standard blocks) that the ROM trap's trial loads:
///   the tape inserted and autostarted, as the device runs it (time 0 is the autostart);
/// - any other `.tzx`, and a standard tape the trial rejects: the machine loaded through the
///   tape signal, at the snapshot after the load and its settle time;
/// - a 48K `.sna` or `.z80`: the snapshot;
/// - a 128K file: `Err("128K")`.
pub fn load(path: &Path) -> Result<Machine, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if is_128k(&data, &ext) {
        return Err("128K".into());
    }
    let mut m = Machine::new();
    match ext.as_str() {
        "tap" | "tzx" => {
            let tap = if ext == "tzx" { tzx::to_tap(&data).ok() } else { Some(data.clone()) };
            let Some(tap) = tap else { return load_signal(&data) };
            match trap_trial(&tap) {
                Ok(()) => {
                    m.tape_insert(&tap).map_err(|e| format!("tape rejected ({e})"))?;
                    m.tape_autostart();
                }
                Err(trap) => {
                    return load_signal(&data).map_err(|e| if e == trap { e } else { format!("{e} (through the ROM trap: {trap})") });
                }
            }
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
const DIGEST_UNISON: u64 = 0x6267_40fe_f731_1d91;

const SIGS: [Sig; 5] = [
    Sig { engine: Engine::Castaway, base: 0xc868, len: 36, wild: &[5, 9, 13, 18, 29], digest: DIGEST_CASTAWAY },
    Sig { engine: Engine::Colony8, base: 0x94fc, len: 36, wild: &[4, 14, 22], digest: DIGEST_COLONY8 },
    // Cave Runner's title tune loop and its in-game burst loop (no operands change)
    Sig { engine: Engine::CaveRunner, base: 0x92f8, len: 22, wild: &[], digest: DIGEST_MM_TITLE },
    Sig { engine: Engine::CaveRunner, base: 0x8852, len: 40, wild: &[], digest: DIGEST_MM_GAME },
    // Unison's melody loop, its two reload operands left out
    Sig { engine: Engine::Unison, base: 0x81c6, len: 36, wild: &[7, 8, 25, 26], digest: DIGEST_UNISON },
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

/// The name a game's WAV files start with: its file name with the dot made an underscore
/// (`foo.tap` -> `foo_tap`), so `foo.tap` and `foo.tzx` in one folder keep their own.
pub fn wav_stem(path: &Path) -> String {
    let stem = path.file_stem().map_or_else(|| "game".into(), |s| s.to_string_lossy().into_owned());
    match path.extension() {
        Some(e) => format!("{stem}_{}", e.to_string_lossy()),
        None => stem,
    }
}

/// The pitches (Hz) a decision plays: per voice, the last pitch it sounded in the frame (at its
/// end, or before an event released it in the frame); none if it is not confident (the beeper
/// plays).
fn sounded_hz(f: &SndFrame) -> Vec<f64> {
    if f.confident == 0 {
        return Vec::new();
    }
    let mut last = f.tone.map(|v| if v.level != 0 { v.period_q8 } else { 0 });
    for e in f.events().iter().filter(|e| e.period_q8 != 0 && e.level != 0) {
        if let Some(p) = last.get_mut(e.voice as usize) {
            *p = e.period_q8;
        }
    }
    last.into_iter().filter(|&p| p != 0).map(hz_of_period_q8).collect()
}

/// Runs one game for `opts.seconds`; with `opts.wav` writes its three WAV files under `opts.out`
/// (`<wav_stem>_beeper.wav`, `<wav_stem>_fm.wav` for FM: sound, `<wav_stem>_fm_b.wav` for FM:
/// engines).
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
    // the mixer in each FM mode, as the firmware runs it: which frames it plays as FM (its
    // hysteresis over the decisions) and its output; FM: sound's is given no machine (method A
    // needs none), FM: engines' the machine right after each frame, as is B on its own (its
    // decisions are the mixer's)
    let (mut mix_a, mut mix_b) = (SndMixer::new(RATE.0, RATE.1), SndMixer::new(RATE.0, RATE.1));
    let mut eng = SndEngines::new();
    let (mut fm, mut fm_b) = (Vec::new(), Vec::new());
    let (mut on_a, mut on_b) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    let mut b_dec = Vec::with_capacity(frames);
    let mut level = 0;
    for k in 0..frames {
        let (rows, kemp) = inputs(&steps, k as u64 * FRAME_MS);
        m.set_keys(rows);
        m.set_kempston(kemp);
        let f = capture(&mut m, &mut beeper, 1).pop().expect("one frame");
        if engine.is_none() {
            engine = detect_engine(&m);
        }
        truth.push(engine.and_then(|e| e.read(&m)));
        let a = mix_a.frame(SND_MODE_FM_A, None, &f.edges, f.start_level, &f.beeper);
        let b = mix_b.frame(SND_MODE_FM_B, Some(&m), &f.edges, f.start_level, &f.beeper);
        b_dec.push(eng.frame(&m, &f.edges, f.start_level));
        if opts.wav {
            fm.extend(a);
            fm_b.extend(b);
        }
        if k >= SND_DELAY_FRAMES as usize {
            on_a.push(mix_a.fm_on());
            on_b.push(mix_b.fm_on());
        }
        level = f.edges.last().map_or(f.start_level, |e| e.level);
        run.push(f);
    }
    // the last frames' decisions come with frames after the run: silent ones
    let quiet = vec![0i16; run.last().map_or(0, |f| f.beeper.len())];
    while on_a.len() < run.len() {
        mix_a.frame(SND_MODE_FM_A, None, &[], level, &quiet);
        mix_b.frame(SND_MODE_FM_B, Some(&m), &[], level, &quiet);
        b_dec.push(eng.frame(&m, &[], level));
        on_a.push(mix_a.fm_on());
        on_b.push(mix_b.fm_on());
    }
    b_dec.drain(..SND_DELAY_FRAMES as usize);
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
    row.fm_pct = pct(sounding.iter().filter(|&&i| on_a[i]).count());
    row.b_fm_pct = pct(sounding.iter().filter(|&&i| on_b[i]).count());
    for (k, style) in [SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES].into_iter().enumerate() {
        row.style_pct[k] = pct(sounding.iter().filter(|&&i| dec[i].style == style).count());
    }
    for d in b_dec.iter().filter(|d| d.confident != 0 && d.engine != 0) {
        let name = eng.name(d.engine);
        if !row.b_engines.contains(&name) {
            row.b_engines.push(name);
        }
    }
    row.engine = engine;
    if truth.iter().any(Option::is_some) {
        row.truth = Some(score(&truth, &played_hz(&dec)));
        row.b_truth = Some(score(&truth, &b_dec.iter().map(sounded_hz).collect::<Vec<_>>()));
    }
    if opts.wav {
        let stem = wav_stem(path);
        let bp: Vec<i16> = run.iter().flat_map(|f| f.beeper.iter().copied()).collect();
        let rate = sample_rate().round() as u32;
        for (suffix, s) in [("beeper", &bp), ("fm", &fm), ("fm_b", &fm_b)] {
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
        Some(Engine::Unison) => "unison",
        None => "",
    }
}

/// A score's frame and note numbers.
fn score_txt(t: &Score) -> String {
    format!(
        "precision {:.1}% recall {:.1}% notes {}/{} wrong {}",
        100.0 * t.precision(),
        100.0 * t.recall(),
        t.notes_found,
        t.true_notes,
        t.wrong.len()
    )
}

/// The human-readable report.
pub fn report_txt(rows: &[Row], opts: &Options) -> String {
    let mut s = format!(
        "music-corpus: {} games, {} s each; FM = frames the mixer plays as FM (the decoder's confident decisions\n\
         after the mixer's hysteresis), of the frames with sound, in FM: sound (A) and FM: engines (B). styles\n\
         (A): slots / flips / pulses, % of the frames with sound. Truth (engines read from RAM): frame precision\n\
         and recall of the confident decisions, notes found of true notes, detected notes matching no true note\n\
         (wrong); for B, after the engines it recognised.\n\n",
        rows.len(),
        opts.seconds
    );
    let _ = writeln!(s, "{:<28} {:>7} {:>6} {:>6}  {:<17}  truth", "game", "sound s", "FM %", "B FM %", "styles s/f/p %");
    for r in rows {
        if r.status != "ok" {
            let _ = writeln!(s, "{:<28} {}", r.file, r.status);
            continue;
        }
        let mut truth = r.truth.as_ref().map_or(String::new(), |t| format!("{}: {}", engine_name(r.engine), score_txt(t)));
        if !r.b_engines.is_empty() || r.b_truth.is_some() {
            let b = r.b_truth.as_ref().map_or(String::new(), |t| format!(" {}", score_txt(t)));
            let sep = if truth.is_empty() { "" } else { "; " };
            truth = format!("{truth}{sep}B: {}:{b}", r.b_engines.join(" + "));
        }
        let truth = if r.note.is_empty() { truth } else { format!("{truth} ({})", r.note) };
        let _ = writeln!(
            s,
            "{:<28} {:>7.1} {:>6.1} {:>6.1}  {:>5.1}/{:>5.1}/{:>5.1}  {}",
            r.file,
            r.sound_seconds(),
            r.fm_pct,
            r.b_fm_pct,
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
        "file,status,sound_seconds,fm_percent,slots_percent,flips_percent,pulses_percent,engine,truth_frames,precision_percent,recall_percent,notes_true,notes_found,wrong_notes,\
         b_fm_percent,b_engines,b_precision_percent,b_recall_percent,b_notes_found,b_wrong_notes,note\n",
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
            let _ = write!(s, ",{:.1},{}", r.b_fm_pct, csv_field(&r.b_engines.join(" + ")));
            match &r.b_truth {
                Some(t) => {
                    let _ = write!(s, ",{:.1},{:.1},{},{}", 100.0 * t.precision(), 100.0 * t.recall(), t.notes_found, t.wrong.len());
                }
                None => s.push_str(",,,,"),
            }
        } else {
            s.push_str(",,,,,,,,,,,,,,,,,,");
        }
        let _ = write!(s, ",{}", csv_field(&r.note));
        s.push('\n');
    }
    s
}
