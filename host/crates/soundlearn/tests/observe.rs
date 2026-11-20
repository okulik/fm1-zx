// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The observing learner on generated multi-channel engines: a two-channel counter engine of
//! pin pulses (its reloads self-modified immediates) and a two-channel phase-accumulator engine
//! (its steps in DE and DE'), each playing a note table whose notes last 3-7 frames. The code
//! runs at 8000h with interrupts off (uncontended RAM: only the `OUT`'s own I/O contention).

mod fixtures;

use fixtures::{accumulator_engine, counter_engine, Asm};
use fixtures::bare as machine;
use soundlearn::listen::Inputs;
use soundlearn::model::Input;
use soundlearn::observe::{observe, Mapping, ObservedModel};
use soundlearn::prog::eval_prog;
use zx_sys::Machine;

fn quiet() -> Inputs {
    Box::new(|_| ([0; 8], 0))
}

const FRAMES: u32 = 1500;

fn show(om: &ObservedModel) -> String {
    let mut s = format!("anchor {:04X}+{} pass {:.1} sites {:?} wild {:?}\n", om.anchor, om.len, om.pass_t, om.sites, om.wild);
    for c in &om.considered {
        s += &format!("  {:?} {:?} score {:.2} clean {:.2} windows {}\n", c.input, c.mapping, c.score, c.clean, c.windows);
    }
    s
}

