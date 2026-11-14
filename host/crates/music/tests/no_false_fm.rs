// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! No FM on sound that is not music (Review Focus 3): zero confident decisions on the test
//! programs (the border tone for 60 s, the scroll test, the tape test), on the pack's games with
//! effects and no music (ZX Dino, A World of One-Way, ZX Sopwith) driven by their `games.toml`
//! demos for 36 s (with presses that make sound added, and a minimum of frames with sound), on
//! Castaway and Escape from Colony 8 in play after their demos start, on generated effect-like
//! signals: single pitch sweeps up and down of 50-300 ms, white-noise bursts, two-tone
//! alternating sirens and a constant tone for 3 s, each as a square wave (speaker flips) and as
//! short pulses (the pin-pulse style), and on sweeps and sirens right after time-slot and
//! speaker-flip music. In all of them no confident frame plays FM silence over sound.

use fmzx_pack::manifest::{encode_keymap, input_index, parse_manifest, BUTTON_COUNT};
use music::synthetic::{Stream, FRAME_T};
use music::truth::Engine;
use music::{capture, decode, decode_stream, repo, test_sna, voices_hz, Frame, RATE};
use zx_sys::{zx_edge, Beeper, KeyMapper, KmInput, Machine, SndFrame};

/// The decisions about `frames`.
fn decide(frames: &[Frame]) -> Vec<SndFrame> {
    decode(frames.iter().map(|f| (f.edges.as_slice(), f.start_level)))
}

/// Frames with a confident decision in `range`, and a short description of the first ones.
fn confident(dec: &[SndFrame], range: std::ops::Range<usize>) -> (usize, String) {
    let conf: Vec<usize> = range.filter(|&i| dec[i].confident != 0).collect();
    let first: Vec<String> = conf.iter().take(6).map(|&i| format!("{i}: {:?} drum {}", voices_hz(&dec[i]), dec[i].drum)).collect();
    (conf.len(), first.join("; "))
}

/// Frames in `range` that have any speaker edge.
fn sounding(frames: &[Frame], range: std::ops::Range<usize>) -> usize {
    range.filter(|&i| frames[i].edges.iter().any(|e| (e.level ^ frames[i].start_level) & 2 != 0)).count()
}

/// Each frame's speaker (EAR) edges.
fn speaker_edges(frames: &[Frame]) -> Vec<usize> {
    frames
        .iter()
        .map(|f| {
            let mut ear = f.start_level & 2;
            f.edges
                .iter()
                .filter(|e| {
                    let changed = e.level & 2 != ear;
                    ear = e.level & 2;
                    changed
                })
                .count()
        })
        .collect()
}

/// Each frame's speaker edges in a generated stream (every edge of a [`stream`] changes EAR).
fn stream_edges(s: &Stream) -> Vec<usize> {
    s.frames.iter().map(Vec::len).collect()
}

/// No confident decision in `range` (sound that is not music) plays FM silence over sound: a
/// confident frame with 8 speaker edges or more and no drum reports a voice (the mixer would
/// otherwise play FM silence instead of the beeper's sound). Inside music such a frame can be a
/// note ending or starting in it, so the ranges checked are the effects, the games in play after
/// their music and the test programs.
fn no_silent_fm(what: &str, edges: &[usize], dec: &[SndFrame], range: std::ops::Range<usize>) {
    let bad = silent_fm(edges, dec, range);
    assert!(bad.is_empty(), "{what}: confident silent frames over sound (frame, edges): {bad:?}");
}

/// The frames [`no_silent_fm`] rejects: (frame, speaker edges).
fn silent_fm(edges: &[usize], dec: &[SndFrame], range: std::ops::Range<usize>) -> Vec<(usize, usize)> {
    range.filter(|&i| dec[i].confident != 0 && dec[i].drum == 0 && voices_hz(&dec[i]).is_empty() && edges[i] >= 8).map(|i| (i, edges[i])).collect()
}

