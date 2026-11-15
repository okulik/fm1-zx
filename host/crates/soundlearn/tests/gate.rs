// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fitting, gating and entry building: `tune.sna` learned end to end and played by method B,
//! the sweep and the siren refused, Castaway left to its built-in entry, and a formula with a
//! term missing caught by the replay (or by the fit), and a drum between the notes added to the
//! tune's entry.

mod fixtures;

use soundlearn::fit::{check_fit, to_entry};
use soundlearn::gate::{add_drum, builtin_at, drum_ok, judge, replay, replay_with_others, OtherSite, ReplayStats, Verdict, BUILTIN};
use soundlearn::listen::{listen, Inputs, PeriodSample, SiteHeard, Snapshot};
use soundlearn::model::{read_engine, EngineModel, Term};
use soundlearn::{encode_list, eval_prog, LearnedEntry};
use zx_sys::{Machine, SndEngines, SND_SITE_DRUM, SND_SITE_TONAL, SND_TONAL};

const CLOCK: f64 = 3_500_000.0;

fn none() -> Inputs {
    Box::new(|_| ([0; 8], 0))
}

fn model_at(m: &Machine, pc: u16) -> EngineModel {
    read_engine(|a| m.peek(a), pc).unwrap_or_else(|e| panic!("{pc:04X}: {e:?}"))
}

/// The learned entry's engine number in a table of the built-ins and one learned entry.
fn learned_engine() -> u8 {
    SndEngines::new().entries().len() as u8 + 1
}

/// Listens to the fixture `sna` for `frames`, reads its routine at the first heard site, fits
/// it, builds its entry and replays it: (model, entry, replay, verdict).
fn learn_fixture(sna: &[u8], frames: u32) -> (EngineModel, LearnedEntry, ReplayStats, Verdict) {
    let load = || fixtures::machine(sna);
    let inputs = none();
    let mut m = load();
    let heard = listen(&mut m, &inputs, frames);
    let site = heard.sites.iter().max_by_key(|s| s.edges).expect("a heard site");
    let model = model_at(&m, site.pc);
    let fit = check_fit(&model, site);
    let entry = to_entry(&model, |a| m.peek(a));
    let stats = replay(&load, &inputs, frames, &entry);
    let verdict = judge(&model, &m, Some(site), Some(&stats), fit.as_ref().ok());
    eprintln!("fit {fit:?}\nreplay {stats:?}\nverdict {verdict:?}");
    (model, entry, stats, verdict)
}

#[test]
fn learns_tune_end_to_end() {
    let fx = fixtures::square_tune();
    let (model, entry, stats, verdict) = learn_fixture(&fx.sna, 600);
    assert_eq!(verdict, Verdict::Learned { heard: true });
    assert!(stats.routine_frames > 400 && stats.claimed * 10 >= stats.routine_frames * 9, "{stats:?}");
    assert!(stats.music_gate);

    // the entry: the routine's signature, its site, the formula and the timing
    assert_eq!((entry.kind, entry.len), (0, model.code_end - model.anchor));
    assert_eq!(entry.sites, vec![(0x14, SND_SITE_TONAL, 0)]);
    assert_eq!((entry.pass, entry.flips, entry.drum, entry.stale), (0, false, 0, model.stale));
    assert!(entry.timing.is_some());
    let list = encode_list(std::slice::from_ref(&entry));
    let b = SndEngines::with_learned(&list).expect("the decoder takes the entry");
    let m = fixtures::machine(&fx.sna);
    assert_eq!(b.digest(&m, learned_engine() as usize - 1, model.anchor), entry.digest);
    // the program gives the full period, T x 256, for every note of the table
    for &(_, n, mm) in &fixtures::TUNE_TABLE {
        let mut m = fixtures::machine(&fx.sna);
        let ix = 0x9000u16;
        m.poke(ix + 2, n);
        m.poke(ix + 3, mm);
        let mut regs = m.regs();
        regs.ix = ix;
        let p = eval_prog(&entry.progs[0], |a| m.peek(a), model.anchor, &regs);
        let half = 87 + 13 * u32::from(n) + 16 * u32::from(mm);
        assert_eq!(p, Some(Some(2 * half * 256)), "note ({n}, {mm})");
    }

    // a fresh run through method B with the learned entry: the tune's sounding frames are claimed
    // at the tune's pitches
    let mut m = fixtures::machine(&fx.sna);
    let mut b = SndEngines::with_learned(&list).expect("decodes");
    let frames = 600;
    let mut sounding = Vec::new();
    let mut decided = Vec::new();
    for _ in 0..frames {
        m.run_frame();
        sounding.push(m.edges().iter().any(|e| e.pc == 0x8014));
        decided.push(b.frame(&m, m.edges(), m.edges_start_level()));
    }
    let delay = zx_sys::SND_DELAY_FRAMES as usize;
    let (mut total, mut good) = (0, 0);
    for f in 0..frames - delay {
        if !sounding[f] {
            continue;
        }
        total += 1;
        let d = &decided[f + delay];
        let p = d.tone[0].period_q8;
        let hz = if p != 0 { CLOCK * 256.0 / f64::from(p) } else { 0.0 };
        if d.confident != 0 && d.engine == learned_engine() && fixtures::TUNE_HZ.iter().any(|t| (hz - t).abs() <= 0.005 * t) {
            good += 1;
        }
    }
    eprintln!("tune: {good} of {total} sounding frames claimed at the tune's pitches");
    assert!(total > 400 && good * 10 >= total * 9, "{good} of {total}");
}

