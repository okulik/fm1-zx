// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The beeper as the FM-1's audio out plays it, captured by fm1-emu (left channel), in the
//! default Sound choice (Beeper), in FM: sound and in FM: engines. Needs the device build
//! (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml`; game 1 is `border.sna` (stripes and a 976 Hz tone).

use emu_gate::budgets::{choose_sound, press_to, sound_mode, SOUND_BEEPER, SOUND_FM, SOUND_FM_B, SOUND_ITEM, VIEW_GAME, VIEW_MENU};
use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use fmzx_pack::testprogs::{BORDER_TONE_HZ, TUNE_HZ, TUNE_NOTE_FRAMES};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PLAY: u8 = 12;
const HOME: u8 = 8;
const OCT_UP: u8 = 1;
const SELECT: u8 = 0;
const BORDER: u32 = 1;
const MASTER_ADC: u8 = 4;
/// I2S frames per second: 750000 / 17.
const RATE: f64 = 750_000.0 / 17.0;

/// Index of the game `id` in `games/games.toml`.
fn game_index(id: &str) -> u32 {
    let text = std::fs::read_to_string(repo().join("games/games.toml")).expect("read the manifest");
    let m = fmzx_pack::manifest::parse_manifest(&text).expect("parse the manifest");
    m.game.iter().position(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}")) as u32
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn firmware() -> &'static Firmware {
    static FW: OnceLock<Firmware> = OnceLock::new();
    FW.get_or_init(|| Firmware::load(&build_dir()))
}

fn pack() -> &'static [u8] {
    static PACK: OnceLock<Vec<u8>> = OnceLock::new();
    PACK.get_or_init(|| fmzx_pack::pack::build_pack(&repo().join("games/games.toml")).expect("build the game pack"))
}

/// Boots with MASTER at full, starts the game at `index` and returns once its view is up.
fn start_game(index: u32) -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 4 });
    m.set_adc(MASTER_ADC, 1023);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    m.turn(SELECT, index as i32);
    m.run_ms(200);
    m.press(PLAY, 30);
    m.run_ms(20);
    let s = m.stats();
    assert_eq!((s.view, s.game), (zx_sys::view::GAME as u32, index));
    m
}

fn start_border() -> Machine {
    start_game(BORDER)
}

/// The host's render of `border.sna`: `frames` frames of the host core through `Beeper`.
fn host_audio(frames: usize) -> Vec<i16> {
    let g = zx_sys::pack_game_at(pack(), BORDER).expect("border entry");
    let sna = g.extract(zx_sys::PACK_MAX_RAW as usize).expect("extract border");
    let mut zx = zx_sys::Machine::new();
    zx.load_sna(&sna).expect("load");
    let mut b = zx_sys::Beeper::new(750_000, 17);
    let mut out = Vec::new();
    for _ in 0..frames {
        zx.run_frame();
        b.set_level(zx.edges_start_level());
        out.extend(b.render(zx.edges()));
    }
    out
}

fn first_loud(a: &[i16]) -> usize {
    a.iter().position(|x| x.unsigned_abs() > 64).expect("no sample above 64")
}

#[test]
fn beeper_sample_exact() {
    let mut m = start_border();
    m.run_ms(2000);
    let dev = m.audio_left();
    let host = host_audio(150);
    let (d, h) = (first_loud(&dev), first_loud(&host));
    assert!(dev.len() >= d + 44_100, "only {} samples after the first loud one", dev.len() - d);
    let (dev, host) = (&dev[d..d + 44_100], &host[h..h + 44_100]);
    let first_diff = dev.iter().zip(host).position(|(a, b)| a != b);
    assert_eq!(first_diff, None, "device and host differ from sample {first_diff:?} on; underruns {}", m.stats().underruns);
}

/// Zero crossings (sign changes, 0 counted as positive) of `a`.
fn crossings(a: &[i16]) -> usize {
    a.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count()
}

#[test]
fn tone_frequency() {
    let mut m = start_border();
    m.run_ms(1500);
    let a = m.audio_left();
    // one second, starting 10 frames (8,809 samples) after the tone began
    let from = first_loud(&a) + 8_809;
    let n = RATE.round() as usize;
    assert!(a.len() >= from + n);
    let hz = crossings(&a[from..from + n]) as f64 / 2.0 * RATE / n as f64;
    println!("tone {hz:.2} Hz (host {BORDER_TONE_HZ} Hz)");
    assert!((hz - BORDER_TONE_HZ).abs() <= BORDER_TONE_HZ * 0.01, "tone {hz:.2} Hz, host {BORDER_TONE_HZ} Hz");
}

