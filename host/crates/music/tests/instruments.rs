// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The FM synth's instruments (`core/sound/fm.c`): each tonal voice plays the patch of its role
//! (bass, lead, pad), latched when a note starts on it, and the drum is noise plus a tone. The
//! mix stays as loud as before the instruments, and no decoder decision changes.

use music::{castaway, colony8, period_q8, test_sna, RATE};
use std::f64::consts::TAU;
use zx_sys::{
    Beeper, Machine, SndDecoder, SndEngines, SndEvent, SndFm, SndFrame, SndMixer, SndVoice, SND_MODE_FM_A, SND_MODE_FM_B, SND_ROLE_AUTO,
    SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD,
};

// ---- the corpus fixtures ---------------------------------------------------------------------

/// Frames each fixture runs: 30 s, the music of all three well under way.
const RUN: usize = 1500;

fn fixtures() -> [(&'static str, fn() -> Machine); 3] {
    [("tune.sna", || test_sna("tune.sna")), ("castaway", castaway), ("colony8", colony8)]
}

fn fnv_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h = (*h ^ b as u64).wrapping_mul(0x100000001b3);
    }
}

/// A decision's hash: every field of `snd_frame` but the roles.
fn decision_hash(h: &mut u64, f: &SndFrame) {
    for v in &f.tone {
        fnv_bytes(h, &v.period_q8.to_le_bytes());
        fnv_bytes(h, &[v.level]);
    }
    fnv_bytes(h, &[f.drum, f.style, f.confident, f.engine, f.n_ev]);
    for e in f.events() {
        fnv_bytes(h, &e.t.to_le_bytes());
        fnv_bytes(h, &e.period_q8.to_le_bytes());
        fnv_bytes(h, &[e.voice, e.level]);
    }
}

/// Method A's and method B's decisions over a fixture's run, hashed (A, B).
fn decisions(mut m: Machine) -> (u64, u64) {
    let (mut a, mut b) = (SndDecoder::new(), SndEngines::new());
    let (mut ha, mut hb) = (0xcbf29ce484222325u64, 0xcbf29ce484222325u64);
    for _ in 0..RUN {
        m.run_frame();
        decision_hash(&mut ha, &a.frame(m.edges(), m.edges_start_level()));
        decision_hash(&mut hb, &b.frame(&m, m.edges(), m.edges_start_level()));
    }
    (ha, hb)
}

/// Method A's and B's decisions on the corpus fixtures, as they were before the instruments
/// (measured at d476d2a, whose sound is main's, by printing `decisions` for each fixture); B's
/// re-measured when its hold took in a tune's rests (SND_B_HOLD 50: it claims more silent frames).
const DECISIONS_BEFORE: [(&str, u64, u64); 3] = [
    ("tune.sna", 0x537823d177f40da8, 0xb7c0ca46af747ae5),
    ("castaway", 0x462b732f90e30155, 0x33a0f4f49c6d1206),
    ("colony8", 0x18ef71ee8b3c8163, 0xf12658e790d0d393),
];

#[test]
fn decisions_unchanged() {
    for ((name, m), (want_name, want_a, want_b)) in fixtures().into_iter().zip(DECISIONS_BEFORE) {
        assert_eq!(name, want_name);
        let (a, b) = decisions(m());
        println!("(\"{name}\", {a:#018x}, {b:#018x}),");
        assert_eq!((a, b), (want_a, want_b), "{name}: decisions (A, B)");
    }
}

// ---- loudness ------------------------------------------------------------------------------

/// The mixer's FM output over a fixture's run in `mode`: the samples of the frames the mixer
/// plays as FM (its hysteresis on), as the firmware runs it (the machine given to the mixer).
fn fm_output(mut m: Machine, mode: u8) -> Vec<i16> {
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    let mut out = Vec::new();
    for _ in 0..RUN {
        m.run_frame();
        let start_level = m.edges_start_level();
        beeper.set_level(start_level);
        let edges = m.edges().to_vec();
        let samples = beeper.render(&edges);
        let o = mixer.frame(mode, Some(&m), &edges, start_level, &samples);
        if mixer.fm_on() {
            out.extend(o);
        }
    }
    out
}

