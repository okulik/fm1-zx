// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The code reader on `tune.sna` and the generated routines of `fixtures`: families, channels,
//! formulas (evaluated against each fixture's run), timing, table walks and refusals.

mod fixtures;

use fixtures::{Fixture, Kind};
use soundlearn::model::{eval_terms, read_engine, scan_out_sites, DurationModel, EngineModel, Family, Input, Refusal, Term, Why};
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
    assert!(matches!(read_engine(moved, 0x6010), Err(Refusal::Unmodelled(Why::Contended, _))), "{:?}", read_engine(moved, 0x6010));
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
    assert_eq!(read_engine(mem, 0x8002), Err(Refusal::Unmodelled(Why::CounterReloaded, 0x8007)));
}

#[test]
fn refuses_unbalanced_flips() {
    // xor_unison with channel 0's padding a T-state short (three NOPs for an LD A,(nn))
    let fx = fixtures::xor_unison();
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if (0x801B..0x801E).contains(&a) { 0 } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8036), Err(Refusal::Unmodelled(Why::UnbalancedFlips, 0x8019)));
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

// ---- review round 1: three unsound shapes -------------------------------------------------

#[test]
fn slots_phase_in_a_register() {
    // one ADD HL,DE a pass, the phase kept in HL: 8000 loop: ADD HL,DE; LD A,H; AND 80h;
    // RRCA x3; OUT (FEh),A; JR loop. A pass is 11+4+7+12+11+12 = 57 T.
    let mem = at_8000(&[0x19, 0x7C, 0xE6, 0x80, 0x0F, 0x0F, 0x0F, 0xD3, 0xFE, 0x18, 0xF5]);
    let md = read_engine(mem, 0x8007).expect("reads");
    assert_eq!(md.family, Family::Slots);
    assert_eq!(md.channels[0].half_period, vec![Term::Recip { num_t: 32768 * 57, x: Input::Reg(9) }]);
}

#[test]
fn refuses_two_adds_to_one_phase() {
    // as above with ADD HL,DE twice a pass: the phase moves by 2 x DE, an octave up; the two
    // ADDs are not one accumulator seen twice
    let mem = at_8000(&[0x19, 0x19, 0x7C, 0xE6, 0x80, 0x0F, 0x0F, 0x0F, 0xD3, 0xFE, 0x18, 0xF4]);
    let got = read_engine(mem, 0x8008);
    assert!(matches!(got, Err(Refusal::Unmodelled(Why::NoToggle, _))), "{got:?}");
}

#[test]
fn refuses_setup_flipping_the_level() {
    // a hidden second channel: every 5 passes the setup path flips the level byte too
    // 8000 top: LD A,(9000h); XOR 10h; LD (9000h),A; OUT (FEh),A; LD B,C; DJNZ $; DEC E;
    // JR NZ,top; 8010 LD E,5; LD A,(9000h); XOR 10h; LD (9000h),A; JR top
    let mem = at_8000(&[
        0x3A, 0x00, 0x90, 0xEE, 0x10, 0x32, 0x00, 0x90, 0xD3, 0xFE, 0x41, 0x10, 0xFE, 0x1D, 0x20, 0xF0,
        0x1E, 0x05, 0x3A, 0x00, 0x90, 0xEE, 0x10, 0x32, 0x00, 0x90, 0x18, 0xE4,
    ]);
    assert_eq!(read_engine(mem, 0x8008), Err(Refusal::Unmodelled(Why::SetupLevel, 0x8010)));
}

#[test]
fn refuses_setup_with_a_speaker_out() {
    // the setup path OUTs to the speaker (its conditional keeps it from being a diamond)
    // 8000 top: XOR 10h; OUT (FEh),A; LD B,C; DJNZ $; DEC E; JR NZ,top;
    // 800A LD E,5; OUT (FEh),A; OR A; JR Z,top; JR top
    let mem = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0x41, 0x10, 0xFE, 0x1D, 0x20, 0xF6, 0x1E, 0x05, 0xD3, 0xFE, 0xB7, 0x28, 0xEF, 0x18, 0xED]);
    assert_eq!(read_engine(mem, 0x8002), Err(Refusal::Unmodelled(Why::SetupOut, 0x800A)));
}

