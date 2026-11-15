// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The code reader on `tune.sna` and the generated routines of `fixtures`: families, channels,
//! formulas (evaluated against each fixture's run), timing, table walks and refusals.

mod fixtures;

use fixtures::{Fixture, Kind};
use soundlearn::model::{eval_terms, read_engine, scan_out_sites, EngineModel, Family, Input, Refusal, Term};
use zx_sys::Machine;

const CLOCK: f64 = 3_500_000.0;

fn read(fx: &Fixture, out: u16) -> Result<EngineModel, Refusal> {
    let m = fixtures::machine(&fx.sna);
    read_engine(|a| m.peek(a), out)
}

fn model(fx: &Fixture) -> EngineModel {
    read(fx, fx.sites[0]).unwrap_or_else(|e| panic!("{}: {e:?}", fx.name))
}

/// Each channel's evaluated half period at every `OUT` of a run, next to the fixture's truth:
/// (pc, truth per channel, model per channel).
fn compare(fx: &Fixture, md: &EngineModel, frames: u32) -> Vec<(u16, Vec<f64>, Vec<f64>)> {
    let (truth, ch) = (fx.truth, md.channels.clone());
    let anchor = md.anchor;
    let n = ch.len();
    fixtures::trace_outs(&fx.sna, frames, move |peek, r| {
        let mut v = truth(peek, r);
        v.extend(ch.iter().map(|c| eval_terms(&c.half_period, peek, anchor, r).unwrap_or(-1.0)));
        v
    })
    .into_iter()
    .filter(|o| o.probe.len() == 2 * n)
    .map(|o| (o.pc, o.probe[..n].to_vec(), o.probe[n..].to_vec()))
    .collect()
}

/// Every channel's evaluated half period equals the truth at every `OUT` of `frames` frames
/// (within `tol`, relative).
fn agrees(fx: &Fixture, md: &EngineModel, frames: u32, tol: f64) {
    let seen = compare(fx, md, frames);
    assert!(seen.len() > 100, "{}: {} OUTs", fx.name, seen.len());
    for (pc, truth, got) in &seen {
        for (t, g) in truth.iter().zip(got) {
            assert!((g - t).abs() <= tol * t, "{}: at {pc:04X} model {g} truth {t}", fx.name);
        }
    }
}

// ---- the fixtures themselves --------------------------------------------------------------

/// Half periods measured from a run: per channel, (measured, truth) for each span between its
/// flips (contention taken out). A span across a note change (the setup between notes) is
/// counted too, so a few disagree.
fn measured(fx: &Fixture, frames: u32) -> Vec<Vec<(f64, f64)>> {
    let truth = fx.truth;
    let outs = fixtures::trace_outs(&fx.sna, frames, move |peek, r| truth(peek, r));
    let channels = outs.first().map_or(0, |o| o.probe.len());
    let mut per = vec![Vec::new(); channels];
    match fx.kind {
        Kind::Square => {
            // every speaker edge; a full period over two of them, made of the half periods that
            // follow each (the toggler's halves differ, the sweep's change every edge)
            let mut level = false;
            let edges: Vec<_> = outs
                .iter()
                .filter(|o| {
                    let e = o.level != level;
                    level = o.level;
                    e
                })
                .collect();
            for w in edges.windows(3) {
                per[0].push(((w[2].t - w[0].t) as f64 / 2.0, (w[0].probe[0] + w[1].probe[0]) / 2.0));
            }
        }
        Kind::Flips => {
            for (c, &site) in fx.sites.iter().enumerate() {
                let mine: Vec<_> = outs.iter().filter(|o| o.pc == site).collect();
                for w in mine.windows(2) {
                    per[c].push(((w[1].t - w[0].t) as f64, w[0].probe[c]));
                }
            }
        }
        Kind::Slots => {
            // a note's level changes of each site: their mean spacing
            for (c, &site) in fx.sites.iter().enumerate() {
                let mine: Vec<_> = outs.iter().filter(|o| o.pc == site).collect();
                let mut i = 0;
                while i < mine.len() {
                    let j = mine[i..].iter().position(|o| o.probe != mine[i].probe).map_or(mine.len(), |k| i + k);
                    let flips: Vec<u64> = mine[i..j].windows(2).filter(|w| w[0].level != w[1].level).map(|w| w[1].t).collect();
                    if flips.len() > 20 {
                        let mean = (flips[flips.len() - 1] - flips[0]) as f64 / (flips.len() - 1) as f64;
                        per[c].push((mean, mine[i].probe[c]));
                    }
                    i = j;
                }
            }
        }
    }
    per
}