fn gate(what: &str, frames: &[Frame], dec: &[SndFrame], range: std::ops::Range<usize>) {
    let (n, first) = confident(dec, range.clone());
    let style = |s: u8| range.clone().filter(|&i| dec[i].style == s && !voices_hz(&dec[i]).is_empty()).count();
    println!(
        "{what}: frames {}..{}, {} with sound; voices reported in {} (flips {}, slots {}, pulses {}); confident {n}",
        range.start,
        range.end,
        sounding(frames, range.clone()),
        range.clone().filter(|&i| !voices_hz(&dec[i]).is_empty()).count(),
        style(zx_sys::SND_STYLE_FLIPS),
        style(zx_sys::SND_STYLE_SLOTS),
        style(zx_sys::SND_STYLE_PULSES),
    );
    assert_eq!(n, 0, "{what}: FM in {n} frames: {first}");
    no_silent_fm(what, &speaker_edges(frames), dec, range);
}

#[test]
fn border_tone_60s() {
    let fr = capture(&mut test_sna("border.sna"), &mut Beeper::new(RATE.0, RATE.1), 3000);
    let dec = decide(&fr);
    gate("border.sna", &fr, &dec, 0..dec.len());
}

#[test]
fn scroll_test() {
    let fr = capture(&mut test_sna("scroll.sna"), &mut Beeper::new(RATE.0, RATE.1), 1000);
    let dec = decide(&fr);
    gate("scroll.sna", &fr, &dec, 0..dec.len());
}

#[test]
fn tape_test() {
    // autostart types LOAD "" (the ROM's key clicks), the program waits, loads its screen and idles
    let tap = std::fs::read(repo().join("games/test/tape.tap")).expect("tape.tap");
    let mut m = Machine::new();
    m.tape_insert(&tap).expect("inserts");
    m.tape_autostart();
    let fr = capture(&mut m, &mut Beeper::new(RATE.0, RATE.1), 1000);
    let dec = decide(&fr);
    gate("tape.tap", &fr, &dec, 0..dec.len());
}

/// One step of a `demo` script: inputs (KmInput bits) held from `at` for `hold` ms.
struct Step {
    button: u32,
    note: u32,
    at: u64,
    hold: u64,
}

/// Parses `demo = "<input>@<ms>[+<hold>],..."` (hold 100 ms when not given), as the firmware
/// gate does.
fn parse_demo(s: &str) -> Vec<Step> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (key, when) = p.split_once('@').expect("key@ms");
            let (at, hold) = when.split_once('+').map_or((when, 100), |(a, h)| (a, h.parse().expect("hold")));
            let i = input_index(key).expect("input");
            let (button, note) = if i < BUTTON_COUNT { (1 << i, 0) } else { (0, 1 << (i - BUTTON_COUNT)) };
            Step { button, note, at: at.parse().expect("ms"), hold }
        })
        .collect()
}

/// A game run under its demo ([`demo_run`]).
struct Run {
    frames: Vec<Frame>,
    /// The game's music engine (`truth.rs`) runs at the frame's end.
    engine: Vec<bool>,
    /// Screen lines that changed in the frame.
    lines: Vec<usize>,
}

/// Game `id` of `games/games.toml` from its start for `frames` frames (20 ms each), with its
/// `demo` keys pressed through its key map as the firmware does, every key `delay_ms` later and
/// the steps `extra` (the `demo` syntax) added; with the music engine and the screen activity.
fn demo_run(id: &str, frames: usize, delay_ms: u64, extra: &str) -> Run {
    let dir = repo().join("games");
    let manifest = parse_manifest(&std::fs::read_to_string(dir.join("games.toml")).expect("games.toml")).expect("parses");
    let game = manifest.game.iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no game {id}"));
    let data = std::fs::read(dir.join(&game.file)).expect("game file");
    let mut m = Machine::new();
    if game.file.ends_with(".tap") {
        m.tape_insert(&data).expect("inserts");
        m.tape_autostart();
    } else {
        m.load_sna(&data).expect("loads");
    }
    let mut km = KeyMapper::new();
    km.set_map(&encode_keymap(&game.keys).expect("key map"));
    let mut steps = parse_demo(game.demo.as_deref().expect("a demo"));
    steps.extend(parse_demo(extra));
    let engine = Engine::of_file(&game.file);
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut run = Run { frames: Vec::new(), engine: Vec::new(), lines: Vec::new() };
    let mut screen: Vec<[u8; 256]> = (0..192).map(|y| m.screen_line(y)).collect();
    for k in 0..frames {
        let ms = (k as u64 * 20).wrapping_sub(delay_ms);
        {
            let mut inp = KmInput::default();
            for s in steps.iter().filter(|s| s.at <= ms && ms < s.at + s.hold) {
                inp.buttons |= s.button;
                inp.notes |= s.note;
            }
            let (rows, kemp) = km.apply(&inp);
            m.set_keys(rows);
            m.set_kempston(kemp);
            run.frames.push(capture(&mut m, &mut beeper, 1).pop().unwrap());
            run.engine.push(engine.is_some_and(|e| e.read(&m).is_some()));
            let now: Vec<[u8; 256]> = (0..192).map(|y| m.screen_line(y)).collect();
            run.lines.push((0..192).filter(|&y| now[y] != screen[y]).count());
            screen = now;
        }
    }
    run
}