#[test]
fn table_walk_needs_the_advanced_pointer() {
    // the pitch C comes from a fixed variable through HL; HL is then reloaded and stepped as a
    // counter: no table walk
    // 8000 start: LD HL,9000h; LD C,(HL); LD HL,(9002h); INC HL; LD (9002h),HL; LD D,100;
    // 800D tog: XOR 10h; OUT (FEh),A; LD B,C; DJNZ $; DEC D; JR NZ,tog; JR start
    let mem = at_8000(&[
        0x21, 0x00, 0x90, 0x4E, 0x2A, 0x02, 0x90, 0x23, 0x22, 0x02, 0x90, 0x16, 0x64,
        0xEE, 0x10, 0xD3, 0xFE, 0x41, 0x10, 0xFE, 0x15, 0x20, 0xF6, 0x18, 0xE7,
    ]);
    let md = read_engine(mem, 0x800F).expect("reads");
    assert_eq!(md.channels[0].half_period[0], Term::Lin { coef_t: 13, x: Input::Reg(3) });
    assert!(!md.table_walk);
}

// ---- Task 10: specific refusals ----------------------------------------------------------------

/// A square loop whose setup path (from the zero test's exit at 800Ah) passes `n` two-way
/// branches that meet again at once, so 2^n paths lead back into the pass:
/// 8000 tog: XOR 10h; OUT (FEh),A; LD B,C; DJNZ $; DEC E; JR NZ,tog;
/// 800A n x (JR Z,+1; NOP); LD E,5; JR tog
fn forked_setup(n: usize) -> Vec<u8> {
    let mut code = vec![0xEE, 0x10, 0xD3, 0xFE, 0x41, 0x10, 0xFE, 0x1D, 0x20, 0xF6];
    for _ in 0..n {
        code.extend([0x28, 0x01, 0x00]);
    }
    code.extend([0x1E, 0x05, 0x18]);
    code.push((-(code.len() as i32 + 1)) as u8);
    code
}

#[test]
fn refuses_setup_paths_past_the_limit() {
    // 2^7 = 128 paths: more than the reader follows, so one could go unchecked
    let code = forked_setup(7);
    let mem = |a: u16| code.get(usize::from(a.wrapping_sub(0x8000))).copied().filter(|_| a >= 0x8000).unwrap_or(0);
    assert_eq!(read_engine(mem, 0x8002), Err(Refusal::Unmodelled(Why::SetupPaths, 0x800A)));
    // 2^5 = 32 paths: all followed, the routine is read
    let code = forked_setup(5);
    let mem = |a: u16| code.get(usize::from(a.wrapping_sub(0x8000))).copied().filter(|_| a >= 0x8000).unwrap_or(0);
    let md = read_engine(mem, 0x8002).expect("reads");
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(33)]);
}

#[test]
fn reads_flips_uneven() {
    // one channel whose reload pass is longer: n passes from flip to flip, n - 1 short and one
    // long, so the half period is 45 x C + 13 (the pass with its other arm, plus the difference)
    let fx = fixtures::flips_uneven();
    let md = model(&fx);
    assert_eq!(md.family, Family::Flips);
    assert_eq!(md.channels.len(), 1);
    assert_eq!(md.channels[0].sites, vec![0x8016]);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 45, x: Input::Reg(3) }, Term::Const(13)]);
    assert!(md.table_walk);
    // E counts the passes, carrying into D: DE is one counter, from the record's word
    assert_eq!(md.duration, Some(DurationModel { counter: Input::Reg(9), per_unit_t: vec![Term::Const(45)], counts_down_from: Some(Input::Indexed(10, -2)), carry: true }));
    // the formula is the note's half period; a span over the note counter's wrap takes the
    // counter's 11 T longer path as well (the truth counts it)
    let seen = compare(&fx, &md, 150);
    assert!(seen.len() > 1000, "{} OUTs", seen.len());
    let exact = seen.iter().filter(|(_, t, g)| t[0] == g[0]).count();
    assert!(seen.iter().all(|(_, t, g)| t[0] == g[0] || t[0] == g[0] + 11.0), "{:?}", seen.iter().find(|(_, t, g)| t[0] != g[0] && t[0] != g[0] + 11.0));
    assert!(exact * 100 >= seen.len() * 80, "{exact} of {}", seen.len());
}