/// Each channel's program, evaluated on the machine's memory and the site's registers at the
/// end of each frame for a while, gives the period the note table says (`want`, T).
fn programs_play_the_table(om: &ObservedModel, code: &[u8], want: &dyn Fn(&Machine, usize) -> f64) {
    let mut m = machine(code);
    let mut checked = 0;
    for _ in 0..300 {
        m.run_frame();
        let Some(site) = m.out_sites().into_iter().find(|s| s.pc == om.site) else { continue };
        for ch in 0..om.channels.len() {
            let prog = om.program(ch);
            let got = eval_prog(&prog, |a| m.peek(a), om.anchor, &site.regs).expect("valid").expect("not silent");
            let got = f64::from(got) / 256.0;
            let w = want(&m, ch);
            // the pass as played holds the OUT's average contention: a few T in 110
            assert!((got / w - 1.0).abs() < 0.03, "channel {ch}: {got} T, the table says {w}");
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked}");
}

#[test]
fn counter_engine_channels_are_its_reloads() {
    let (code, l, notes) = counter_engine();
    let load = || machine(&code);
    let site = l["out"];
    let om = observe(&load, &quiet(), FRAMES, site).unwrap_or_else(|e| panic!("refused: {e}"));
    eprintln!("{}", show(&om));
    assert_eq!(om.anchor, l["loop"], "{}", show(&om));
    assert_eq!(om.len, l["table"] - l["loop"], "{}", show(&om));
    assert!(om.pass_t >= 110.0 && om.pass_t < 114.0, "{}", om.pass_t);
    assert_eq!(om.sites, vec![site - om.anchor]);
    let (o1, o2) = (l["r1"] + 1 - om.anchor, l["r2"] + 1 - om.anchor);
    assert_eq!(om.wild, vec![o1, o2]);
    let mut got: Vec<(Input, Mapping)> = om.channels.iter().map(|c| (c.input, c.mapping)).collect();
    got.sort();
    assert_eq!(got, vec![(Input::Rel(o1), Mapping::Counter { mult: 1 }), (Input::Rel(o2), Mapping::Counter { mult: 1 })], "{}", show(&om));
    assert!(om.channels.iter().all(|c| c.score > 0.9), "{}", show(&om));
    // the table's notes: whatever note is playing, its reloads are in the immediates
    let notes1: Vec<u8> = notes.iter().map(|n| n.0).collect();
    programs_play_the_table(&om, &code, &|m, ch| {
        let v = m.peek(om.anchor + if om.channels[ch].input == Input::Rel(o1) { o1 } else { o2 });
        assert!(notes1.contains(&v) || notes.iter().any(|n| n.1 == v));
        110.0 * f64::from(v)
    });
}

#[test]
fn accumulator_engine_channels_are_its_steps() {
    let (code, l, notes) = accumulator_engine();
    let load = || machine(&code);
    let site = l["out1"];
    let om = observe(&load, &quiet(), FRAMES, site).unwrap_or_else(|e| panic!("refused: {e}"));
    eprintln!("{}", show(&om));
    assert_eq!(om.anchor, l["loop"], "{}", show(&om));
    assert!(om.pass_t >= 108.0 && om.pass_t < 112.0, "{}", om.pass_t);
    assert_eq!(om.sites, vec![l["out1"] - om.anchor, l["out2"] - om.anchor]);
    let mut got: Vec<(Input, Mapping)> = om.channels.iter().map(|c| (c.input, c.mapping)).collect();
    got.sort();
    assert_eq!(got, vec![(Input::Reg(9), Mapping::Accumulator { bits: 16 }), (Input::Reg(16), Mapping::Accumulator { bits: 16 })], "{}", show(&om));
    assert!(om.channels.iter().all(|c| c.score > 0.9), "{}", show(&om));
    programs_play_the_table(&om, &code, &|m, ch| {
        let r = m.out_sites().into_iter().find(|s| s.pc == om.site).expect("sounds").regs;
        let step = if om.channels[ch].input == Input::Reg(9) { r.de } else { r.de_ };
        assert!(notes.iter().any(|n| n.0 == step || n.1 == step), "{step}");
        108.0 * 65536.0 / f64::from(step)
    });
}

/// A routine whose sound comes from no value it reads: noise from a shift register, its
/// "notes" a value it reads but does not play.
///
/// ```text
/// start: DI ; LD HL,1 ; LD IX,table
/// next:  LD A,(IX+0) ; OR A ; JR Z,start ; LD (k+1),A ; INC IX ; LD DE,passes
/// loop:  ADD HL,HL ; SBC A,A ; AND 2Dh ; XOR L ; LD L,A ; AND 16 ; OUT (254),A
/// k:     LD A,n ; AND 0 ; ADD A,C ; LD C,A
///        DEC DE ; LD A,D ; OR E ; JP NZ,loop ; JP next
/// ```
#[test]
fn noise_is_refused() {
    let mut a = Asm::default();
    a.label("start").b(&[0xF3, 0x21, 0x01, 0x00]).w(&[0xDD, 0x21], "table");
    a.label("next").b(&[0xDD, 0x7E, 0x00, 0xB7]).r(0x28, "start").w(&[0x32], "k+1").b(&[0xDD, 0x23, 0x11, 0x00, 0x0C]);
    a.label("loop").b(&[0x29, 0x9F, 0xE6, 0x2D, 0xAD, 0x6F, 0xE6, 0x10]);
    a.label("out").b(&[0xD3, 0xFE]);
    a.label("k").b(&[0x3E, 0x00, 0xE6, 0x00, 0x81, 0x4F, 0x1B, 0x7A, 0xB3]).w(&[0xC2], "loop").w(&[0xC3], "next");
    a.label("table").b(&[90, 120, 60, 200, 150, 75, 0]);
    let k = a.at("k");
    a.labels.insert("k+1", k + 1);
    let site = a.at("out");
    let (code, _) = a.finish();
    let load = || machine(&code);
    let r = observe(&load, &quiet(), 600, site);
    assert!(r.as_ref().is_err_and(|e| e.contains("no value")), "{r:?}");
}

#[test]
fn a_site_that_never_sounds_is_refused() {
    let (code, l, _) = counter_engine();
    let load = || machine(&code);
    let r = observe(&load, &quiet(), 200, l["next"]);
    assert!(r.as_ref().is_err_and(|e| e.contains("never sounds")), "{r:?}");
}

#[test]
fn observing_is_fast() {
    let (code, l, _) = counter_engine();
    let load = || machine(&code);
    let t = std::time::Instant::now();
    observe(&load, &quiet(), 9000, l["out"]).expect("observed");
    let secs = t.elapsed().as_secs_f64();
    eprintln!("observing_is_fast: {secs:.2} s for 9000 frames");
    assert!(secs < 5.0, "{secs} s");
}