#[test]
fn effects_games_demo() {
    // effects, no music: the first 36 s of each demo, with presses added where the demo makes
    // little sound: ZX Sopwith a shot every 0.6 s from 7 s (its sounds are its shots, bombs and
    // crashes, a frame or two each: 24 frames with sound under the demo alone, 33 with the shots),
    // A World of One-Way its shop from the menu (it moves in silence: 5 frames, 9 with the shop
    // clicks). Each must make at least `min` frames with sound for the gate to mean anything
    let shots: Vec<String> = (0..48).map(|k| format!("note.C5@{}+100", 7_000 + 600 * k)).collect();
    let shots = shots.join(", ");
    for (id, extra, min) in [("dino", "", 70), ("oneway", "button.OCT+@9000, button.OCT+@9500, button.OCT+@11000", 8), ("sopwith", shots.as_str(), 30)] {
        let fr = demo_run(id, 1800, 0, extra).frames;
        let dec = decide(&fr);
        let n = sounding(&fr, 0..fr.len());
        println!("{id}: {n} frames with sound, {} speaker edges", speaker_edges(&fr).iter().sum::<usize>());
        assert!(n >= min, "{id}: {n} frames with sound, the gate needs {min}");
        gate(id, &fr, &dec, 0..dec.len());
    }
}

/// The pack's game `id` with music on its title, its demo's keys `delay_ms` later: the music
/// plays in FM until its engine stops for good (the keys start the game, which is then played:
/// the screen changes in most seconds); from there no FM sound at all (no confident decision
/// with a voice or a drum), and after the decoder's longest rest (`REST_FRAMES`, 1 s of FM
/// silence the music may still be in) no confident decision at all.
fn in_play(id: &str, delay_ms: u64) {
    const REST_FRAMES: usize = 50;
    let frames = 1800 + delay_ms as usize / 20;
    let run = demo_run(id, frames, delay_ms, "");
    let (fr, engine) = (&run.frames, &run.engine);
    let dec = decide(fr);
    let stop = engine.iter().rposition(|&e| e).expect("the title music plays") + 1;
    // (the frame the engine stops in still holds its last note's tail)
    no_silent_fm(&format!("{id} in play"), &speaker_edges(fr), &dec, stop + 1..dec.len());
    let first = engine.iter().position(|&e| e).unwrap();
    let title = (first..stop).filter(|&i| dec[i].confident != 0).count();
    println!("{id}: title music from frame {first} to {stop}: confident in {title} frames");
    assert!(title * 2 > stop - first, "{id}: the title music plays in FM");
    let sound: Vec<usize> = (stop..dec.len()).filter(|&i| dec[i].confident != 0 && (!voices_hz(&dec[i]).is_empty() || dec[i].drum != 0)).collect();
    let rest = (stop..stop + REST_FRAMES).filter(|&i| dec[i].confident != 0).count();
    println!("{id}: from the music's end at frame {stop}: {} confident decisions with FM sound; {rest} confident silent frames in the first second", sound.len());
    assert!(sound.is_empty(), "{id} in play: FM sound in frames {sound:?}");
    // the game is played: the screen changes in most seconds after the music
    let secs: Vec<usize> = run.lines[stop..].chunks(50).map(|c| c.iter().sum()).collect();
    let active = secs.iter().filter(|&&l| l >= 8).count();
    println!("{id}: screen lines changed per second in play: {secs:?}");
    assert!(active * 10 >= secs.len() * 8, "{id}: the game is not played ({active} of {} seconds active)", secs.len());
    gate(&format!("{id} in play"), fr, &dec, stop + REST_FRAMES..dec.len());
}