#[test]
fn fixtures_sound_as_listed() {
    for fx in fixtures::all() {
        let per = measured(&fx, 150);
        assert_eq!(per.len(), if fx.kind == Kind::Square { 1 } else { fx.sites.len() }, "{}", fx.name);
        for (c, spans) in per.iter().enumerate() {
            let tol = if fx.kind == Kind::Slots { 0.002 } else { 0.0 };
            let good = spans.iter().filter(|&&(got, truth)| (got - truth).abs() <= tol * truth).count();
            assert!(good >= 3, "{} channel {c}: {good} spans agree", fx.name);
            assert!(good * 100 >= spans.len() * 95, "{} channel {c}: {good} of {} spans agree; {:?}", fx.name, spans.len(), &spans[..spans.len().min(8)]);
        }
        // the speaker OUTs are the listed sites
        let mut sites: Vec<u16> = fixtures::trace_outs(&fx.sna, 20, |_, _| Vec::new()).iter().map(|o| o.pc).collect();
        sites.sort_unstable();
        sites.dedup();
        assert_eq!(sites, fx.sites, "{}", fx.name);
    }
}

// ---- the code reader ------------------------------------------------------------------------

#[test]
fn reads_tune() {
    let fx = fixtures::square_tune();
    let md = model(&fx);
    assert_eq!(md.family, Family::Square);
    assert_eq!(md.channels.len(), 1);
    assert_eq!(md.channels[0].sites, vec![0x8014]);
    assert!(!md.flips);
    assert_eq!((md.pass_t, md.pos.clone()), (0, vec![0]));
    assert_eq!((md.anchor, md.code_end), (0x8000, 0x802F));
    // 13 B + 16 C + 87, B and C through IX
    let half = &md.channels[0].half_period;
    assert!(half.contains(&Term::Lin { coef_t: 13, x: Input::Indexed(11, 2) }), "{half:?}");
    assert!(half.contains(&Term::Lin { coef_t: 16, x: Input::Indexed(11, 3) }), "{half:?}");
    assert!(half.contains(&Term::Const(87)), "{half:?}");
    assert_eq!(half.len(), 3, "{half:?}");
    // every note: the listing's period, and the pitch fmzx-pack measured
    let m = fixtures::machine(&fx.sna);
    for (i, &(_, n, mm)) in fixtures::TUNE_TABLE.iter().enumerate() {
        let mut regs = m.regs();
        regs.ix = fixtures::TUNE_TABLE_AT + 4 * i as u16;
        let h = eval_terms(half, &|a| m.peek(a), md.anchor, &regs).expect("sounds");
        assert_eq!(h, 87.0 + 13.0 * f64::from(n) + 16.0 * f64::from(mm));
        let hz = CLOCK / (2.0 * h);
        // TUNE_HZ counts zero crossings of the rendered audio over 21 frames: it is itself only
        // good to about 0.13% (C5: 3,339 T is 524.11 Hz, measured 524.67)
        assert!((hz - fixtures::TUNE_HZ[i]).abs() <= 0.0015 * fixtures::TUNE_HZ[i], "note {i}: {hz} Hz vs {}", fixtures::TUNE_HZ[i]);
    }
    agrees(&fx, &md, 200, 0.0);
    // the notes come from a table IX walks, 4 bytes a note; HL counts the edges down from K
    assert!(md.table_walk);
    let d = md.duration.as_ref().expect("a duration counter");
    assert_eq!(d.counter, Input::Reg(10));
    assert_eq!(d.counts_down_from, Some(Input::Indexed(11, 0)));
    assert_eq!(d.per_unit_t, *half);
    // at a note's sixth edge (HL = K - 5), five half periods have passed since its first
    let mut regs = m.regs();
    regs.ix = fixtures::TUNE_TABLE_AT;
    regs.hl = fixtures::TUNE_TABLE[0].0 - 5;
    assert_eq!(d.held(&|a| m.peek(a), md.anchor, &regs), Some(5.0 * (87.0 + 13.0 * 244.0 + 16.0 * 5.0)));
    // memory is not to be read outside the loop 8010h-8027h
    assert_eq!(md.stale, Some((0x28, 0x10u16.wrapping_sub(0x28))));
    assert!(md.wild.is_empty() && md.reloc.is_empty(), "{:?} {:?}", md.wild, md.reloc);
}