#[test]
fn silent_when_paused() {
    let mut m = start_border();
    m.run_ms(500);
    let under = m.stats().underruns;
    assert!(m.audio_left()[m.audio_left().len() - 4410..].iter().any(|x| x.unsigned_abs() > 1000), "no tone before the pause");
    m.hold(PLAY, true);
    let mut waited = 0;
    while m.stats().view != zx_sys::view::PAUSED as u32 {
        m.run_ms(1);
        waited += 1;
        assert!(waited < 200, "PLAY did not pause");
    }
    m.hold(PLAY, false);
    let frames = m.stats().frames;
    m.run_ms(50);
    let quiet_from = m.audio_left().len();
    m.run_ms(300);
    let a = m.audio_left();
    let loud = a[quiet_from..].iter().position(|x| x.unsigned_abs() >= 16);
    assert_eq!(loud, None, "a sample of {:?} at {loud:?} after 50 ms paused", loud.map(|i| a[quiet_from + i]));
    assert!(a.len() - quiet_from >= 290 * 44, "audio stopped playing");
    let s = m.stats();
    assert_eq!(s.view, zx_sys::view::PAUSED as u32);
    assert_eq!(s.underruns, under, "underruns counted while paused");
    assert_eq!(s.frames, frames, "frames ran while paused");
}

#[test]
fn tone_resumes_after_pause() {
    let mut m = start_border();
    m.run_ms(300);
    for (key, want) in [(PLAY, zx_sys::view::PAUSED), (PLAY, zx_sys::view::GAME)] {
        m.press(key, 30);
        let mut waited = 0;
        while m.stats().view != want as u32 {
            m.run_ms(1);
            waited += 1;
            assert!(waited < 300, "view {} after PLAY", m.stats().view);
        }
        m.run_ms(200);
    }
    let a = m.audio_left();
    let last = &a[a.len() - 4410..]; // the last 100 ms
    assert!(last.iter().any(|x| x.unsigned_abs() > 1000), "no tone after resuming");
    let hz = crossings(last) as f64 / 2.0 * RATE / last.len() as f64;
    assert!((hz - BORDER_TONE_HZ).abs() <= BORDER_TONE_HZ * 0.02, "tone {hz:.1} Hz after resuming");
    let s = m.stats();
    assert_eq!((s.underruns, s.overflows), (0, 0), "{s:?}");
}

#[test]
fn tune_pitches_in_firmware() {
    let mut m = start_game(game_index("tune"));
    // two passes of eight notes, plus the time the first note takes to start
    m.run_ms(2 * 8 * 25 * 20 + 600);
    let a = m.audio_left();
    let start = first_loud(&a);
    let frame = RATE * 69_888.0 / 3_500_000.0; // samples per Spectrum frame
    let note = TUNE_NOTE_FRAMES as f64 * frame;
    assert!(a.len() as f64 >= start as f64 + 16.0 * note, "only {} samples after the first loud one", a.len() - start);
    for k in 0..16 {
        let i = k % 8;
        // each note's window, 3 frames off each end
        let from = start + (k as f64 * note + 3.0 * frame) as usize;
        let to = start + ((k + 1) as f64 * note - 3.0 * frame) as usize;
        let hz = crossings(&a[from..to]) as f64 / 2.0 * RATE / (to - from) as f64;
        println!("note {i} (pass {}): {hz:.2} Hz (host {} Hz)", k / 8, TUNE_HZ[i]);
        assert!((hz - TUNE_HZ[i]).abs() <= TUNE_HZ[i] * 0.02, "note {i} of pass {}: {hz:.2} Hz, host {} Hz", k / 8, TUNE_HZ[i]);
    }
    // no silent stretch anywhere in the two passes, seams included: the longest run between
    // zero crossings is the lowest note's half-period (0.96 ms) plus the longest seam's excess
    // (0.1 ms) and a margin: measured 0.95 ms; 1.5 ms is far below a missed note
    let span = &a[start..start + (16.0 * note) as usize];
    let (mut run, mut longest) = (0usize, 0usize);
    for w in span.windows(2) {
        if (w[0] < 0) != (w[1] < 0) {
            run = 0;
        } else {
            run += 1;
            longest = longest.max(run);
        }
    }
    let ms = longest as f64 * 1000.0 / RATE;
    println!("longest run between zero crossings: {ms:.2} ms");
    assert!(ms <= 1.5, "{ms:.2} ms without a zero crossing");
    // and the level never rests near zero for long: a decaying held level would show here
    let (mut quiet, mut longest_quiet) = (0usize, 0usize);
    for x in span {
        quiet = if x.unsigned_abs() <= 64 { quiet + 1 } else { 0 };
        longest_quiet = longest_quiet.max(quiet);
    }
    let ms = longest_quiet as f64 * 1000.0 / RATE;
    println!("longest quiet run: {ms:.2} ms");
    assert!(ms <= 0.5, "{ms:.2} ms with |x| <= 64");
}