/// (samples, peak, RMS, samples at full scale) of `s`.
fn loudness(s: &[i16]) -> (usize, i32, f64, usize) {
    let peak = s.iter().map(|&x| (x as i32).abs()).max().unwrap_or(0);
    let rms = if s.is_empty() { 0.0 } else { (s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt() };
    let full = s.iter().filter(|&&x| x >= 32767 || x <= -32767).count();
    (s.len(), peak, rms, full)
}

/// The FM output's loudness per fixture and mode before the instruments: (fixture, mode,
/// samples played as FM, peak, RMS, samples at full scale), measured at d476d2a (main's sound)
/// with `fm_output` and `loudness` as they are here; FM: engines' sample counts re-measured when
/// B's hold took in a tune's rests (more silent frames play as FM; peak and RMS as measured then).
const LOUDNESS_BEFORE: [(&str, u8, usize, i32, f64, usize); 6] = [
    ("tune.sna", SND_MODE_FM_A, 1294983, 6056, 2441.7, 0),
    ("tune.sna", SND_MODE_FM_B, 0, 0, 0.0, 0), // no engine entry knows tune.sna's loop
    ("castaway", SND_MODE_FM_A, 1166366, 10671, 2784.4, 0),
    ("castaway", SND_MODE_FM_B, 1175175, 12500, 3568.8, 0),
    ("colony8", SND_MODE_FM_A, 1183103, 13362, 2209.9, 0),
    ("colony8", SND_MODE_FM_B, 1195437, 16979, 2950.9, 0),
];

fn db(a: f64, b: f64) -> f64 {
    20.0 * (a / b).log10()
}

#[test]
fn loudness_matches_today() {
    let mut bad = Vec::new();
    let mut i = 0;
    for (name, m) in fixtures() {
        for mode in [SND_MODE_FM_A, SND_MODE_FM_B] {
            let (n, peak, rms, full) = loudness(&fm_output(m(), mode));
            let (want_name, want_mode, want_n, want_peak, want_rms, want_full) = LOUDNESS_BEFORE[i];
            i += 1;
            assert_eq!((name, mode), (want_name, want_mode));
            // the same frames play as FM
            assert_eq!(n, want_n, "{name} mode {mode}: samples played as FM");
            if n == 0 {
                continue;
            }
            let (dp, dr) = (db(peak as f64, want_peak as f64), db(rms, want_rms));
            println!("{name} mode {mode}: peak {peak} ({dp:+.2} dB), RMS {rms:.1} ({dr:+.2} dB), {full} samples at full scale");
            // loudness is the RMS: within 1 dB. The peak may not rise by more than 1 dB nor clip
            // anew; it may fall further, since the lead's 2:1 FM is square-ish (more RMS for its
            // peak than before: about 3.5 dB of crest factor less, so no gain keeps both within
            // 1 dB)
            if dp > 1.0 || dr.abs() > 1.0 || full > want_full {
                bad.push(format!(
                    "{name} mode {mode}: peak {peak} against {want_peak}, RMS {rms:.1} against {want_rms:.1}, \
                     full scale {full} against {want_full}"
                ));
            }
        }
    }
    assert!(bad.is_empty(), "loudness off by more than 1 dB, or new clipping: {bad:#?}");
}

// ---- the instruments ---------------------------------------------------------------------------

/// The analysis rate: 1 s is 44,100 samples.
const SR: u32 = 44_100;
/// Samples per 20 ms frame at SR.
const FRAME: usize = 882;

fn ms(x: f64) -> usize {
    (x / 1000.0 * SR as f64).round() as usize
}

/// A frame holding voice 0 at `hz` (0: off), level 255, with `role`.
fn held(hz: f64, role: u8) -> SndFrame {
    let mut fr = SndFrame { confident: 1, ..Default::default() };
    if hz > 0.0 {
        fr.tone[0] = SndVoice { period_q8: period_q8(hz), level: 255 };
    }
    fr.role[0] = role;
    fr
}

/// `frames` frames of `fr` through `fm`, at SR.
fn render(fm: &mut SndFm, fr: &SndFrame, frames: usize) -> Vec<i16> {
    (0..frames).flat_map(|_| fm.render(fr, FRAME)).collect()
}

/// One held note of `role` at `hz` for `secs` seconds, from a fresh synth.
fn note(role: u8, hz: f64, secs: f64) -> Vec<i16> {
    render(&mut SndFm::new(SR, 1), &held(hz, role), (secs * 50.0).round() as usize)
}

/// Rising zero crossings (interpolated, in samples) after a dip below half the negative peak.
fn crossings(s: &[i16]) -> Vec<f64> {
    let peak = s.iter().map(|&x| (x as i32).abs()).max().unwrap();
    let th = peak / 2;
    let (mut armed, mut times) = (false, Vec::new());
    for i in 1..s.len() {
        let (a, b) = (s[i - 1] as f64, s[i] as f64);
        if (s[i] as i32) < -th {
            armed = true;
        }
        if armed && a <= 0.0 && b > 0.0 {
            times.push((i - 1) as f64 + a / (a - b));
            armed = false;
        }
    }
    times
}

/// The Hann-windowed power of `s` summed over the 1 Hz steps of `lo..=hi` Hz.
fn band_power(s: &[f64], lo: f64, hi: f64) -> f64 {
    let n = s.len() as f64;
    let w: Vec<f64> = s.iter().enumerate().map(|(i, &x)| x * (0.5 - 0.5 * (TAU * i as f64 / n).cos())).collect();
    let mut f = lo;
    let mut p = 0.0;
    while f <= hi {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &x) in w.iter().enumerate() {
            let a = TAU * f * i as f64 / SR as f64;
            re += x * a.cos();
            im += x * a.sin();
        }
        p += re * re + im * im;
        f += 1.0;
    }
    p
}