#[test]
fn reads_square_one_counter() {
    let fx = fixtures::square_one_counter();
    let md = model(&fx);
    assert_eq!(md.family, Family::Square);
    assert_eq!(md.channels.len(), 1);
    // the reload is C, as it stands at the edge
    assert!(md.channels[0].half_period.contains(&Term::Lin { coef_t: 13, x: Input::Reg(3) }), "{:?}", md.channels[0].half_period);
    agrees(&fx, &md, 200, 0.0);
    assert!(md.table_walk);
    let d = md.duration.as_ref().expect("a duration counter");
    assert_eq!((d.counter, d.counts_down_from), (Input::Reg(9), Some(Input::Indexed(10, -2))));
    // the loop table pointer: LD HL,801Fh lies past the code, so nothing is relocated
    assert_eq!((md.anchor, md.code_end), (0x8000, 0x801F));
    assert!(md.reloc.is_empty());
}

#[test]
fn toggler_two_sites_one_channel() {
    let fx = fixtures::toggler_two_sites();
    for &site in &fx.sites {
        let md = read(&fx, site).unwrap_or_else(|e| panic!("from {site:04X}: {e:?}"));
        assert_eq!(md.family, Family::Square);
        assert_eq!(md.channels.len(), 1, "from {site:04X}");
        assert_eq!(md.channels[0].sites, fx.sites);
        assert!(!md.flips);
        agrees(&fx, &md, 200, 0.0);
        let d = md.duration.as_ref().expect("a duration counter");
        assert_eq!((d.counter, d.counts_down_from), (Input::Reg(4), Some(Input::Indexed(11, 1))));
        assert!(md.table_walk);
    }
}

#[test]
fn ambiguous_partition_refused() {
    let fx = fixtures::toggler_ambiguous();
    for &site in &fx.sites {
        assert!(matches!(read(&fx, site), Err(Refusal::Ambiguous(_))), "from {site:04X}: {:?}", read(&fx, site));
    }
}

#[test]
fn xor_unison_two_channels() {
    let fx = fixtures::xor_unison();
    for &site in &fx.sites {
        let md = read(&fx, site).unwrap_or_else(|e| panic!("from {site:04X}: {e:?}"));
        assert_eq!(md.family, Family::Flips);
        assert!(md.flips);
        assert_eq!(md.channels.len(), 2);
        assert_eq!(md.channels[0].sites, vec![0x8036]);
        assert_eq!(md.channels[1].sites, vec![0x8056]);
        assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 180, x: Input::Abs(0x9001) }]);
        assert_eq!(md.channels[1].half_period, vec![Term::Lin { coef_t: 180, x: Input::Abs(0x9002) }]);
        assert_eq!(md.pass_t, 0);
        agrees(&fx, &md, 200, 0.0);
        let d = md.duration.as_ref().expect("a duration counter");
        assert_eq!((d.counter, d.counts_down_from), (Input::Reg(8), Some(Input::Indexed(10, -2))));
        assert_eq!(d.per_unit_t, vec![Term::Const(180)]);
        assert!(md.table_walk);
    }
}