/// The fit's samples span two flips of a site, `2 s` half periods for a channel of `s` sites: the
/// flips fixture (one site per channel) and the toggler (one channel, two sites) are learned from
/// each of their sites; a time-slot routine's sites follow every channel, and are not fitted.
#[test]
fn learns_other_families() {
    for (fx, learned) in [(fixtures::xor_unison(), true), (fixtures::toggler_two_sites(), true), (fixtures::phase_slots(), false)] {
        let load = || fixtures::machine(&fx.sna);
        let inputs = none();
        let mut m = load();
        let heard = listen(&mut m, &inputs, 300);
        assert_eq!(heard.sites.len(), fx.sites.len(), "{}", fx.name);
        for site in &heard.sites {
            let model = model_at(&m, site.pc);
            let fit = check_fit(&model, site);
            let stats = replay(&load, &inputs, 300, &to_entry(&model, |a| m.peek(a)));
            let verdict = judge(&model, &m, Some(site), Some(&stats), fit.as_ref().ok());
            eprintln!("{} {:04X}: {fit:?} {stats:?} {verdict:?}", fx.name, site.pc);
            if learned {
                assert!(fit.as_ref().is_ok_and(|f| f.inliers > 0.95), "{}: {fit:?}", fx.name);
                assert_eq!(verdict, Verdict::Learned { heard: true }, "{}", fx.name);
            } else {
                assert!(fit.is_err() && matches!(verdict, Verdict::Refused(_)), "{}: {fit:?} {verdict:?}", fx.name);
            }
        }
    }
}

#[test]
fn refuses_sweep() {
    let fx = fixtures::sweep();
    let (model, _, _, verdict) = learn_fixture(&fx.sna, 300);
    assert!(matches!(verdict, Verdict::Refused(_)), "heard: {verdict:?}");
    let code_only = judge(&model, &fixtures::machine(&fx.sna), None, None, None);
    assert!(matches!(code_only, Verdict::Refused(_)), "code-only: {code_only:?}");
}

#[test]
fn refuses_siren() {
    let fx = fixtures::siren();
    let (model, _, _, verdict) = learn_fixture(&fx.sna, 300);
    assert!(matches!(verdict, Verdict::Refused(_)), "heard: {verdict:?}");
    let code_only = judge(&model, &fixtures::machine(&fx.sna), None, None, None);
    assert!(matches!(code_only, Verdict::Refused(_)), "code-only: {code_only:?}");
}