#[test]
fn refuses_setup_flipping_the_flips_level() {
    // flips_uneven with its note setup flipping the level (XOR 10h) instead of setting it
    // (LD A,10h): a flip between notes that no channel accounts for
    let fx = fixtures::flips_uneven();
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x800D { 0xEE } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8016), Err(Refusal::Unmodelled(Why::SetupFlipsLevel, 0x8024)));
}

#[test]
fn reads_toggler_saved_nested() {
    // one channel from two sites: the counts B and C are saved and put back around each half's
    // nested delay, so both halves count the same registers (one counter, two places)
    let fx = fixtures::toggler_saved_nested();
    for site in fx.sites.clone() {
        let md = read(&fx, site).unwrap_or_else(|e| panic!("from {site:04X}: {e:?}"));
        assert_eq!(md.family, Family::Square);
        assert_eq!(md.channels.len(), 1);
        assert_eq!(md.channels[0].sites, vec![0x8016, 0x8021]);
        // 13 x B + 3339 x C - 3282.5, the half constant rounded
        assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(2) }, Term::Lin { coef_t: 3339, x: Input::Reg(3) }, Term::Const(-3282)]);
        assert!(md.stack, "it pushes and pops");
        // called once a note: frames end outside it, and its formula reads registers only
        assert_eq!((md.stale, md.duration.is_none()), (None, true));
        agrees(&fx, &md, 150, 0.0004); // the half constant's half T
    }
}

#[test]
fn refuses_unmatched_stack() {
    let fx = fixtures::toggler_saved_nested();
    let m = fixtures::machine(&fx.sna);
    // the second POP BC gone: each pass leaves a pair more on the stack
    let mem = |a: u16| if a == 0x8028 { 0x00 } else { m.peek(a) };
    assert!(matches!(read_engine(mem, 0x8016), Err(Refusal::Unmodelled(Why::StackOp, _))), "{:?}", read_engine(mem, 0x8016));
    // POP DE takes what PUSH BC put
    let mem = |a: u16| if a == 0x801D { 0xD1 } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8016), Err(Refusal::Unmodelled(Why::StackOp, 0x8016)));
    // the stack pointer moved in the pass (INC SP for the second PUSH)
    let mem = |a: u16| if a == 0x801E { 0x33 } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8016), Err(Refusal::Unmodelled(Why::StackOp, 0x801E)));
}

#[test]
fn nested_delay_only_without_a_reload() {
    // 8000 tog: XOR 10h; OUT (FEh),A; LD C,3; d: DJNZ d; NOP; NOP; DEC C; JR NZ,d; JR tog: the
    // inner loop counts B (0 after it, 256 each further time), the outer C
    let nested = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0x0E, 0x03, 0x10, 0xFE, 0x00, 0x00, 0x0D, 0x20, 0xF9, 0x18, 0xF1]);
    let md = read_engine(nested, 0x8002).expect("reads");
    // 7 + 11 + 7 + (13 B - 5) + 3 x (256 x 13 - 5 + 12 + 12) - (256 x 13 - 5 + 12 - 7) + 12
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(2) }, Term::Const(7 + 11 + 7 - 5 + 3 * 3347 - 3328 - 5 + 12 + 5)]);
    // LD B,5 in the outer loop's body: each inner run counts 5, a product the reader does not
    // take: not a nested delay, and the loop is refused
    let reload = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0x0E, 0x03, 0x10, 0xFE, 0x06, 0x05, 0x0D, 0x20, 0xF9, 0x18, 0xF1]);
    let got = read_engine(reload, 0x8002);
    assert!(got.is_err(), "{got:?}");
}

