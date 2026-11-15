// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The learner for one game ([`learn`]): listening, reading the code, the gates, and the
//! entries kept.
//!
//! 1. **Listen** to the game for [`LISTEN_FRAMES`] under the inputs.
//! 2. **Collect the sites:** the heard ones (heard in the most frames first), then the candidate
//!    speaker `OUT`s in memory ([`scan_out_sites`]) at frame 0 and at the end of the run.
//! 3. **Read** each site's routine: a site inside a built-in entry's routine is refused at once
//!    ([`builtin_at`]); a site of a routine already read is skipped, and so is a routine whose
//!    start was read from another site. A site is read in the memory at the end of the run, and
//!    in the memory at frame 0 if that fails and the code there differs (a site found only at
//!    frame 0: there only).
//! 4. **Judge** each routine ([`judge`]): a routine with a heard site is fitted to that site (the
//!    one heard in the most frames) and, if the fit's gates pass, replayed through method B with
//!    its entry. A learned heard routine then gets the drums the replay shows
//!    ([`drum_ok`], [`add_drum`]), each kept only if the routine, replayed with it, still passes.
//!    A routine never heard goes through the code-only gates.
//! 5. **Keep** at most [`SND_LEARNED_MAX`] learned entries: heard ones first, then by the frames
//!    their sites sound in.
//!
//! What it says is in [`crate::report`].

use crate::entry::{encode_list, LearnedEntry};
use crate::fit::{check_fit, to_entry, FitResult};
use crate::gate::{ReplayStats, add_drum, builtin_at, drum_ok, judge_table, replay_table, Verdict, BUILTIN, NOT_REPLAYED};
use crate::listen::{listen, Inputs, SiteHeard, LISTEN_FRAMES};
use crate::model::{read_engine, scan_out_sites, EngineModel};
use crate::report::{self, Routine};
use std::collections::{BTreeSet, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use zx_sys::{Machine, SndEngines, SND_LEARNED_MAX};

/// The learner's version: part of the personal pack's cache key, so bump it when learning
/// changes.
pub const LEARNER_VERSION: u32 = 2;

/// Drums tried per learned routine (each costs a replay).
const DRUM_TRIES: usize = 4;
/// Bytes either side of a site the code reader may look at (a site's code is compared over them
/// in the two memories).
const READ_REACH: u16 = 512;

/// What the learner made of a game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSound {
    /// The learned entries, at most [`SND_LEARNED_MAX`], in the order the pack holds them.
    pub entries: Vec<LearnedEntry>,
    /// The summary, without the game's name (`music learned (1 routine, heard, 1 channel)`);
    /// the personal pack prints it after the name and a colon.
    pub line: String,
    /// One block per routine ([`report::block`]): indented lines, each ending in a line break.
    pub details: String,
}

/// How [`learn_with`] learns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LearnOptions {
    /// The built-in entries are not consulted: a routine they match is read, judged and replayed
    /// like any other, through method B with the learned entry alone (tests: a learned entry
    /// measured against the built-in one for the same routine). Never for a pack.
    pub ignore_builtins: bool,
}

/// Learns the music routines of the game `load` gives (time 0 of the run), under `inputs` (see
/// the module docs). Never panics: a panic in the learner is caught and gives
/// `no music learned (learner error: ...)`.
pub fn learn(load: &dyn Fn() -> Machine, inputs: &Inputs) -> GameSound {
    learn_with(load, inputs, &LearnOptions::default())
}

/// [`learn`] with options.
pub fn learn_with(load: &dyn Fn() -> Machine, inputs: &Inputs, opts: &LearnOptions) -> GameSound {
    match catch_unwind(AssertUnwindSafe(|| learn_game(load, inputs, opts))) {
        Ok(s) => s,
        Err(p) => {
            let why = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".into());
            GameSound {
                entries: Vec::new(),
                line: format!("no music learned (learner error: {why})"),
                details: format!("  learner error: {why}\n"),
            }
        }
    }
}

/// Whether the code within the reader's reach of `pc` is the same in both memories.
fn same_code(a: &Machine, b: &Machine, pc: u16) -> bool {
    (0..=2 * READ_REACH).all(|i| {
        let x = pc.wrapping_sub(READ_REACH).wrapping_add(i);
        a.peek(x) == b.peek(x)
    })
}