fn to_f64(s: &[i16]) -> Vec<f64> {
    s.iter().map(|&x| x as f64).collect()
}

/// Power of harmonic h relative to the fundamental, in dB (bands of +-3% around each).
fn harmonic_db(s: &[f64], f: f64, h: f64) -> f64 {
    let band = |c: f64| band_power(s, c * 0.97, c * 1.03);
    10.0 * (band(h * f) / band(f)).log10()
}

#[test]
fn lead_has_odd_harmonics_and_vibrato() {
    // 2:1 FM, sin(p + I sin 2p): sidebands at f +- 2kf, the odd harmonics only (D's hollow,
    // square-ish lead). Against a float reference with the settled index 1.3, over 0.5-1 s
    let f = 440.0;
    let s = note(SND_ROLE_LEAD, f, 1.0);
    let tail = to_f64(&s[ms(500.0)..ms(1000.0)]);
    let reference: Vec<f64> = (0..tail.len())
        .map(|i| {
            let p = TAU * f * i as f64 / SR as f64;
            (p + 1.3 * (2.0 * p).sin()).sin()
        })
        .collect();
    for h in [2.0, 3.0] {
        let (got, want) = (harmonic_db(&tail, f, h), harmonic_db(&reference, f, h));
        println!("harmonic {h}: {got:.1} dB (reference {want:.1} dB)");
        if h == 2.0 {
            assert!(got < -40.0 && want < -40.0, "no second harmonic: {got:.1} dB, reference {want:.1} dB");
        } else {
            assert!((got - want).abs() <= 2.0, "harmonic {h}: {got:.1} dB, reference {want:.1} dB");
        }
    }

    // the instantaneous frequency (period by period, from the zero crossings), in cents off f
    let times = crossings(&s);
    let cents: Vec<(f64, f64)> = times
        .windows(2)
        .map(|w| ((w[0] + w[1]) / 2.0 / SR as f64, 1200.0 * (SR as f64 / (w[1] - w[0]) / f).log2()))
        .collect();
    let early: Vec<f64> = cents.iter().filter(|c| c.0 > 0.010 && c.0 < 0.120).map(|c| c.1).collect();
    let swing = early.iter().fold(0f64, |m, &c| m.max(c.abs()));
    println!("before 120 ms: {} periods, largest deviation {swing:.2} cents", early.len());
    assert!(early.len() > 40 && swing < 1.5, "no vibrato before 120 ms ({swing:.2} cents)");

    let late: Vec<(f64, f64)> = cents.iter().copied().filter(|c| c.0 > 0.400).collect();
    // the best-fitting sinusoid: rate by a search over 5-8 Hz, depth its amplitude
    let fit = |hz: f64| {
        let (mut a, mut b) = (0.0, 0.0);
        for &(t, c) in &late {
            a += c * (TAU * hz * t).cos();
            b += c * (TAU * hz * t).sin();
        }
        2.0 * (a * a + b * b).sqrt() / late.len() as f64
    };
    let rate = (500..=800).map(|k| k as f64 / 100.0).max_by(|&x, &y| fit(x).total_cmp(&fit(y))).unwrap();
    let depth = fit(rate);
    let (lo, hi) = late.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c.1), hi.max(c.1)));
    println!("after 400 ms: rate {rate:.2} Hz, depth {depth:.2} cents (range {lo:.2} .. {hi:.2})");
    assert!((rate - 6.5).abs() <= 0.3, "vibrato rate {rate:.2} Hz");
    assert!((depth - 18.0).abs() <= 3.0, "vibrato depth {depth:.2} cents");
    assert!((hi - 18.0).abs() <= 3.0 && (lo + 18.0).abs() <= 3.0, "vibrato swing {lo:.2} .. {hi:.2} cents");
}

