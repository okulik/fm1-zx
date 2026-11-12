// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The beeper as the FM-1's audio out plays it, captured by fm1-emu (left channel). Needs the
//! device build (`FMZX_FM1_BUILD`).
//!
//! Pack: `games/games.toml`; game 1 is `border.sna` (stripes and a 976 Hz tone).

use emu_gate::fw::{build_dir, BootOpt, Firmware, Machine};
use fmzx_pack::testprogs::BORDER_TONE_HZ;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PLAY: u8 = 12;
const SELECT: u8 = 0;
const BORDER: u32 = 1;
const MASTER_ADC: u8 = 4;
/// I2S frames per second: 750000 / 17.
const RATE: f64 = 750_000.0 / 17.0;

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

/// Boots with MASTER at full, starts `border` and returns once its view is up.
fn start_border() -> Machine {
    let mut m = Machine::boot(&firmware().with_pack(pack()), BootOpt { rgb565: false, divider: 4 });
    m.set_adc(MASTER_ADC, 1023);
    m.run_ms(1500);
    assert_eq!(m.resets(), 0);
    m.turn(SELECT, BORDER as i32);
    m.run_ms(200);
    m.press(PLAY, 30);
    m.run_ms(20);
    let s = m.stats();
    assert_eq!((s.view, s.game), (zx_sys::view::GAME as u32, BORDER));
    m
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