#[test]
fn registers_not_given_back_are_not_inputs() {
    // the first POP BC; PUSH BC become NOPs: the second half counts B and C from 0 (256 each),
    // so its delay is a constant while the first half's counts the note: a duty cycle
    let fx = fixtures::toggler_saved_nested();
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x801D || a == 0x801E { 0x00 } else { m.peek(a) };
    assert!(read_engine(mem, 0x8016).is_err(), "{:?}", read_engine(mem, 0x8016));
}

/// A routine called once a burst (a short burst of its note each call): the call's target sets
/// the burst's passes (HL, a pair counted with a carry), IX and the level, then the loop flips
/// the speaker every (IX+0) passes, its reload arm 28 T longer than the other:
/// 8000 main: CALL burst; JR main; 8010 burst: LD HL,0300h; LD IX,9000h; LD A,10h; LD B,1;
/// 801B loop: DJNZ skip; LD B,(IX+0); XOR 10h; OUT (FEh),A; JR next; 8026 skip: NOP x 4;
/// 802A next: DEC L; JR NZ,loop; DEC H; JR NZ,loop; RET
fn burst_routine() -> Vec<u8> {
    let mut m = vec![0u8; 0x31];
    m[..5].copy_from_slice(&[0xCD, 0x10, 0x80, 0x18, 0xFB]);
    m[0x10..].copy_from_slice(&[
        0x21, 0x00, 0x03, 0xDD, 0x21, 0x00, 0x90, 0x3E, 0x10, 0x06, 0x01, 0x10, 0x09, 0xDD, 0x46, 0x00, 0xEE, 0x10, 0xD3, 0xFE,
        0x18, 0x04, 0x00, 0x00, 0x00, 0x00, 0x2D, 0x20, 0xEE, 0x25, 0x20, 0xEB, 0xC9,
    ]);
    m
}

#[test]
fn reads_the_code_a_call_runs_into_the_loop() {
    let code = burst_routine();
    let mem = |code: Vec<u8>| move |a: u16| code.get(usize::from(a.wrapping_sub(0x8000))).copied().filter(|_| a >= 0x8000).unwrap_or(0);
    let md = read_engine(mem(code.clone()), 0x8022).expect("reads");
    assert_eq!(md.family, Family::Flips);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 45, x: Input::Indexed(11, 0) }, Term::Const(28)]);
    // the routine starts where the call goes; the burst's passes come from its operand
    assert_eq!(md.anchor, 0x8010);
    assert_eq!(md.duration, Some(DurationModel { counter: Input::Reg(10), per_unit_t: vec![Term::Const(45)], counts_down_from: Some(Input::Rel16(1)), carry: true }));
    // no call to it: the loop alone, its timing not known
    let mut alone = code.clone();
    alone[0] = 0;
    let md = read_engine(mem(alone), 0x8022).expect("reads");
    assert_eq!((md.anchor, md.duration), (0x801B, None));
    // the code before the loop makes an edge of its own (OUT for LD A,10h): refused
    let mut edge = code;
    edge[0x17..0x19].copy_from_slice(&[0xD3, 0xFE]);
    let got = read_engine(mem(edge), 0x8022);
    assert!(matches!(got, Err(Refusal::Unmodelled(Why::SetupOut, _))), "{got:?}");
}