#[test]
fn castaway_in_play() {
    // the title tune until the demo's keys (12 s, 14.5 s); then the game, which is silent
    in_play("castaway", 0);
}

#[test]
fn colony8_in_play() {
    // the title's music, then the game's effects; the title takes keys later here than on the
    // device (its demo's 5 s and 7 s keys leave it in the menu), so the keys come 5 s later
    in_play("colony8", 5_000);
}

/// A sound as a list of (start, end) T-states of the speaker's high spans.
type Spans = Vec<(u64, u64)>;

/// A square wave whose frequency at time t (T-states from `a`) is `hz(t)`, from `a` to `b`.
fn square(a: u64, b: u64, hz: &dyn Fn(f64) -> f64) -> Spans {
    let mut v = Vec::new();
    let mut t = a as f64;
    let mut high = true;
    let mut start = a;
    while t < b as f64 {
        let half = 3_500_000.0 / hz(t - a as f64) / 2.0;
        t += half;
        if high {
            v.push((start, (t as u64).min(b)));
        } else {
            start = t as u64;
        }
        high = !high;
    }
    v
}

/// Short pulses (200 T) at the frequency `hz(t)`, from `a` to `b`.
fn pulses(a: u64, b: u64, hz: &dyn Fn(f64) -> f64) -> Spans {
    let mut v = Vec::new();
    let mut t = a as f64;
    while t < b as f64 {
        v.push((t as u64, t as u64 + 200));
        t += 3_500_000.0 / hz(t - a as f64);
    }
    v
}

/// Random edges `gap` T apart (gap from `lo` to `hi`), from `a` to `b`.
fn noise(a: u64, b: u64, lo: u32, hi: u32, seed: &mut u32) -> Spans {
    let mut v = Vec::new();
    let mut t = a;
    let mut next = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        lo + *seed % (hi - lo)
    };
    while t < b {
        let h = next() as u64;
        v.push((t, t + h));
        t += h + next() as u64;
    }
    v
}

/// High spans to a stream of `frames` frames.
fn stream(mut spans: Spans, frames: usize) -> Stream {
    spans.sort();
    let end = frames as u64 * FRAME_T;
    let mut out = vec![Vec::new(); frames];
    let mut push = |t: u64, level: u8| {
        if t < end {
            out[(t / FRAME_T) as usize].push(zx_edge { tstate: (t % FRAME_T) as u32, level, pc: 0 });
        }
    };
    let mut cur: Option<(u64, u64)> = None;
    for (a, b) in spans {
        match cur {
            Some((c0, c1)) if a <= c1 => cur = Some((c0, c1.max(b))),
            Some((c0, c1)) => {
                push(c0, 2);
                push(c1, 0);
                cur = Some((a, b));
            }
            None => cur = Some((a, b)),
        }
    }
    if let Some((c0, c1)) = cur {
        push(c0, 2);
        push(c1, 0);
    }
    Stream { frames: out, start_level: 0, truth: vec![Some(Vec::new()); frames], drum: vec![false; frames], silent: vec![false; frames] }
}

/// A sound generator: from, to, the frequency at a time from its start.
type Gen = fn(u64, u64, &dyn Fn(f64) -> f64) -> Spans;