#[test]
fn builtin_wins() {
    // Castaway's tape, loaded with zx-sys alone, until its title tune plays
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../third_party/games/castaway/castawy.tap");
    let mut m = Machine::new();
    m.tape_insert(&std::fs::read(path).expect("castawy.tap")).expect("inserts");
    m.tape_autostart();
    let heard = listen(&mut m, &none(), 400);
    let tune: Vec<u16> = heard.sites.iter().filter(|s| s.edges > 1000).map(|s| s.pc).collect();
    assert_eq!(tune.len(), 2, "the tune's two sites: {tune:04X?}");
    for pc in tune {
        assert_eq!(builtin_at(&m, pc), Some(0xC864), "{pc:04X}");
    }
    // anywhere inside the built-in's signature, not only at its sites
    let len = SndEngines::new().entries()[0].len;
    assert!(len > 0x20);
    assert_eq!(builtin_at(&m, 0xC874), Some(0xC864));
    assert_eq!(builtin_at(&m, 0xC864 + len - 1), Some(0xC864));
    // nothing else is a built-in's
    assert_eq!(builtin_at(&m, 0xC864 + len), None);
    assert_eq!(builtin_at(&m, 0x8000), None);

    // a code-only candidate inside Castaway's tune routine (tune.sna's model, moved there: a table
    // walk and a duration, so the code-only gates pass) is refused for the built-in
    let tune = fixtures::machine(&fixtures::square_tune().sna);
    let mut model = model_at(&tune, 0x8014);
    assert_eq!(judge(&model, &tune, None, None, None), Verdict::Learned { heard: false });
    let shift = 0xC864u16.wrapping_sub(model.anchor);
    model.anchor = 0xC864;
    model.code_end = model.code_end.wrapping_add(shift);
    model.channels[0].sites = vec![0xC874];
    assert_eq!(judge(&model, &m, None, None, None), Verdict::Refused(BUILTIN.into()));
    // one whose sites lie elsewhere but whose anchor is in it, too
    model.channels[0].sites = vec![0x8014];
    assert_eq!(judge(&model, &m, None, None, None), Verdict::Refused(BUILTIN.into()));
    // and a replay that the built-in table claims, whatever else it shows
    let model = model_at(&tune, 0x8014);
    let stats = ReplayStats { routine_frames: 500, claimed: 500, music_gate: true, builtin_claimed: 450 };
    assert_eq!(judge(&model, &tune, None, Some(&stats), None), Verdict::Refused(BUILTIN.into()));
    assert_eq!(BUILTIN, "built-in entry");
}

/// A site heard with the samples `(predicted half period = BC, measured full period)`, against
/// tune's model with its half period replaced by `BC` T.
fn fit_samples(samples: &[(u16, f64)]) -> soundlearn::fit::FitResult {
    let tune = fixtures::machine(&fixtures::square_tune().sna);
    let mut model = model_at(&tune, 0x8014);
    model.channels[0].half_period = vec![Term::Lin { coef_t: 1, x: soundlearn::model::Input::Reg(8) }];
    let mut site = SiteHeard { pc: 0x8014, edges: 0, frames_with_edges: 0, periods: Vec::new(), snapshots: Vec::new() };
    for (i, &(bc, got)) in samples.iter().enumerate() {
        let mut regs = tune.regs();
        regs.bc = bc;
        site.periods.push(PeriodSample { period_t: got, at_frame: i as u32, snap: i as u32 });
        site.snapshots.push(Snapshot { regs, reads: Vec::new() });
    }
    check_fit(&model, &site).expect("evaluates")
}