#[test]
fn beeper_default_unchanged() {
    // the Sound choice starts as Beeper, which every test above measures: the beeper itself,
    // with no delay (beeper_sample_exact) and nothing of FM
    let m = start_border();
    assert_eq!(sound_mode(&m), SOUND_BEEPER);
}

/// Where `needle` (at least a few hundred samples, not silence) first occurs in `hay`.
fn find(hay: &[i16], needle: &[i16]) -> Option<usize> {
    (0..=hay.len().checked_sub(needle.len())?).find(|&p| hay[p..p + needle.len()] == *needle)
}

/// Harmonics fitted per voice: a 2-operator FM voice at ratio 1:1 is a harmonic series of its
/// note (index up to 2.5: the energy lies in the first few).
const HARMONICS: usize = 6;

/// Cosine and sine columns of the harmonics of `hz` below Nyquist, over `n` samples.
fn harmonic_columns(hz: f64, n: usize, rate: f64) -> Vec<Vec<f64>> {
    let mut cols = Vec::new();
    for k in (1..=HARMONICS).filter(|&k| k as f64 * hz < rate * 0.45) {
        let w = 2.0 * std::f64::consts::PI * k as f64 * hz / rate;
        cols.push((0..n).map(|i| (w * i as f64).cos()).collect());
        cols.push((0..n).map(|i| (w * i as f64).sin()).collect());
    }
    cols
}

/// Least squares of `y` on `cols`: (residual energy, coefficients).
fn fit(y: &[f64], cols: &[Vec<f64>]) -> (f64, Vec<f64>) {
    let m = cols.len();
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let mut g: Vec<Vec<f64>> = (0..m).map(|i| (0..m).map(|j| dot(&cols[i], &cols[j])).collect()).collect();
    let mut c: Vec<f64> = cols.iter().map(|col| dot(col, y)).collect();
    let rhs = c.clone();
    for (i, row) in g.iter_mut().enumerate() {
        row[i] *= 1.0 + 1e-12;
    }
    // Gaussian elimination with partial pivoting
    for k in 0..m {
        let p = (k..m).max_by(|&a, &b| g[a][k].abs().total_cmp(&g[b][k].abs())).expect("rows");
        g.swap(k, p);
        c.swap(k, p);
        for r in k + 1..m {
            let f = g[r][k] / g[k][k];
            for j in k..m {
                g[r][j] -= f * g[k][j];
            }
            c[r] -= f * c[k];
        }
    }
    let mut x = vec![0.0; m];
    for k in (0..m).rev() {
        x[k] = (c[k] - (k + 1..m).map(|j| g[k][j] * x[j]).sum::<f64>()) / g[k][k];
    }
    (dot(y, y) - dot(&rhs, &x), x)
}