#[test]
fn phase_slots_recip() {
    let fx = fixtures::phase_slots();
    let md = model(&fx);
    assert_eq!(md.family, Family::Slots);
    assert!(!md.flips);
    assert_eq!(md.channels.len(), 2);
    assert_eq!((md.pass_t, md.pos.clone()), (218, vec![0, 97]));
    for (c, at) in [(0usize, 0x9004u16), (1, 0x9006)] {
        assert_eq!(md.channels[c].half_period, vec![Term::Recip { num_t: 32768 * 218, x: Input::Abs16(at) }]);
    }
    // the frequencies the run makes (the fixture's truth, checked against its edges)
    agrees(&fx, &md, 200, 0.002);
    let d = md.duration.as_ref().expect("a duration counter");
    assert_eq!((d.counter, d.counts_down_from), (Input::Reg(8), Some(Input::Rel16(0x19))));
    assert_eq!(d.per_unit_t, vec![Term::Const(218)]);
    // a zero step is silence
    let regs = fixtures::machine(&fx.sna).regs();
    assert_eq!(eval_terms(&md.channels[0].half_period, &|_| 0, md.anchor, &regs), None);
    // the steps are copied from a table HL walks
    assert!(md.table_walk);
    // JP NZ,pass's target is inside the code: relocated
    assert_eq!(md.reloc, vec![0x45]);
}

#[test]
fn refuses_computed_jump() {
    let fx = fixtures::computed_jump();
    assert_eq!(read(&fx, 0x8002), Err(Refusal::ComputedJump(0x8007)));
}

#[test]
fn sweep_has_no_table_walk() {
    let fx = fixtures::sweep();
    let md = model(&fx);
    assert_eq!(md.family, Family::Square);
    assert!(!md.table_walk);
    agrees(&fx, &md, 100, 0.0);
    // E counts the passes but also sets the pitch: the note's time is not a count of equal units
    assert!(md.duration.is_none(), "{:?}", md.duration);
}

#[test]
fn siren_has_no_table_walk() {
    let fx = fixtures::siren();
    let md = model(&fx);
    assert_eq!(md.family, Family::Square);
    assert!(!md.table_walk);
    agrees(&fx, &md, 100, 0.0);
    // H counts 64 edges down from the operand of LD H,64 (anchor 8002h: start is not reached)
    assert_eq!(md.anchor, 0x8002);
    let d = md.duration.as_ref().expect("a duration counter");
    assert_eq!((d.counter, d.counts_down_from), (Input::Reg(6), Some(Input::Rel(0x07))));
}

#[test]
fn scan_finds_sites() {
    for fx in fixtures::all() {
        let m = fixtures::machine(&fx.sna);
        assert_eq!(scan_out_sites(|a| m.peek(a)), fx.sites, "{}", fx.name);
    }
    // a RAM full of OUT (FFh),A: an odd port, no speaker
    assert!(scan_out_sites(|a| if a % 2 == 0 { 0xD3 } else { 0xFF }).is_empty());
    let mut m = Machine::new();
    for a in 0x4000..=0xFFFFu16 {
        m.poke(a, if a % 2 == 0 { 0xD3 } else { 0xFF });
    }
    assert!(scan_out_sites(|a| m.peek(a)).is_empty());
    // OUT (C),r counts only after C is set even
    let mem = |code: &'static [u8]| move |a: u16| if (0x9000..0x9000 + code.len() as u16).contains(&a) { code[(a - 0x9000) as usize] } else { 0 };
    assert_eq!(scan_out_sites(mem(&[0x0E, 0xFE, 0xED, 0x79])), vec![0x9002]);
    assert_eq!(scan_out_sites(mem(&[0x01, 0xFE, 0x00, 0x3C, 0xED, 0x41])), vec![0x9004]);
    assert!(scan_out_sites(mem(&[0x0E, 0xFF, 0xED, 0x79])).is_empty());
    assert!(scan_out_sites(mem(&[0x0E, 0xFE, 0x0C, 0xED, 0x79])).is_empty());
    assert!(scan_out_sites(mem(&[0x3E, 0xFE, 0xED, 0x79])).is_empty());
}

#[test]
fn refuses_what_is_not_a_speaker_out() {
    let fx = fixtures::square_tune();
    let m = fixtures::machine(&fx.sna);
    // not an OUT; OUT to an odd port
    assert_eq!(read_engine(|a| m.peek(a), 0x8016), Err(Refusal::NotSpeaker(0x8016)));
    assert_eq!(read_engine(|a| if a == 0x9000 { 0xD3 } else if a == 0x9001 { 0xFF } else { 0 }, 0x9000), Err(Refusal::NotSpeaker(0x9000)));
}