/// The effect-like signals: (name, its high spans from T 0, its length in frames).
fn effects() -> Vec<(String, Spans, usize)> {
    let ms = |x: f64| (x * 3_500.0) as u64;
    let mut v = Vec::new();
    let mut seed = 0x1234_5678u32;
    for (kind, gen) in [("square", square as Gen), ("pulses", pulses as Gen)] {
        // single sweeps, exponential in pitch, up and down, alone after 200 ms of silence
        for d in [50.0, 100.0, 200.0, 300.0] {
            for (f0, f1) in [(200.0f64, 2_000.0f64), (2_000.0, 200.0), (400.0, 1_600.0), (1_600.0, 400.0), (800.0, 1_000.0), (1_000.0, 800.0), (300.0, 600.0)] {
                let len = ms(d) as f64;
                let a = ms(200.0);
                let sp = gen(a, a + ms(d), &move |t: f64| f0 * (f1 / f0).powf(t / len));
                v.push((format!("{kind} sweep {f0}->{f1} Hz in {d} ms"), sp, 40));
            }
        }
        // the same sweep again and again (a jump sound, shots), 0.5 s apart
        for (f0, f1, d) in [(300.0f64, 1_200.0f64, 150.0), (1_500.0, 300.0, 80.0), (600.0, 900.0, 250.0)] {
            let len = ms(d) as f64;
            let mut sp = Vec::new();
            for k in 0..8 {
                let a = ms(200.0 + 500.0 * k as f64);
                sp.extend(gen(a, a + ms(d), &move |t: f64| f0 * (f1 / f0).powf(t / len)));
            }
            v.push((format!("{kind} sweep {f0}->{f1} Hz in {d} ms, 8 times"), sp, 220));
        }
        // two-tone sirens: tones of 20-60 ms, 3 s
        for (lo, hi) in [(600.0, 800.0), (700.0, 1_050.0), (440.0, 587.0)] {
            for tone in [20.0, 30.0, 40.0, 45.0, 60.0] {
                let mut sp = Vec::new();
                let mut k = 0;
                while (k as f64) * tone < 3_000.0 {
                    let a = ms(200.0 + k as f64 * tone);
                    let f = if k % 2 == 0 { lo } else { hi };
                    sp.extend(gen(a, a + ms(tone), &move |_| f));
                    k += 1;
                }
                v.push((format!("{kind} siren {lo}/{hi} Hz, {tone} ms tones"), sp, 170));
            }
        }
        // a constant tone for 3 s
        for f in [440.0, 1_000.0, 2_000.0] {
            v.push((format!("{kind} constant {f} Hz for 3 s"), gen(ms(200.0), ms(3_200.0), &move |_| f), 170));
        }
    }
    // white-noise bursts of 30-500 ms: dense (too many edges to decode) and sparse
    for (lo, hi) in [(20, 200), (100, 600), (300, 3_000), (1_000, 6_000)] {
        for d in [30.0, 100.0, 500.0] {
            let mut sp = Vec::new();
            for k in 0..4 {
                let a = ms(200.0 + 700.0 * k as f64);
                sp.extend(noise(a, a + ms(d), lo, hi, &mut seed));
            }
            v.push((format!("noise {lo}-{hi} T in {d} ms bursts"), sp, 150));
        }
    }
    v
}

#[test]
fn effect_signals() {
    let mut bad = Vec::new();
    for (name, spans, frames) in effects() {
        let s = stream(spans, frames);
        let dec = decode_stream(&s);
        let voiced = dec.iter().filter(|f| !voices_hz(f).is_empty()).count();
        let (n, first) = confident(&dec, 0..dec.len());
        no_silent_fm(&name, &stream_edges(&s), &dec, 0..dec.len());
        println!("{name}: voices in {voiced} of {frames} frames, confident {n}");
        if n > 0 {
            bad.push(format!("{name}: FM in {n} frames: {first}"));
        }
    }
    assert!(bad.is_empty(), "effects played in FM:\n{}", bad.join("\n"));
}



/// A two-tone siren of `tone_ms` tones for `secs` seconds after 200 ms of silence, as a square wave.
fn siren(lo: f64, hi: f64, tone_ms: f64, secs: f64) -> Spans {
    let ms = |x: f64| (x * 3_500.0) as u64;
    let mut sp = Vec::new();
    let mut k = 0;
    while (k as f64) * tone_ms < secs * 1000.0 {
        let a = ms(200.0 + k as f64 * tone_ms);
        let f = if k % 2 == 0 { lo } else { hi };
        sp.extend(square(a, a + ms(tone_ms), &move |_| f));
        k += 1;
    }
    sp
}