#[test]
fn carry_counter_holds_the_passes() {
    // DurationModel's carry: from HL = 0300h (768 passes to go: L's 0 counts 256) the note has
    // held 1 pass at 03FFh, 255 at 0301h, 256 at 0200h (L reached 0 and H took one), 257 at
    // 02FFh, 511 at 0201h, 512 at 0100h
    use soundlearn::model::carry_to_go;
    let start = carry_to_go(0x0300);
    // the burst routine's duration: held() and its entry's timing program (45 T a pass) agree
    let code = burst_routine();
    let peek = |a: u16| code.get(usize::from(a.wrapping_sub(0x8000))).copied().filter(|_| a >= 0x8000).unwrap_or(0);
    let md = read_engine(peek, 0x8022).expect("reads");
    let timing = soundlearn::fit::to_entry(&md, peek).timing.expect("a timing program");
    let d = md.duration.clone().expect("a duration");
    for (hl, held) in [(0x03FFu32, 1u32), (0x0301, 255), (0x0200, 256), (0x02FF, 257), (0x0201, 511), (0x0100, 512)] {
        assert_eq!(start - carry_to_go(hl), held, "{hl:04X}");
        let mut regs = Machine::new().regs();
        regs.hl = hl as u16;
        assert_eq!(d.held(&peek, md.anchor, &regs), Some(f64::from(45 * held)), "{hl:04X}");
        assert_eq!(soundlearn::eval_prog(&timing, peek, md.anchor, &regs), Some(Some(45 * held)), "{hl:04X}");
        // and the device's interpreter, on a machine holding the routine
        let mut m = Machine::new();
        for (i, &b) in code.iter().enumerate() {
            m.poke(0x8000 + i as u16, b);
        }
        assert_eq!(zx_sys::snd_prog_eval(&soundlearn::encode_prog(&timing), &m, md.anchor, &regs), (0, 45 * held), "{hl:04X}");
    }
}

#[test]
fn leading_pop_takes_the_pair_the_pass_gives_back() {
    // 8000 tog: XOR 10h; OUT (FEh),A; POP BC; PUSH BC; d: DJNZ d; POP BC; PUSH BC; JR tog: the
    // pass pops what it pushed the time before, BC as the pass found it (B at the OUT is the count)
    let kept = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0xC1, 0xC5, 0x10, 0xFE, 0xC1, 0xC5, 0x18, 0xF4]);
    let md = read_engine(kept, 0x8002).expect("reads");
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(2) }, Term::Const(72 - 5)]);
    // without the second POP BC; PUSH BC the DJNZ leaves B at 0 for the next pass's OUT, and with
    // LD B,7 before the jump back at 7: the POP's value is the pair from before, not BC as the
    // pass starts with it, so the count is not B at the OUT
    let zeroed = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0xC1, 0xC5, 0x10, 0xFE, 0x18, 0xF6]);
    assert_eq!(read_engine(zeroed, 0x8002), Err(Refusal::Unmodelled(Why::StackOp, 0x8002)));
    let changed = at_8000(&[0xEE, 0x10, 0xD3, 0xFE, 0xC1, 0xC5, 0x10, 0xFE, 0x06, 0x07, 0x18, 0xF4]);
    assert_eq!(read_engine(changed, 0x8002), Err(Refusal::Unmodelled(Why::StackOp, 0x8002)));
}

// ---- calls, keyboard branches, conditional toggles -------------------------------------------

#[test]
fn reads_a_call_inside_the_signature() {
    // the pass calls its delay: the called routine's code is part of the pass, and of the
    // signature (its bytes pinned by the digest)
    let fx = fixtures::call_in_pass(false);
    let md = model(&fx);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(78)]);
    assert!(md.anchor <= 0x801D && md.code_end >= 0x8021, "{:04X}-{:04X}", md.anchor, md.code_end);
    assert!(md.stack, "the call's return address is on the stack");
    agrees(&fx, &md, 150, 0.0);
    // the called routine changes the counter the pass tests (LD B,C becomes LD E,C): refused
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x801D { 0x59 } else { m.peek(a) };
    assert!(read_engine(mem, 0x8010).is_err(), "{:?}", read_engine(mem, 0x8010));
    // a POP in the called routine takes its return address (LD B,C becomes POP BC): refused
    let mem = |a: u16| if a == 0x801D { 0xC1 } else { m.peek(a) };
    assert!(matches!(read_engine(mem, 0x8010), Err(Refusal::Unmodelled(Why::StackOp, _))), "{:?}", read_engine(mem, 0x8010));
}