/// Peak |sample| of each `w`-sample window of `s`.
fn envelope(s: &[i16], w: usize) -> Vec<i32> {
    s.chunks(w).map(|c| c.iter().map(|&x| (x as i32).abs()).max().unwrap()).collect()
}

#[test]
fn pad_beats_at_detune() {
    // two carriers 4 cents apart, half and half: the envelope falls to nothing once a beat
    let f = 880.0;
    let want = f * (2f64.powf(4.0 / 1200.0) - 1.0);
    let s = note(SND_ROLE_PAD, f, 3.0);
    let w = ms(5.0);
    let env = envelope(&s, w);
    let top = *env.iter().max().unwrap();
    // the dips: runs of windows below 20% of the top, each dip's middle
    let mut dips = Vec::new();
    let mut start = None;
    for (i, &e) in env.iter().enumerate() {
        match (e < top / 5, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                dips.push((a + i - 1) as f64 / 2.0 * w as f64 / SR as f64);
                start = None;
            }
            _ => {}
        }
    }
    println!("dips at {dips:.3?} s");
    assert!(dips.len() >= 4, "the pad beats ({} dips)", dips.len());
    let got = (dips.len() - 1) as f64 / (dips[dips.len() - 1] - dips[0]);
    println!("beat {got:.3} Hz, want {want:.3} Hz");
    assert!((got / want - 1.0).abs() <= 0.10, "beat {got:.3} Hz, want {want:.3} Hz");
}

#[test]
fn envelopes_match_table() {
    // (role, attack ms, decay tau ms, sustain, release ms)
    for (name, role, attack, tau, sus, release) in [
        ("bass", SND_ROLE_BASS, 3.0, 200.0, 0.8, 50.0),
        ("lead", SND_ROLE_LEAD, 4.0, 150.0, 0.8, 50.0),
        ("pad", SND_ROLE_PAD, 10.0, 200.0, 0.85, 100.0),
    ] {
        // sample by sample, reading voice 0's envelope (Q16) after each
        let mut fm = SndFm::new(SR, 1);
        let fr = held(220.0, role);
        let env: Vec<f64> = (0..ms(400.0))
            .map(|_| {
                fm.render(&fr, 1);
                fm.voice_env(0) as f64 / 65536.0
            })
            .collect();
        assert_eq!(fm.voice_role(0), role, "{name}: the voice plays its role");
        let peak = env.iter().position(|&e| e >= 1.0).expect("reaches its peak") as f64 * 1000.0 / SR as f64;
        let at = env[ms(attack + tau)];
        let want = sus + (1.0 - sus) / std::f64::consts::E;
        println!("{name}: peak at {peak:.2} ms, at attack + tau {at:.4} (want {want:.4})");
        assert!((peak - attack).abs() <= 0.5, "{name}: peak at {peak:.2} ms, attack {attack} ms");
        assert!((at / want - 1.0).abs() <= 0.03, "{name}: {at:.4} at attack + tau, want {want:.4}");

        // stop it: a release, then exact silence after the release time
        let s = render(&mut fm, &held(0.0, role), 10);
        let last = s.iter().rposition(|&x| x != 0).expect("released, not cut") as f64 * 1000.0 / SR as f64;
        println!("{name}: silent from {last:.2} ms after the stop");
        assert!((last - release).abs() <= 1.0, "{name}: silent from {last:.2} ms, release {release} ms");
    }
}