#[test]
fn slow_siren_bounded() {
    // a siren whose tones outlast the look-ahead (60 ms) cannot be told from a tune's first two
    // notes before its first return; from its second return (A B A B) it is an alternation, and
    // no music until a new pitch: at most its first two tones play in FM
    for tone in [80.0, 100.0, 150.0, 250.0, 500.0] {
        let s = stream(siren(600.0, 800.0, tone, 5.0), 270);
        let dec = decode_stream(&s);
        let conf: Vec<usize> = (0..dec.len()).filter(|&i| dec[i].confident != 0).collect();
        no_silent_fm("slow siren", &stream_edges(&s), &dec, 0..dec.len());
        let two = (2.0 * tone / 20.0) as usize;
        println!("siren of {tone} ms tones for 5 s: confident in {} frames ({:?}..{:?}), two tones are {two} frames", conf.len(), conf.first(), conf.last());
        assert!(conf.len() <= two, "siren of {tone} ms tones: FM in {} frames", conf.len());
        assert!(conf.last().is_none_or(|&l| l < 10 + 3 * two), "siren of {tone} ms tones: FM at frame {:?}", conf.last());
    }
}

/// `music`, then `gap` silent frames, then `effect`; returns the stream and the effect's first
/// frame.
fn after_music(mut s: Stream, gap: usize, effect: Stream) -> (Stream, usize) {
    s.frames.extend((0..gap).map(|_| Vec::new()));
    s.truth.extend((0..gap).map(|_| Some(Vec::new())));
    s.drum.extend((0..gap).map(|_| false));
    s.silent.extend((0..gap).map(|_| true));
    let n = s.frames.len();
    (s.then(effect), n)
}

/// Time-slot music (the slots tune, ending with its loop's last notes), then `gap` silent
/// frames, then `effect`; returns the stream and the effect's first frame.
fn after_slots_music(gap: usize, effect: Stream) -> (Stream, usize) {
    after_music(music::synthetic::slots_loop(&music::synthetic::slots_tune()), gap, effect)
}

/// Speaker-flip music (the XOR tune: long notes, detuned unisons and chords), then `gap` silent
/// frames, then `effect`.
fn after_flips_music(gap: usize, effect: Stream) -> (Stream, usize) {
    after_music(music::synthetic::xor_loop(&music::synthetic::xor_tune(0)), gap, effect)
}

/// An effect played after music: its name, high spans (from T 0), length in frames, and for a
/// siren its tone length in frames (0 otherwise).
type Effect = (String, Spans, usize, f64);

/// The effects played after music: single sweeps of 50-300 ms (from T 0), the same sweep again
/// and again 0.5 s apart, and two-tone sirens of 20-60 ms tones for 3 s, as square waves.
fn music_effects() -> Vec<Effect> {
    let ms = |x: f64| (x * 3_500.0) as u64;
    let mut effects: Vec<Effect> = Vec::new();
    for d in [50.0, 150.0, 300.0] {
        for (f0, f1) in [(400.0f64, 1_600.0f64), (1_600.0, 400.0), (800.0, 1_000.0), (1_000.0, 800.0), (500.0, 600.0), (600.0, 500.0)] {
            let len = ms(d) as f64;
            effects.push((format!("sweep {f0}->{f1} Hz in {d} ms"), square(0, ms(d), &move |t: f64| f0 * (f1 / f0).powf(t / len)), 20, 0.0));
        }
    }
    // the same sweep again and again, 0.5 s apart: the later ones come after the loop is forgotten
    for (f0, f1, d) in [(300.0f64, 1_200.0f64, 150.0), (1_500.0, 300.0, 80.0), (600.0, 900.0, 250.0), (800.0, 1_000.0, 300.0), (1_000.0, 800.0, 300.0), (500.0, 600.0, 300.0)] {
        let len = ms(d) as f64;
        let mut sp = Vec::new();
        for k in 0..8 {
            let a = ms(500.0 * k as f64);
            sp.extend(square(a, a + ms(d), &move |t: f64| f0 * (f1 / f0).powf(t / len)));
        }
        effects.push((format!("sweep {f0}->{f1} Hz in {d} ms, 8 times"), sp, 210, 0.0));
    }
    for tone in [20.0, 30.0, 40.0, 45.0, 60.0] {
        for (lo, hi) in [(600.0, 800.0), (440.0, 587.0)] {
            let mut sp = Vec::new();
            let mut k = 0;
            while (k as f64) * tone < 3_000.0 {
                let f = if k % 2 == 0 { lo } else { hi };
                sp.extend(square(ms(k as f64 * tone), ms((k + 1) as f64 * tone), &move |_| f));
                k += 1;
            }
            effects.push((format!("siren {lo}/{hi} Hz, {tone} ms tones"), sp, 160, tone / 20.0));
        }
    }
    effects
}