/// The pitch of each of `notes` (Hz) in `y` by a harmonic least-squares fit: each note's
/// frequency in turn is searched over +-3% in 0.2% steps with the others' harmonics fitted at
/// their own pitch (so a voice a semitone away is not mistaken for part of it). Returns, for the
/// voice with the largest fitted amplitude, (its note, the frequency found, the share of the
/// window's energy the fit explains); the frequency is `None` when the best fit is at either end
/// of the search (no pitch near the note).
fn strongest_pitch(y: &[f64], notes: &[f64], rate: f64) -> (f64, Option<f64>, f64) {
    let energy = y.iter().map(|v| v * v).sum::<f64>().max(1.0);
    let mut best = (0.0, None, 0.0, -1.0);
    for (j, &hz) in notes.iter().enumerate() {
        let fixed: Vec<Vec<f64>> =
            notes.iter().enumerate().filter(|&(i, _)| i != j).flat_map(|(_, &f)| harmonic_columns(f, y.len(), rate)).collect();
        let steps: Vec<f64> = (-15..=15).map(|s| hz * (1.0 + 0.002 * f64::from(s))).collect();
        let fits: Vec<(f64, f64)> = steps
            .iter()
            .map(|&f| {
                let free = harmonic_columns(f, y.len(), rate);
                let nf = free.len();
                let cols: Vec<Vec<f64>> = free.into_iter().chain(fixed.iter().cloned()).collect();
                let (res, x) = fit(y, &cols);
                (res, x[..nf].iter().map(|v| v * v).sum::<f64>().sqrt())
            })
            .collect();
        let k = (0..fits.len()).min_by(|&a, &b| fits[a].0.total_cmp(&fits[b].0)).expect("steps");
        let found = (k != 0 && k != fits.len() - 1).then_some(steps[k]);
        if fits[k].1 > best.3 {
            best = (hz, found, 1.0 - fits[k].0 / energy, fits[k].1);
        }
    }
    (best.0, best.1, best.2)
}

#[test]
fn sound_menu_switches_to_fm() {
    castaway_title_in(SOUND_FM);
}

#[test]
fn fm_engines_castaway_title() {
    castaway_title_in(SOUND_FM_B);
}

#[test]
fn sound_menu_cycles_three() {
    // from HOME, PLAY on the Sound item: Beeper -> FM: sound -> FM: engines -> Beeper, the menu
    // staying up and the game's frames stopped
    let mut m = start_border();
    press_to(&mut m, HOME, VIEW_MENU);
    let frames = m.stats().frames;
    for _ in 0..SOUND_ITEM {
        m.press(OCT_UP, 30);
        m.run_ms(30);
    }
    assert_eq!(sound_mode(&m), SOUND_BEEPER, "Beeper by default");
    for want in [SOUND_FM, SOUND_FM_B, SOUND_BEEPER] {
        m.press(PLAY, 30);
        m.run_ms(30);
        assert_eq!(sound_mode(&m), want, "the Sound item's next choice");
        assert_eq!(m.stats().view, VIEW_MENU, "the Sound item left the menu");
    }
    assert_eq!(m.stats().frames, frames, "frames ran in the menu");
    press_to(&mut m, HOME, VIEW_GAME);
    assert_eq!(sound_mode(&m), SOUND_BEEPER);
}