#[test]
fn fit_counts_notes_and_inliers_soundly() {
    let exact = |bc: u16, n: usize| std::iter::repeat_n((bc, 2.0 * f64::from(bc)), n);
    // three notes, 8 samples each: all inliers
    let f = fit_samples(&exact(600, 8).chain(exact(800, 8)).chain(exact(1000, 8)).collect::<Vec<_>>());
    assert_eq!((f.inliers, f.distinct_notes), (1.0, 3), "{f:?}");
    // a predicted period with fewer than 8 samples agrees with nothing
    let f = fit_samples(&exact(600, 8).chain(exact(800, 7)).collect::<Vec<_>>());
    assert_eq!(f.distinct_notes, 1, "{f:?}");
    assert!((f.inliers - 8.0 / 15.0).abs() < 1e-9, "{f:?}");
    // two predictions 1 T apart (0.2%) are one note: a two-pitch siren stays two notes
    let f = fit_samples(&exact(500, 20).chain(exact(501, 20)).chain(exact(1000, 20)).collect::<Vec<_>>());
    assert_eq!((f.inliers, f.distinct_notes), (1.0, 2), "{f:?}");
    assert!((f.span_semitones - 12.0).abs() < 0.01, "{f:?}");
    // predictions that never repeat, every sample 10% off: no inliers (not 160 T of slack each)
    let f = fit_samples(&(0..30).map(|k| (350 + 3 * k, 1.1 * 2.0 * f64::from(350 + 3 * k))).collect::<Vec<_>>());
    assert_eq!((f.inliers, f.distinct_notes), (0.0, 0), "{f:?}");
    // a note whose samples are 10% off, many of them: its trimmed mean disagrees
    let f = fit_samples(&std::iter::repeat_n((700u16, 1540.0), 50).collect::<Vec<_>>());
    assert_eq!((f.inliers, f.distinct_notes), (0.0, 0), "{f:?}");
}

#[test]
fn partial_formula_refused() {
    let fx = fixtures::square_one_counter();
    let load = || fixtures::machine(&fx.sna);
    let frames = 300;
    let (model, entry, stats, verdict) = learn_fixture(&fx.sna, frames);
    // the whole formula is claimed (the replay is not what fails below)
    assert!(stats.routine_frames > 200 && stats.claimed * 10 >= stats.routine_frames * 9, "whole formula: {stats:?}");
    assert_eq!(verdict, Verdict::Learned { heard: true }, "whole formula");
    assert_eq!(entry.progs.len(), 1);
    let m = load();
    let heard = listen(&mut load(), &none(), frames);
    let site = &heard.sites[0];
    // without its delay term (13n + 58 read as 58): method B's check rejects it, the replay fails
    let mut partial = model.clone();
    partial.channels[0].half_period.retain(|t| !matches!(t, Term::Lin { .. }));
    assert_eq!(partial.channels[0].half_period, vec![Term::Const(58)]);
    let stats = replay(&load, &none(), frames, &to_entry(&partial, |a| m.peek(a)));
    eprintln!("without the delay: {stats:?}");
    assert!(stats.routine_frames > 200 && stats.claimed * 10 < stats.routine_frames * 9, "{stats:?}");
    let verdict = judge(&partial, &m, Some(site), Some(&stats), Some(&full_fit(&model, site)));
    assert!(matches!(&verdict, Verdict::Refused(why) if why.starts_with("replay")), "{verdict:?}");
    // without its constant (13n): 1.8-3.6% flat, inside the check's tolerance (1/16 of a period),
    // so the replay passes; the fit (0.6%) catches it
    let mut partial = model.clone();
    partial.channels[0].half_period.retain(|t| !matches!(t, Term::Const(_)));
    let stats = replay(&load, &none(), frames, &to_entry(&partial, |a| m.peek(a)));
    let fit = check_fit(&partial, site).expect("evaluates");
    eprintln!("without the constant: {stats:?}, {fit:?}");
    assert!(fit.inliers < 0.5, "{fit:?}");
    let verdict = judge(&partial, &m, Some(site), Some(&stats), Some(&fit));
    assert!(matches!(&verdict, Verdict::Refused(why) if why.starts_with("fit")), "{verdict:?}");
}

/// The whole model's fit (its gates pass), to judge a partial model's replay on its own.
fn full_fit(model: &EngineModel, site: &soundlearn::listen::SiteHeard) -> soundlearn::fit::FitResult {
    check_fit(model, site).expect("fits")
}