/// Every effect of [`music_effects`] after the music `join` puts before it, `gap` frames later:
/// the music plays in FM, the effect not at all, and no confident frame plays FM silence over
/// sound. `first_tone`: a siren's first tone may play in FM in the frames decided before its
/// first return (A B A) is in the look-ahead, none of them after the tone (see
/// [`effects_after_flips_music`]).
fn effects_after(what: &str, join: fn(usize, Stream) -> (Stream, usize), gaps: &[usize], first_tone: bool) {
    let mut bad = Vec::new();
    let mut allowed = 0;
    for (name, spans, frames, tone) in music_effects() {
        for &gap in gaps {
            let (s, n) = join(gap, stream(spans.clone(), frames));
            let dec = decode_stream(&s);
            let music = dec[..n - gap].iter().filter(|f| f.confident != 0).count();
            assert!(music > (n - gap) / 2, "the music before it plays in FM ({music} of {})", n - gap);
            let silent = silent_fm(&stream_edges(&s), &dec, n..dec.len());
            let voiced = (n..dec.len()).filter(|&i| !voices_hz(&dec[i]).is_empty()).count();
            let (c, first) = confident(&dec, n..dec.len());
            // a siren's first tone, decided before the tone after the next can be seen (2 tones and
            // a frame from its start, less the 3 frames of look-ahead)
            let judged = n + (2.0 * tone + 1.0 - 3.0).max(0.0).ceil() as usize;
            let early = if first_tone && tone > 0.0 { (n..judged.min(n + tone.ceil() as usize)).filter(|&i| dec[i].confident != 0).count() } else { 0 };
            println!("{name} after {gap} silent frames: voices in {voiced} frames, confident {c} ({early} in the first tone before its return is seen), {} silent over sound", silent.len());
            allowed += early;
            if c > early || !silent.is_empty() {
                bad.push(format!("{name} after {gap} silent frames: FM in {c} frames ({} silent over sound): {first}", silent.len()));
            }
        }
    }
    println!("{what}: {allowed} confident frames in sirens' first tones");
    assert!(bad.is_empty(), "effects after {what} played in FM ({} cases):\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn effects_after_slots_music() {
    // a sweep or a siren right after time-slot music (or 20 frames later) stays on the beeper,
    // through the 25 frames the loop is held (the time-slot path) and after the loop is forgotten
    // (speaker flips); the music's pitch changes do not carry over to it
    effects_after("time-slot music", after_slots_music, &[0, 20], false);
}

#[test]
fn effects_after_flips_music() {
    // the same effects right after speaker-flip music, 5 or 20 frames later, inside the 2 s its
    // pitch changes keep the music context: a sweep's steps (glides) are not notes and end the
    // context, a frame with sound that no voice of the notes around it explains is no rest, and
    // a quick alternation (A B A) is no music. A siren's first tone is the exception: within the
    // music's context its first tone cannot be told from the tune's next note until its first
    // return is in the 60 ms look-ahead, so the frames of a 40-60 ms tone decided before that
    // (1, 2, 3 for 40, 45, 60 ms) play in FM; tones of 20-30 ms (the return seen in time) not at
    // all, and nothing after the first tone
    effects_after("speaker-flip music", after_flips_music, &[0, 5, 20], true);
}

#[test]
fn jingle_after_slots_music() {
    // a jingle of three discrete notes after time-slot music is music: FM is allowed for it, but
    // every note it plays must be right. After the music the time-slot decoder (its loop held for
    // 25 frames) decides the jingle's frames and plays them on the beeper; the jingle alone
    // shows the decoder reads each note's pitch right
    let ms = |x: f64| (x * 3_500.0) as u64;
    let midi = music::score::midi_of;
    // decoded voices (confident or not) in `range`: every one within half a semitone of a true
    // pitch in the frame or a neighbour; returns, per jingle note, the frames that decode it
    let check = |what: &str, s: &Stream, dec: &[SndFrame], range: std::ops::Range<usize>, notes: &[f64; 3]| -> [usize; 3] {
        let mut per_note = [0usize; 3];
        for i in range {
            let truth: Vec<f64> = (i.saturating_sub(1)..=(i + 1).min(s.truth.len() - 1)).flat_map(|j| s.truth[j].clone().unwrap_or_default()).collect();
            for v in voices_hz(&dec[i]) {
                assert!(truth.iter().any(|&t| (midi(t) - midi(v)).abs() < 0.5), "{what}: frame {i} decodes {v:.1} Hz, true {truth:?}");
                if let Some(k) = notes.iter().position(|&n| (midi(n) - midi(v)).abs() < 0.5) {
                    per_note[k] += 1;
                }
            }
        }
        per_note
    };
    for (notes, note_ms) in [([1_046.5, 1_318.5, 1_568.0], 80.0), ([523.25, 659.26, 783.99], 120.0), ([784.0, 988.0, 1_175.0], 60.0)] {
        let mut sp = Vec::new();
        for (k, &f) in notes.iter().enumerate() {
            sp.extend(square(ms(k as f64 * note_ms), ms((k + 1) as f64 * note_ms - 5.0), &move |_| f));
        }
        let frames = (3.0 * note_ms / 20.0) as usize + 10;
        let mut effect = stream(sp, frames);
        effect.truth = (0..frames)
            .map(|i| {
                let mid = (i as f64 + 0.5) * 20.0;
                let k = (mid / note_ms) as usize;
                Some(if k < 3 && mid < (k + 1) as f64 * note_ms - 5.0 { vec![notes[k]] } else { vec![] })
            })
            .collect();
        // the jingle alone: the decoder reads each of its notes right in 2 frames or more
        let alone = decode_stream(&effect);
        let per_note = check("jingle alone", &effect, &alone, 0..frames, &notes);
        println!("jingle {notes:?} of {note_ms} ms notes alone: frames decoding each note {per_note:?}");
        assert!(per_note.iter().all(|&c| c >= 2), "jingle {notes:?} alone: frames decoding each note {per_note:?}");
        for gap in [0, 20] {
            let (s, n) = after_slots_music(gap, effect.clone());
            let dec = decode_stream(&s);
            let sc = music::score::score(&s.truth[n..], &music::played_hz(&dec[n..]));
            let (c, _) = confident(&dec, n..dec.len());
            // every frame of the jingle with sound reached the decoder, which took it as time slots
            // (the loop is held), and any voice it reports is right
            let sounding: Vec<usize> = (n..dec.len()).filter(|&i| !s.frames[i].is_empty()).collect();
            let slots = sounding.iter().filter(|&&i| dec[i].style == zx_sys::SND_STYLE_SLOTS).count();
            let per_note = check("jingle after music", &s, &dec, n..dec.len(), &notes);
            println!(
                "jingle {notes:?} of {note_ms} ms notes after {gap} silent frames: {} sounding frames, {slots} decided as time slots, \
                 frames decoding each note {per_note:?}, {c} confident; {sc}",
                sounding.len()
            );
            assert!(sounding.len() >= 3, "the jingle sounds");
            assert_eq!(slots, sounding.len(), "jingle after {gap} silent frames: a frame not decided by the time-slot path");
            assert_eq!(sc.wrong_pitch, 0, "jingle after {gap} silent frames: a wrong pitch in FM");
            assert!(sc.wrong.is_empty(), "jingle after {gap} silent frames: {} wrong notes", sc.wrong.len());
        }
    }
}