#[test]
fn refuses_a_call_outside_the_signature() {
    // the called routine at A000h: one signature cannot cover it and the pass, so its bytes
    // would go unpinned
    let fx = fixtures::call_in_pass(true);
    assert_eq!(read(&fx, 0x8010), Err(Refusal::Unmodelled(Why::CalleeOutsideSignature, 0x8010)));
}

#[test]
fn reads_a_toggle_condition_as_silence() {
    let fx = fixtures::toggle_silence();
    let md = model(&fx);
    assert_eq!(md.channels[0].silent_if_bit, Some((Input::Abs(0x9000), 0x10, true)));
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(40)]);
    agrees(&fx, &md, 150, 0.0);
    // the entry's period program (a full period, x 256): silent while the bit is set
    let m = fixtures::machine(&fx.sna);
    let prog = soundlearn::fit::to_entry(&md, |a| m.peek(a)).progs[0].clone();
    let mut regs = Machine::new().regs();
    regs.bc = 100;
    let m = &m;
    let with = |v: u8| move |a: u16| if a == 0x9000 { v } else { m.peek(a) };
    assert_eq!(soundlearn::eval_prog(&prog, with(0x10), md.anchor, &regs), Some(None));
    assert_eq!(soundlearn::eval_prog(&prog, with(0xEF), md.anchor, &regs), Some(Some(2 * 1340 * 256)));
}

#[test]
fn contended_fixed_read_needs_the_played_fit() {
    // a read of 6000h in the pass: read, its wait left to the fit as played (as a pointer's)
    let fx = fixtures::contended_fixed_read();
    let md = model(&fx);
    assert!(md.pointer_reads);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(71)]);
    agrees(&fx, &md, 150, 0.0);
}

#[test]
fn reads_the_side_an_idle_keyboard_takes() {
    let fx = fixtures::idle_keyboard();
    let md = model(&fx);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(87)]);
    agrees(&fx, &md, 150, 0.0);
    // CPL; AND 1Fh becomes CPL; AND 7Fh: the side then depends on bits an idle keyboard does
    // not fix, and the branch is refused
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x801C { 0x7F } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8010), Err(Refusal::Unmodelled(Why::BranchIdiom, 0x801D)));
}

#[test]
fn reads_a_counter_test_past_a_load() {
    let fx = fixtures::neutral_before_test();
    let md = model(&fx);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(58)]);
    agrees(&fx, &md, 150, 0.0);
    // LD B,C becomes INC B, which sets the flags: not the counter's test
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x8016 { 0x04 } else { m.peek(a) };
    assert!(read_engine(mem, 0x8010).is_err(), "{:?}", read_engine(mem, 0x8010));
}

#[test]
fn reads_a_level_kept_in_memory_between_calls() {
    let fx = fixtures::level_in_memory();
    let md = model(&fx);
    assert_eq!(md.channels[0].half_period, vec![Term::Lin { coef_t: 13, x: Input::Reg(3) }, Term::Const(33)]);
    agrees(&fx, &md, 150, 0.0);
    // the level stored somewhere else (LD (9001h),A): the next note's level is not the last's
    let m = fixtures::machine(&fx.sna);
    let mem = |a: u16| if a == 0x801E { 0x01 } else { m.peek(a) };
    assert_eq!(read_engine(mem, 0x8015), Err(Refusal::Unmodelled(Why::SetupLevel, 0x801D)));
}