/// A site's routine: the built-in entry it lies in, or the model read from the first of `mems`
/// that gives one (a later one only where its code differs). Err: the first refusal.
fn read_site<'m>(pc: u16, mems: &[&'m Machine], builtins: bool) -> Result<(EngineModel, &'m Machine), String> {
    let mut first = None;
    for (i, &m) in mems.iter().enumerate() {
        if i > 0 && same_code(mems[0], m, pc) {
            continue;
        }
        if let Some(b) = builtin_at(m, pc).filter(|_| builtins) {
            first.get_or_insert_with(|| format!("{BUILTIN} at {b:04X}h"));
            break;
        }
        match read_engine(|a| m.peek(a), pc) {
            Ok(model) => return Ok((model, m)),
            Err(r) => {
                first.get_or_insert_with(|| report::refusal(&r));
            }
        }
    }
    Err(first.unwrap_or_else(|| "not read".into()))
}

/// One routine's outcome: its report and, if learned, its entry and whether it was heard.
struct Judged {
    routine: Routine,
    entry: Option<LearnedEntry>,
    /// The frames its heard sites sound in (0 for code only).
    sounding: u32,
}

fn learn_game(load: &dyn Fn() -> Machine, inputs: &Inputs, opts: &LearnOptions) -> GameSound {
    let builtins = !opts.ignore_builtins;
    let start = load();
    let mut end = load();
    let heard = listen(&mut end, inputs, LISTEN_FRAMES);
    let mut sites: Vec<&SiteHeard> = heard.sites.iter().collect();
    sites.sort_by_key(|s| (std::cmp::Reverse(s.frames_with_edges), s.pc));
    let heard_at = |pc: u16| heard.sites.iter().find(|s| s.pc == pc);

    // the candidates: heard sites first, then the code's (from the end memory, then frame 0's)
    let at_end: BTreeSet<u16> = scan_out_sites(|a| end.peek(a)).into_iter().collect();
    let at_start: BTreeSet<u16> = scan_out_sites(|a| start.peek(a)).into_iter().collect();
    let mut candidates: Vec<(u16, Vec<&Machine>)> = sites.iter().map(|s| (s.pc, vec![&end, &start])).collect();
    for &pc in at_end.union(&at_start) {
        if heard_at(pc).is_none() {
            candidates.push((pc, if at_end.contains(&pc) { vec![&end, &start] } else { vec![&start] }));
        }
    }

    let mut done: Vec<Judged> = Vec::new();
    let mut covered: HashSet<u16> = HashSet::new();
    let mut anchors: HashSet<u16> = HashSet::new();
    for (pc, mems) in candidates {
        if covered.contains(&pc) {
            continue;
        }
        let site_heard = heard_at(pc).map(|s| (s.frames_with_edges, s.edges));
        let (model, mem) = match read_site(pc, &mems, builtins) {
            Ok(got) => got,
            Err(why) => {
                let routine = Routine { site: pc, heard: site_heard, model: None, fit: None, replay: None, drums: Vec::new(), outcome: Err(why) };
                done.push(Judged { routine, entry: None, sounding: 0 });
                continue;
            }
        };
        covered.extend(model.channels.iter().flat_map(|c| c.sites.iter().copied()));
        if !anchors.insert(model.anchor) {
            continue;
        }
        done.push(judge_routine(load, inputs, &heard.sites, pc, model, mem, builtins));
    }

    // the entries kept: heard first, then by sounding frames; at most SND_LEARNED_MAX, and only
    // a list the decoder takes
    let mut order: Vec<usize> = (0..done.len()).filter(|&i| done[i].entry.is_some()).collect();
    order.sort_by_key(|&i| (done[i].routine.heard.is_none(), std::cmp::Reverse(done[i].sounding)));
    let mut kept: Vec<usize> = Vec::new();
    for i in order {
        if kept.len() >= SND_LEARNED_MAX {
            done[i].routine.outcome = Err(format!("only {SND_LEARNED_MAX} learned routines a game: heard ones first, then those sounding longest"));
            continue;
        }
        let list: Vec<LearnedEntry> = kept.iter().chain([&i]).filter_map(|&k| done[k].entry.clone()).collect();
        if SndEngines::with_learned(&encode_list(&list)).is_ok() {
            kept.push(i);
        } else {
            done[i].routine.outcome = Err("method B's decoder refuses its entry".into());
        }
    }
    let entries: Vec<LearnedEntry> = kept.iter().filter_map(|&k| done[k].entry.clone()).collect();
    let learned: Vec<(bool, usize)> = kept.iter().map(|&k| (done[k].routine.heard.is_some(), done[k].entry.as_ref().map_or(0, |e| e.progs.len()))).collect();
    // the first refusal: a heard routine's, else a code-only routine the reader could read
    let refused = done
        .iter()
        .filter(|j| j.routine.heard.is_some() || j.routine.model.is_some())
        .find_map(|j| report::reason(&j.routine));
    let details = done.iter().map(|j| report::block(&j.routine)).collect();
    GameSound { entries, line: report::line(&learned, refused.as_deref()), details }
}