#[test]
fn drum_between_notes() {
    let fx = fixtures::tune_with_drum();
    let load = || fixtures::machine(&fx.sna);
    let frames = 300;
    let (model, entry, _, verdict) = learn_fixture(&fx.sna, frames);
    assert_eq!(verdict, Verdict::Learned { heard: true });
    assert_eq!(model.anchor, 0x8100);
    // the drum sounds only between the claimed notes, and its routine is noise (no model, no fit)
    let (stats, others) = replay_with_others(&load, &none(), frames, &entry);
    eprintln!("tone alone: {stats:?}, others {others:?}");
    let drum = others.iter().find(|o| o.pc == 0x8209).expect("the drum heard");
    assert!(drum.frames >= 10 && drum.between == drum.frames, "{drum:?}");
    let m = load();
    assert!(read_engine(|a| m.peek(a), 0x8209).is_err());
    assert!(drum_ok(drum, None));
    // not a drum: a site heard away from the notes, or one that fits as a tone
    assert!(!drum_ok(&OtherSite { between: drum.frames - 1, ..drum.clone() }, None));
    let tone_fit = soundlearn::fit::FitResult { inliers: 0.99, distinct_notes: 8, span_semitones: 12.0, max_hz: 1000.0 };
    assert!(!drum_ok(drum, Some(&tone_fit)));

    // the entry with the drum site: its signature grows to cover the drum routine
    let with = add_drum(&entry, model.anchor, 0x8209, |a| m.peek(a)).expect("in reach");
    assert_eq!((with.len, with.drum), (0x10B, 200));
    assert_eq!(with.sites, vec![(0x03, SND_SITE_TONAL, 0), (0x109, SND_SITE_DRUM, 0)]);
    let list = encode_list(std::slice::from_ref(&with));
    let b = SndEngines::with_learned(&list).expect("the decoder takes it");
    assert_eq!(b.digest(&m, learned_engine() as usize - 1, model.anchor), with.digest);
    let drummed = replay(&load, &none(), frames, &with);
    eprintln!("with the drum: {drummed:?}");
    assert!(drummed.claimed > stats.claimed && drummed.claimed * 100 >= drummed.routine_frames * 95, "{drummed:?}");
    // method B plays the drum: drum events in the claimed frames
    let mut b = SndEngines::with_learned(&list).expect("decodes");
    let mut m = load();
    let mut drums = 0;
    for _ in 0..frames {
        m.run_frame();
        let d = b.frame(&m, m.edges(), m.edges_start_level());
        if d.engine == learned_engine() && d.events().iter().any(|e| e.voice as usize == SND_TONAL && e.level != 0) {
            drums += 1;
        }
    }
    eprintln!("{drums} claimed frames with a drum, of {} drum frames", drum.frames);
    // (some drum hits fall in frames the check refuses: the note changes there too)
    assert!(drums * 3 >= drum.frames, "{drums} frames with a drum");

    // a drum before the anchor: the anchor moves back to it, every offset with it
    let before = add_drum(&entry, model.anchor, 0x80F0, |a| m.peek(a)).expect("in reach");
    assert_eq!((before.len, before.sites.clone()), (entry.len + 0x10, vec![(0, SND_SITE_DRUM, 0), (0x13, SND_SITE_TONAL, 0)]));
    let b = SndEngines::with_learned(&encode_list(std::slice::from_ref(&before))).expect("decodes");
    assert_eq!(b.digest(&m, learned_engine() as usize - 1, 0x80F0), before.digest);
    // out of reach
    assert!(add_drum(&entry, model.anchor, 0x8100 + 0x400, |a| m.peek(a)).is_none());
    // not another routine's: the entry's own site, inside its routine, or overlapping its start
    for pc in [0x8103, 0x8105, model.anchor + entry.len - 1, model.anchor - 1] {
        assert!(add_drum(&entry, model.anchor, pc, |a| m.peek(a)).is_none(), "{pc:04X}");
    }
    assert!(add_drum(&entry, model.anchor, model.anchor - 2, |a| m.peek(a)).is_some());
}