#[test]
fn role_latched_at_note_start() {
    let bass = |hz: f64| held(hz, SND_ROLE_BASS);
    let lead = |hz: f64| held(hz, SND_ROLE_LEAD);
    // a note started as bass, then frames saying lead: the same note, a glide (under 1/32),
    // then a new note
    let seq = [(bass(220.0), 5), (lead(220.0), 10), (lead(224.0), 10), (lead(260.0), 10)];
    // the same with bass throughout up to the new note
    let all_bass = [(bass(220.0), 5), (bass(220.0), 10), (bass(224.0), 10)];
    let mut fm = SndFm::new(SR, 1);
    let mut got = Vec::new();
    let mut roles = Vec::new();
    for (fr, n) in seq {
        got.extend(render(&mut fm, &fr, n));
        roles.push(fm.voice_role(0));
    }
    let mut fm = SndFm::new(SR, 1);
    let want: Vec<i16> = all_bass.iter().flat_map(|(fr, n)| render(&mut fm, fr, *n)).collect();
    println!("latched roles: {roles:?}");
    assert_eq!(roles, [SND_ROLE_BASS, SND_ROLE_BASS, SND_ROLE_BASS, SND_ROLE_LEAD]);
    assert!(got[..want.len()] == want[..], "held and glided, the note keeps the bass patch");
    assert!(got[want.len()..] != render(&mut SndFm::new(SR, 1), &bass(260.0), 10)[..], "the new note is not bass");

    // a note event in a frame takes the frame's role for its voice
    let mut fm = SndFm::new(SR, 1);
    render(&mut fm, &bass(220.0), 5);
    let mut fr = lead(220.0);
    fr.ev[0] = SndEvent { t: 1000, period_q8: period_q8(330.0), voice: 0, level: 255 };
    fr.n_ev = 1;
    fm.render(&fr, FRAME);
    assert_eq!(fm.voice_role(0), SND_ROLE_LEAD, "the event's new note takes the frame's role");

    // SND_ROLE_AUTO plays as lead
    let auto = note(SND_ROLE_AUTO, 440.0, 0.5);
    assert!(auto == note(SND_ROLE_LEAD, 440.0, 0.5), "auto plays the lead patch");
}

fn rms(s: &[i16]) -> f64 {
    (s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
}

#[test]
fn drum_has_tone_and_noise() {
    let drum = |level: u8, secs: f64| {
        let fr = SndFrame { confident: 1, drum: level, ..Default::default() };
        render(&mut SndFm::new(SR, 1), &fr, (secs * 50.0) as usize)
    };
    let s = drum(255, 1.0);
    let head = to_f64(&s[..ms(100.0)]);
    // the tone: the largest 1 Hz step between 100 and 400 Hz
    let power = |f: f64| band_power(&head, f, f);
    let peak = (100..=400).map(|f| f as f64).max_by(|&a, &b| power(a).total_cmp(&power(b))).unwrap();
    println!("tone peak at {peak} Hz");
    assert!((peak - 190.0).abs() <= 5.0, "a tone at 190 Hz: the peak at {peak} Hz");
    // the noise: broadband, a good part of the energy above 2 kHz
    let (hi, all) = (band_power(&head, 2000.0, 10000.0), band_power(&head, 20.0, 10000.0));
    println!("energy above 2 kHz: {:.0}%", 100.0 * hi / all);
    assert!(hi > all * 0.2, "broadband noise ({:.0}% above 2 kHz)", 100.0 * hi / all);
    // the decay: the noise's tau 70 ms (the tone, tau 30 ms, gone by 100 ms)
    let (a, b) = (rms(&s[ms(100.0)..ms(140.0)]), rms(&s[ms(240.0)..ms(280.0)]));
    let want = (-140.0f64 / 70.0).exp();
    println!("RMS 100-140 ms {a:.0}, 240-280 ms {b:.0}: ratio {:.3} (want {want:.3})", b / a);
    assert!((b / a / want - 1.0).abs() <= 0.2, "decay ratio {:.3}, want {want:.3}", b / a);
    // the strength follows the drum level
    let half = drum(128, 0.2);
    let r = rms(&half[..ms(100.0)]) / rms(&s[..ms(100.0)]);
    println!("level 128 against 255: RMS ratio {r:.3}");
    assert!((r - 128.0 / 255.0).abs() < 0.05, "strength ratio {r:.3}");
}