/// The verdict on a routine read from `mem` at `pc`, with its entry if learned.
fn judge_routine(load: &dyn Fn() -> Machine, inputs: &Inputs, heard: &[SiteHeard], pc: u16, model: EngineModel, mem: &Machine, builtins: bool) -> Judged {
    let peek = |a: u16| mem.peek(a);
    let judge = |model: &EngineModel, mem: &Machine, heard: Option<&SiteHeard>, replay: Option<&ReplayStats>, fit: Option<&FitResult>| {
        judge_table(model, mem, heard, replay, fit, builtins)
    };
    // the routine's site heard in the most frames
    let site = model
        .channels
        .iter()
        .flat_map(|c| c.sites.iter())
        .filter_map(|&p| heard.iter().find(|s| s.pc == p))
        .max_by_key(|s| (s.frames_with_edges, std::cmp::Reverse(s.pc)));
    let mut routine = Routine {
        site: site.map_or(pc, |s| s.pc),
        heard: site.map(|s| (s.frames_with_edges, s.edges)),
        model: None,
        fit: None,
        replay: None,
        drums: Vec::new(),
        outcome: Ok(()),
    };
    let sounding = model.channels.iter().flat_map(|c| c.sites.iter()).filter_map(|&p| heard.iter().find(|s| s.pc == p)).map(|s| s.frames_with_edges).sum();
    let mut entry = to_entry(&model, peek);
    let verdict = match site {
        None => judge(&model, mem, None, None, None),
        Some(site) => {
            let fit = check_fit(&model, site);
            routine.fit = Some(fit.clone());
            match judge(&model, mem, Some(site), None, fit.as_ref().ok()) {
                Verdict::Refused(why) if why == NOT_REPLAYED => {
                    let (stats, others) = replay_table(load, inputs, LISTEN_FRAMES, &entry, builtins);
                    let v = judge(&model, mem, Some(site), Some(&stats), fit.as_ref().ok());
                    routine.replay = Some(stats);
                    if v == (Verdict::Learned { heard: true }) {
                        // drums: other routines' noise between the claimed notes
                        let mut anchor = model.anchor;
                        let mut tried = 0;
                        let mut others = others;
                        others.sort_by_key(|o| (std::cmp::Reverse(o.frames), o.pc));
                        for o in &others {
                            if tried >= DRUM_TRIES {
                                break;
                            }
                            let other_fit = heard
                                .iter()
                                .find(|s| s.pc == o.pc)
                                .and_then(|s| read_engine(peek, o.pc).ok().and_then(|om| check_fit(&om, s).ok()));
                            if !drum_ok(o, other_fit.as_ref()) {
                                continue;
                            }
                            let Some(grown) = add_drum(&entry, anchor, o.pc, peek) else { continue };
                            tried += 1;
                            let stats = replay_table(load, inputs, LISTEN_FRAMES, &grown, builtins).0;
                            if judge(&model, mem, Some(site), Some(&stats), fit.as_ref().ok()) == v {
                                // add_drum moves the anchor back to a drum before it
                                if o.pc.wrapping_sub(anchor) >= 0x8000 {
                                    anchor = o.pc;
                                }
                                entry = grown;
                                routine.drums.push(o.pc);
                                routine.replay = Some(stats);
                            }
                        }
                    }
                    v
                }
                v => v,
            }
        }
    };
    if let Verdict::Refused(why) = verdict {
        routine.outcome = Err(why);
    }
    routine.model = Some(model);
    let entry = routine.outcome.is_ok().then_some(entry);
    Judged { routine, entry, sounding }
}