#[test]
fn refuses_contended_code() {
    // square_one_counter's loop moved to 6000h: whole-T coefficients cannot carry contention
    let fx = fixtures::square_one_counter();
    let m = fixtures::machine(&fx.sna);
    let moved = |a: u16| if (0x6000..0x6040).contains(&a) { m.peek(a + 0x2000) } else { 0 };
    assert!(matches!(read_engine(moved, 0x6010), Err(Refusal::UnknownShape(_))), "{:?}", read_engine(moved, 0x6010));
}

#[test]
fn refuses_unmodelled_self_modification() {
    // the sweep with its setup writing its own JR NZ offset:
    // 8000 start: LD E,200; 8002 LD (800Eh),A; 8005 tog: XOR 10h; 8007 OUT (FEh),A; 8009 LD B,E;
    // 800A d: DJNZ d; 800C DEC E; 800D JR NZ,tog; 800F JR start
    let mem = at_8000(&[0x1E, 0xC8, 0x32, 0x0E, 0x80, 0xEE, 0x10, 0xD3, 0xFE, 0x43, 0x10, 0xFE, 0x1D, 0x20, 0xF6, 0x18, 0xEF]);
    assert_eq!(read_engine(mem, 0x8007), Err(Refusal::SelfModUnmodelled(0x8002)));
}

#[test]
fn self_modified_operand_is_wild() {
    // the same, writing the operand of LD E,200 instead (LD (8001h),A): wild, read as Rel
    let mem = at_8000(&[0x1E, 0xC8, 0x32, 0x01, 0x80, 0xEE, 0x10, 0xD3, 0xFE, 0x43, 0x10, 0xFE, 0x1D, 0x20, 0xF6, 0x18, 0xEF]);
    let md = read_engine(mem, 0x8007).expect("reads");
    assert_eq!(md.wild, vec![0x01]);
    // the sweep restarts from it: the relocated address of LD (8001h),A
    assert_eq!(md.reloc, vec![0x03]);
}

/// Memory holding `code` at 8000h, zeros elsewhere.
fn at_8000(code: &'static [u8]) -> impl Fn(u16) -> u8 {
    move |a: u16| if (0x8000..0x8000 + code.len() as u16).contains(&a) { code[(a - 0x8000) as usize] } else { 0 }
}

#[test]
fn refuses_counter_reloaded_in_pass() {
    // the zero test's counter is set every pass: JR NZ,tog is never taken, the delay always runs
    // 8000 tog: XOR 10h; OUT (FEh),A; LD C,1; DEC C; JR NZ,tog; LD B,100; d: DJNZ d; JR tog
    let mem = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0x0E, 0x01, 0x0D, 0x20, 0xF7, 0x06, 0x64, 0x10, 0xFE, 0x18, 0xF1]);
    assert_eq!(read_engine(mem, 0x8002), Err(Refusal::UnknownShape(0x8007)));
}

#[test]
fn refuses_unbalanced_flips() {
    // xor_unison with channel 0's padding a T-state short (three NOPs for an LD A,(nn))
    let fx = fixtures::xor_unison();
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if (0x801B..0x801E).contains(&a) { 0 } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8036), Err(Refusal::UnknownShape(0x8019)));
}

#[test]
fn same_byte_in_one_half_period_is_one_channel() {
    // two delays from the byte at IX+0 between the same two edges: one longer delay
    // 8000 LD IX,9000h; 8004 tog: XOR 10h; 8006 OUT (FEh),A; 8008 LD B,(IX+0); 800B d1: DJNZ d1;
    // 800D LD C,(IX+0); 8010 d2: DEC C; 8011 JR NZ,d2; 8013 JR tog
    let mem = at_8000(&[
        0xDD, 0x21, 0x00, 0x90, 0xEE, 0x10, 0xD3, 0xFE, 0xDD, 0x46, 0x00, 0x10, 0xFE, 0xDD, 0x4E, 0x00, 0x0D, 0x20, 0xFD, 0x18, 0xEF,
    ]);
    let md = read_engine(mem, 0x8006).expect("reads");
    assert_eq!(md.family, Family::Square);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 29, x: Input::Indexed(11, 0) }, Term::Const(58)]);
    assert!(md.duration.is_none() && !md.table_walk);
}