/// Castaway's title tune in the FM choice `sound` (FM: sound or FM: engines), chosen from the
/// HOME menu while it plays, then Beeper again: the device plays the host mixer's output sample
/// for sample, the strongest of the tune's notes (its RAM, the truth) shows at its pitch in the
/// FM windows, the frames are decoded as music (FM: sound: the decoder is confident; FM: engines:
/// the mixer plays B's FM), no underruns; and back in Beeper the live beeper again after the
/// crossfade.
fn castaway_title_in(sound: u32) {
    use music::truth::Engine;
    use zx_sys::{SndMixer, SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B};
    const D: usize = zx_sys::SND_DELAY_FRAMES as usize;
    // Castaway's title tune plays from frame 155; its long notes (one or both channels) run to
    // about frame 310
    let mut m = start_game(game_index("castaway"));
    for _ in 0..2000 {
        if m.stats().frames >= 165 {
            break;
        }
        m.run_ms(5);
    }
    assert!(m.stats().frames >= 165, "Castaway stopped at frame {}", m.stats().frames);
    let fm_from = choose_sound(&mut m, sound) as usize;
    let fm_mode = if sound == SOUND_FM { SND_MODE_FM_A } else { SND_MODE_FM_B };
    m.run_ms(2600);
    let beeper_from = choose_sound(&mut m, SOUND_BEEPER) as usize;
    m.run_ms(1000);
    let dev = m.audio_left();
    let s = m.stats();
    assert_eq!((s.underruns, s.overflows), (0, 0), "{s:?}");
    println!("FM from frame {fm_from}, Beeper again from frame {beeper_from}");
    assert!(beeper_from >= fm_from + 120, "only {} frames in FM", beeper_from - fm_from);

    // the host: the same frames through the beeper and the mixer, switched at the same frames,
    // and the notes the tune plays (from the game's RAM)
    let frames = beeper_from + 60;
    let mut zx = music::castaway();
    let mut b = zx_sys::Beeper::new(music::RATE.0, music::RATE.1);
    let mut mix = SndMixer::new(music::RATE.0, music::RATE.1);
    let mut dec = zx_sys::SndDecoder::new();
    let (mut beeper, mut mixed, mut starts, mut truth, mut confident) = (vec![], vec![], vec![0usize], vec![], vec![]);
    for i in 0..frames {
        let f = music::capture(&mut zx, &mut b, 1).pop().expect("a frame");
        truth.push(Engine::Castaway.read(&zx));
        let a_confident = dec.frame(&f.edges, f.start_level).confident != 0;
        let mode = if (fm_from..beeper_from).contains(&i) { fm_mode } else { SND_MODE_BEEPER };
        mixed.extend(mix.frame(mode, Some(&zx), &f.edges, f.start_level, &f.beeper));
        // music in frame i: A's decoder confident in it; B's FM played for it (the mixer's
        // decision for frame i comes SND_DELAY_FRAMES later)
        if sound == SOUND_FM {
            confident.push(a_confident);
        } else if i >= D {
            confident.push(mix.fm_on());
        }
        beeper.extend_from_slice(&f.beeper);
        starts.push(beeper.len());
    }

    // the device plays the host's mix sample for sample from the switch frame on (silence,
    // then the delayed path fading in) to well past the 2 s scored below
    let at = |t: usize| starts[t];
    let probe = at(fm_from + 8);
    let p = find(&dev, &mixed[probe..probe + 2000]).expect("the host's FM mix is not in the device audio");
    let (from, to) = (at(fm_from), at(fm_from + 120));
    let dev_fm = &dev[p + from - probe..p + to - probe];
    assert!(dev_fm == &mixed[from..to], "the device's FM audio differs from the host's mix");

    // after 200 ms, 2 s of 20 ms windows: output frame t plays frame t - 3's music; the strongest
    // of its notes (by a harmonic fit, strongest_pitch) shows at its pitch within 1%
    let rate = music::sample_rate();
    let (mut scored, mut right, mut fm_frames) = (0, 0, 0);
    for t in fm_from + 10..fm_from + 110 {
        let Some(truth) = truth[t - D].as_ref().filter(|v| !v.is_empty()) else { continue };
        let mut notes: Vec<f64> = Vec::new();
        for &hz in truth {
            if !notes.iter().any(|&n| (n - hz).abs() <= n * 0.005) {
                notes.push(hz); // a unison is one voice to the fit
            }
        }
        fm_frames += usize::from(confident[t - D]);
        let w: Vec<f64> = dev[p + at(t) - probe..p + at(t + 1) - probe].iter().map(|&x| f64::from(x)).collect();
        let (hz, got, explained) = strongest_pitch(&w, &notes, rate);
        scored += 1;
        let ok = explained >= 0.5 && got.is_some_and(|f| (f - hz).abs() <= hz * 0.01);
        right += usize::from(ok);
        if !ok {
            println!("frame {t}: notes {truth:.1?}, strongest {hz:.1} Hz found at {got:.1?} ({:.0}% explained)", explained * 100.0);
        }
    }
    println!("{right} of {scored} windows within 1% of the strongest note; {fm_frames} of them decoded as music");
    assert!(scored >= 90, "only {scored} windows with notes");
    assert!(right * 10 >= scored * 9, "{right} of {scored} windows within 1%");
    assert!(fm_frames * 10 >= scored * 9, "only {fm_frames} of {scored} windows decoded as music");

    // back to Beeper: after the crossfade from the delayed path (4 ms) the live beeper again,
    // delay-free, sample for sample
    let probe = at(beeper_from + 3);
    let p = find(&dev, &beeper[probe..probe + 2000]).expect("the live beeper is not in the device audio after the switch back");
    let xfade = (rate / f64::from(zx_sys::SND_XFADE_DIV)).ceil() as usize + 1;
    let (from, to) = (at(beeper_from), at(beeper_from + 45));
    assert!(&dev[p + from - probe..p + to - probe] == &mixed[from..to], "the switch back differs from the host's mix");
    assert!(&dev[p + from + xfade - probe..p + to - probe] == &beeper[from + xfade..to], "not the live beeper after the crossfade");
    assert!(dev[p + from - probe..p + from + xfade - probe] != beeper[from..from + xfade], "no crossfade at the switch back");
}
