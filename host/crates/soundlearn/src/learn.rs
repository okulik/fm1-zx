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
//! 4. **Judge** each routine ([`judge`](crate::gate::judge)): a routine with a heard site is fitted to that site (the
//!    one heard in the most frames) and, if the fit's gates pass, replayed through method B with
//!    its entry. A learned heard routine then gets the drums the replay shows
//!    ([`drum_ok`], [`add_drum`]), each kept only if the routine, replayed with it, still passes.
//!    A routine never heard goes through the code-only gates.
//! 5. **Observe** each heard routine its code did not give (the reader refused it or a gate did,
//!    not as a built-in entry's), from its site heard in the most frames, if that site sounded in
//!    at least [`OBSERVE_FRAMES`] frames and lies in no routine observed before it
//!    ([`crate::observe`]: the values its loop reads that hold its channels' pitches, by the
//!    spectrum of the speaker); a site that is an `OUT` of a routine learned by observation
//!    before it is skipped, as one of a routine already read. Its entry is an observed one
//!    ([`LearnedEntry::observed`]): every `OUT` of the routine a tonal site of channel 0, the
//!    pass as played (the pitch heard holds the contention), no timing; its signature leaves out
//!    the bytes written while it played and those that differ between the frames it sounds in.
//!    A channel whose input is a register (or read through one) is kept only if it reads the
//!    same at every `OUT` of the routine that sounds in a frame, in at least 95% of the frames
//!    two or more of them sound in (method B reads the registers of the `OUT` that made the
//!    frame's last edge). It is learned if no built-in entry matches inside it and the built-in
//!    table claims none of its frames, it overlaps no routine learned from its code, and its
//!    replay through method B claims at least half of the frames its sites sound in and passes
//!    the music gate.
//! 6. **Keep** at most [`SND_LEARNED_MAX`] learned entries, matching at most
//!    [`SND_LEARNED_WORK_MAX`] site-signature bytes in all: heard ones first, then by the frames
//!    their sites sound in.
//! 7. **Roles:** each kept routine's channels get stored roles ([`crate::roles`]) from its replay,
//!    in its entry's `instrument[]`; each built-in routine the built-in table gave frames to while
//!    listening gets them from those frames, in a kind-1 record ([`BuiltinRoles`]: after the
//!    learned entries, while the list holds fewer than [`SND_LEARNED_MAX`]; none if it has no
//!    role), reported in the block of its `OUT` heard in the most frames, a block refused as a
//!    built-in entry first (none if no block has one of its `OUT`s). The owner's overrides ([`LearnOptions::instruments`]) win, per channel:
//!    the routine named by any of its `OUT`s the report shows. An override that names no such
//!    routine, a routine another key names too, or more channels than the routine has, is
//!    [`GameSound::override_error`].
//! 8. **Scores** ([`crate::scores`]): each heard routine whose `OUT` sounded in at least
//!    [`SCORE_FRAMES`] frames, that no kept entry gives and no built-in entry matches (refused as
//!    one, or its observation refused for one), and whose `OUT` lies in no routine before it (a
//!    learned one, or one taken for a score: its `OUT`s and its span as read or observed), gets
//!    a recorded score for each tune of the run listened to and, when that gives one, of the
//!    extra runs (each key of 1-9, 0, ENTER and SPACE once, but the one of the learner's run,
//!    [`LearnOptions::run`]), which may give scores to `OUT`s heard there only too; each is
//!    verified by a replay of its run with the game's whole list (kind-2 records, after the
//!    roles). An `OUT` heard in the extra runs only says its scores in its block if it has one
//!    (found in the code), else in a block after the others.
//!
//! What it says is in [`crate::report`].

use crate::entry::{encode_list, BuiltinRoles, LearnedEntry};
use crate::fit::{check_fit, digest, to_entry, FitResult};
use crate::gate::{ReplayStats, add_drum, builtin_at, drum_ok, judge_table, replay_table, Verdict, BUILTIN, NOT_REPLAYED};
use crate::listen::{key_runs, listen_observed, run_inputs, Inputs, RunKind, SiteHeard, LISTEN_FRAMES};
use crate::model::{read_engine, scan_out_sites, EngineModel, Input, Refusal};
use crate::observe::{observe, ObservedModel};
use crate::report::{self, ChannelRole, Observation, Routine};
use crate::roles::BuiltinRun;
use crate::scores::{record, Candidate, Tap, SCORE_FRAMES};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::entry::ScoreRecord;
use zx_sys::{Machine, SndEngines, SND_LEARNED_MAX, SND_LEARNED_SITES_MAX, SND_LEARNED_WORK_MAX, SND_SITE_TONAL, SND_TONAL};

/// The learner's version: part of the personal pack's cache key, so bump it when learning
/// changes. Bump it too whenever method B's behaviour changes (`core/sound/engines.c`,
/// `core/sound/learned.c`, the decoder's limits in `core/sound/sound.h`): the learner replays
/// through them and keeps only lists the decoder takes, and the cache key does not cover the core.
pub const LEARNER_VERSION: u32 = 8;

/// Drums tried per learned routine (each costs a replay).
const DRUM_TRIES: usize = 4;
/// Bytes either side of a site the code reader may look at (a site's code is compared over them
/// in the two memories).
const READ_REACH: u16 = 512;
/// The frames a heard routine's site must sound in to be observed.
pub const OBSERVE_FRAMES: u32 = 100;
/// The share of the frames two or more of an observed routine's `OUT`s sound in, in which a
/// channel read through a register must read the same at each of them.
const AGREE: f64 = 0.95;

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
    /// The built-in routines' stored roles (kind-1 records), after the entries in the pack's list.
    pub roles: Vec<BuiltinRoles>,
    /// The recorded scores (kind-2 records), after the roles in the pack's list.
    pub scores: Vec<ScoreRecord>,
    /// An override ([`LearnOptions::instruments`]) that names no routine of the game, or does not
    /// fit the routine: `[game.instruments] "C694": ...`. The personal pack is not built.
    pub override_error: Option<String>,
}

/// How [`learn_with`] learns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LearnOptions {
    /// The built-in entries are not consulted: a routine they match is read, judged and replayed
    /// like any other, through method B with the learned entry alone (tests: a learned entry
    /// measured against the built-in one for the same routine). Never for a pack.
    pub ignore_builtins: bool,
    /// The owner's stored roles: a routine's speaker `OUT` (as the report shows it) to its
    /// channels' roles (`SND_ROLE_*`, 1-3), from channel 0; they win over the analysis.
    pub instruments: BTreeMap<u16, Vec<u8>>,
    /// The learner's run when it is one of [`choose_run`](crate::listen::choose_run)'s: the key
    /// it presses once is not run again for the recorded scores (step 8 of the module docs).
    pub run: Option<RunKind>,
}

/// Learns the music routines of the game `load` gives (time 0 of the run), under `inputs` (see
/// the module docs). Never panics: a panic in the learner is caught and gives
/// `no music learned (learner error: ...)`.
pub fn learn(load: &(dyn Fn() -> Machine + Sync), inputs: &Inputs) -> GameSound {
    learn_with(load, inputs, &LearnOptions::default())
}

/// [`learn`] with options.
pub fn learn_with(load: &(dyn Fn() -> Machine + Sync), inputs: &Inputs, opts: &LearnOptions) -> GameSound {
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
                roles: Vec::new(),
                scores: Vec::new(),
                override_error: None,
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
    let mut not_out = None;
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
            // the site is not an OUT in this memory (its code changed): the other's reason counts
            Err(r @ Refusal::NotSpeaker(_)) => {
                not_out.get_or_insert_with(|| report::refusal(&r));
            }
            Err(r) => {
                first.get_or_insert_with(|| report::refusal(&r));
            }
        }
    }
    Err(first.or(not_out).unwrap_or_else(|| "not read".into()))
}

/// One routine's outcome: its report and, if learned, its entry and whether it was heard.
struct Judged {
    routine: Routine,
    entry: Option<LearnedEntry>,
    /// The frames its heard sites sound in (0 for code only).
    sounding: u32,
}

fn learn_game(load: &(dyn Fn() -> Machine + Sync), inputs: &Inputs, opts: &LearnOptions) -> GameSound {
    let builtins = !opts.ignore_builtins;
    let start = load();
    let mut end = load();
    let mut builtin_run = builtins.then(BuiltinRun::new);
    let mut tap = Tap::default();
    let heard = listen_observed(&mut end, inputs, LISTEN_FRAMES, &mut |m| {
        if let Some(r) = builtin_run.as_mut() {
            r.frame(m);
        }
        tap.frame(m);
    });
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
                let routine = Routine { site: pc, heard: site_heard, model: None, fit: None, replay: None, drums: Vec::new(), outcome: Err(why), roles: Vec::new(), observation: None, scores: Vec::new() };
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
    observe_refused(load, inputs, &heard.sites, &end, &mut done, builtins);

    // the entries kept: heard first, then by sounding frames; at most SND_LEARNED_MAX, and only
    // a list the decoder takes
    let mut order: Vec<usize> = (0..done.len()).filter(|&i| done[i].entry.is_some()).collect();
    order.sort_by_key(|&i| (done[i].routine.heard.is_none(), std::cmp::Reverse(done[i].sounding)));
    let cands: Vec<LearnedEntry> = order.iter().filter_map(|&i| done[i].entry.clone()).collect();
    let mut kept: Vec<usize> = Vec::new();
    for (&i, outcome) in order.iter().zip(keep(&cands)) {
        let r = &mut done[i].routine;
        match (outcome, r.observation.as_mut()) {
            (Ok(()), _) => kept.push(i),
            // an observed entry left out: the code's refusal stands, the observation says why
            (Err(why), Some(o)) => {
                r.outcome = Err(o.code.clone());
                o.outcome = Err(why);
            }
            (Err(why), None) => r.outcome = Err(why),
        }
    }
    let builtin_heard = builtin_run.map_or_else(Vec::new, |r| r.heard());
    let (roles, override_error) = stored_roles(&mut done, &kept, &builtin_heard, &opts.instruments);
    let entries: Vec<LearnedEntry> = kept.iter().filter_map(|&k| done[k].entry.clone()).collect();
    let (cands, groups) = score_candidates(&done);
    let covered = |pc: u16| groups.iter().any(|(outs, spans)| outs.contains(&pc) || spans.iter().any(|&(a, len)| pc.wrapping_sub(a) < len));
    let extra: Vec<(String, Inputs)> = key_runs(opts.run).into_iter().map(|k| (k.short(), run_inputs(k, 0))).collect();
    let got = record(load, inputs, tap, &extra, &cands, &covered, &entries, &roles, builtins);
    let scores = got.scores;
    for (k, o) in got.outcomes {
        done[k].routine.scores.push(o);
    }
    // an OUT heard in the extra runs only: in its block if it has one, else in one after them
    let mut heard_only = Vec::new();
    for (site, o) in got.heard {
        match done.iter_mut().find(|j| j.routine.site == site) {
            Some(j) => j.routine.scores.extend(o),
            None => heard_only.push(report::heard_only_block(site, &o)),
        }
    }
    let learned: Vec<(bool, bool, usize)> = kept
        .iter()
        .map(|&k| {
            let j = &done[k];
            (j.routine.heard.is_some(), j.entry.as_ref().is_some_and(|e| e.observed), j.entry.as_ref().map_or(0, |e| e.progs.len()))
        })
        .collect();
    // the first refusal: a heard routine's, else a code-only routine the reader could read
    let refused = done
        .iter()
        .filter(|j| j.routine.heard.is_some() || j.routine.model.is_some())
        .find_map(|j| report::reason(&j.routine));
    let details = done.iter().map(|j| report::block(&j.routine)).chain(heard_only).collect();
    let voices: Vec<u8> = scores.iter().map(|s| s.voices).collect();
    GameSound { entries, line: report::line(&learned, &voices, refused.as_deref()), details, roles, scores, override_error }
}

/// The routines a score may be recorded for (step 8 of the module docs), in the order of `done`:
/// (block, its `OUT`); and the `OUT`s and spans of those and of the learned routines.
fn score_candidates(done: &[Judged]) -> (Vec<Candidate>, Vec<Group>) {
    let builtin = |r: &Routine| {
        r.outcome.as_ref().is_err_and(|w| w.starts_with(BUILTIN))
            || r.observation.as_ref().is_some_and(|o| o.outcome.as_ref().is_err_and(|w| w.contains("built-in")))
    };
    // the routines before
    let mut groups: Vec<Group> = Vec::new();
    let mut cands = Vec::new();
    for (k, j) in done.iter().enumerate() {
        let r = &j.routine;
        let mut outs: BTreeSet<u16> = [r.site].into();
        let mut spans = Vec::new();
        if let Some(m) = &r.model {
            outs.extend(m.channels.iter().flat_map(|c| c.sites.iter().copied()));
            spans.push((m.anchor, m.code_end.wrapping_sub(m.anchor)));
        }
        if let Some(m) = r.observation.as_ref().and_then(|o| o.model.as_ref()) {
            outs.extend(m.sites.iter().map(|&off| m.anchor.wrapping_add(off)));
            spans.push((m.anchor, m.len));
        }
        let site = r.site;
        let before = groups.iter().any(|(outs, spans)| outs.contains(&site) || spans.iter().any(|&(a, len)| site.wrapping_sub(a) < len));
        let candidate = r.heard.is_some_and(|(frames, _)| frames >= SCORE_FRAMES) && r.outcome.is_err() && !builtin(r);
        if candidate || r.outcome.is_ok() {
            groups.push((outs, spans));
        }
        if candidate && !before {
            cands.push((k, site));
        }
    }
    (cands, groups)
}

/// A routine's `OUT`s, and its spans (start, length).
type Group = (BTreeSet<u16>, Vec<(u16, u16)>);

/// A routine that takes stored roles: its block in `done`, its channels, the analysis' roles and
/// the speaker `OUT`s that name it; a built-in one's index in the built-in table.
struct Takes {
    block: usize,
    channels: usize,
    analysis: Option<[u8; SND_TONAL]>,
    outs: BTreeSet<u16>,
    builtin: Option<usize>,
}

/// The stored roles (step 6 of the module docs): each kept routine's into its entry, each
/// built-in routine's (`heard`) as a kind-1 record, the roles into the routines' blocks; and the
/// first override that does not fit.
fn stored_roles(done: &mut [Judged], kept: &[usize], heard: &[crate::roles::BuiltinHeard], instruments: &BTreeMap<u16, Vec<u8>>) -> (Vec<BuiltinRoles>, Option<String>) {
    let mut takes: Vec<Takes> = Vec::new();
    for &k in kept {
        let r = &done[k].routine;
        let code_outs = r.model.iter().flat_map(|m| m.channels.iter().flat_map(|c| c.sites.iter().copied()));
        let observed = r.observation.as_ref().filter(|o| o.outcome.is_ok());
        let (channels, analysis, outs) = match (observed, &r.model) {
            (Some(o), _) => {
                let m = o.model.as_ref().expect("an observed routine learned has its model");
                let outs = m.sites.iter().map(|&off| m.anchor.wrapping_add(off)).chain(code_outs).chain([r.site]).collect();
                (done[k].entry.as_ref().map_or(0, |e| e.progs.len()), o.replay.as_ref().and_then(|p| p.roles), outs)
            }
            (None, Some(m)) => (m.channels.len(), r.replay.as_ref().and_then(|p| p.roles), code_outs.chain([r.site]).collect()),
            (None, None) => continue,
        };
        takes.push(Takes { block: k, channels, analysis, outs, builtin: None });
    }
    let table = SndEngines::new().entries();
    let shown: BTreeSet<u16> = done.iter().map(|j| j.routine.site).collect();
    for h in heard {
        let blocks: Vec<(u16, &Result<(), String>)> = done.iter().map(|j| (j.routine.site, &j.routine.outcome)).collect();
        let Some(block) = builtin_block(&blocks, &h.outs).filter(|b| takes.iter().all(|t| t.block != *b)) else { continue };
        let outs = h.outs.intersection(&shown).copied().collect();
        takes.push(Takes { block, channels: usize::from(table[h.index].channels), analysis: h.roles, outs, builtin: Some(h.index) });
    }

    // the overrides: each names one routine, not named by another key, with at most its channels
    let mut over: Vec<Option<&Vec<u8>>> = vec![None; takes.len()];
    let mut error = None;
    for (&out, roles) in instruments {
        let why = match takes.iter().position(|t| t.outs.contains(&out)) {
            None => {
                let all: BTreeSet<u16> = takes.iter().flat_map(|t| t.outs.iter().copied()).collect();
                let all: Vec<String> = all.iter().map(|o| format!("{o:04X}")).collect();
                let all = if all.is_empty() { "none".to_string() } else { all.join(", ") };
                format!("no learned or built-in routine of the game has its speaker OUT there (theirs: {all})")
            }
            Some(i) if over[i].is_some() => "names a routine another key names too".to_string(),
            Some(i) if roles.len() > takes[i].channels => format!("{} roles for a routine of {} channel(s)", roles.len(), takes[i].channels),
            Some(i) => {
                over[i] = Some(roles);
                continue;
            }
        };
        error.get_or_insert(format!("[game.instruments] \"{out:04X}\": {why}"));
    }

    let learned = kept.len();
    let mut records = Vec::new();
    for (t, over) in takes.iter().zip(over) {
        let mut roles: Vec<ChannelRole> = (0..t.channels)
            .map(|c| match (over.and_then(|o| o.get(c)), t.analysis) {
                (Some(&r), _) => ChannelRole::Override(r),
                (None, None) => ChannelRole::TooLittle,
                (None, Some(a)) if a[c] == 0 => ChannelRole::NotHeard,
                (None, Some(a)) => ChannelRole::Analysis(a[c]),
            })
            .collect();
        let mut bytes = [0; SND_TONAL];
        for (b, r) in bytes.iter_mut().zip(&roles) {
            *b = r.role();
        }
        match t.builtin {
            None => {
                if let Some(e) = done[t.block].entry.as_mut() {
                    e.instrument = bytes;
                }
            }
            Some(_) if bytes == [0; SND_TONAL] => {}
            Some(_) if learned + records.len() >= SND_LEARNED_MAX => roles.fill(ChannelRole::NoRoom),
            Some(i) => records.push(BuiltinRoles { digest: table[i].digest, channels: table[i].channels, roles: bytes }),
        }
        done[t.block].routine.roles = roles;
    }
    (records, error)
}

/// The block a built-in routine's roles go in, of `blocks` (each block's site and outcome, the
/// OUT heard in the most frames first): of the refused blocks whose site sounded in the
/// built-in's frames (`outs`), the first refused as a built-in entry ([`BUILTIN`]), else the
/// first (a sound effect's OUT may sound in those frames too, and in more of them).
fn builtin_block(blocks: &[(u16, &Result<(), String>)], outs: &BTreeSet<u16>) -> Option<usize> {
    let refused = |&(site, outcome): &(u16, &Result<(), String>)| outcome.is_err() && outs.contains(&site);
    let builtin = |b: &(u16, &Result<(), String>)| b.1.as_ref().is_err_and(|why| why.starts_with(BUILTIN));
    blocks.iter().position(|b| refused(b) && builtin(b)).or_else(|| blocks.iter().position(refused))
}

/// Step 5 of the module docs: each heard routine of `done` its code did not give, observed (in
/// the order of `done`), its block given the observation and, if learned, its entry; a block
/// whose site is an `OUT` of a routine learned by observation before it is dropped. `mem` is
/// the memory at the end of the run (for the built-in entries' signatures).
fn observe_refused(load: &dyn Fn() -> Machine, inputs: &Inputs, heard: &[SiteHeard], mem: &Machine, done: &mut Vec<Judged>, builtins: bool) {
    // the routines learned from their code: anchor and length
    let learned: Vec<(u16, u16)> = done
        .iter()
        .filter(|j| j.entry.is_some())
        .filter_map(|j| j.routine.model.as_ref())
        .map(|m| (m.anchor, m.code_end.wrapping_sub(m.anchor)))
        .collect();
    // the routines observed: anchor, length, the site observed; and their OUTs
    let mut observed: Vec<(u16, u16, u16)> = Vec::new();
    let mut outs: HashSet<u16> = HashSet::new();
    let mut skip = vec![false; done.len()];
    for (k, j) in done.iter_mut().enumerate() {
        let r = &j.routine;
        let (Some((frames, _)), Err(code)) = (r.heard, &r.outcome) else { continue };
        if code.starts_with(BUILTIN) || frames < OBSERVE_FRAMES {
            continue;
        }
        let site = r.site;
        // an OUT of a routine learned by observation already: skipped, as a site of a routine
        // already read
        if outs.contains(&site) {
            skip[k] = true;
            continue;
        }
        let mut o = Observation { code: code.clone(), model: None, dropped: Vec::new(), replay: None, outcome: Ok(()) };
        let got = match observed.iter().find(|&&(a, len, _)| site.wrapping_sub(a) < len) {
            Some(&(_, _, from)) => Err(format!("its OUT lies in the routine observed from the OUT at {from:04X}h")),
            None => match observe(load, inputs, LISTEN_FRAMES, site) {
                Err(why) => Err(why),
                Ok(om) => {
                    observed.push((om.anchor, om.len, site));
                    let got = judge_observed(load, inputs, mem, &om, &learned, builtins, &mut o);
                    o.model = Some(om);
                    got
                }
            },
        };
        match got {
            Ok(entry) => {
                let m = o.model.as_ref().expect("observed");
                outs.extend(m.sites.iter().map(|&off| m.anchor.wrapping_add(off)));
                j.sounding = m.sites.iter().filter_map(|&off| heard.iter().find(|s| s.pc == m.anchor.wrapping_add(off))).map(|s| s.frames_with_edges).sum();
                j.entry = Some(entry);
                j.routine.outcome = Ok(());
            }
            Err(why) => o.outcome = Err(why),
        }
        j.routine.observation = Some(o);
    }
    let mut k = 0;
    done.retain(|_| {
        k += 1;
        !skip[k - 1]
    });
}

/// The verdict on an observed routine `om` (step 5 of the module docs; `learned`: the routines
/// learned from their code, anchor and length), with its entry if learned; `o` gets the
/// channels dropped and the replay.
fn judge_observed(load: &dyn Fn() -> Machine, inputs: &Inputs, mem: &Machine, om: &ObservedModel, learned: &[(u16, u16)], builtins: bool, o: &mut Observation) -> Result<LearnedEntry, String> {
    let outs: Vec<u16> = om.sites.iter().map(|&off| om.anchor.wrapping_add(off)).collect();
    if let Some(b) = outs.iter().chain([&om.anchor]).filter(|_| builtins).find_map(|&pc| builtin_at(mem, pc)) {
        return Err(format!("the {BUILTIN} at {b:04X}h matches in it"));
    }
    let overlaps = |&&(a, len): &&(u16, u16)| om.anchor.wrapping_sub(a) < len || a.wrapping_sub(om.anchor) < om.len;
    if let Some(&(a, _)) = learned.iter().find(overlaps) {
        return Err(format!("it overlaps the routine at {a:04X}h, learned from its code"));
    }
    if om.sites.len() > SND_LEARNED_SITES_MAX {
        return Err(format!("{} OUTs (at most {SND_LEARNED_SITES_MAX})", om.sites.len()));
    }
    let run = observed_run(load, inputs, om);
    let mut keep = Vec::new();
    for (c, (ch, &share)) in om.channels.iter().zip(&run.agree).enumerate() {
        if share >= AGREE {
            keep.push(c);
        } else {
            o.dropped.push((*ch, share));
        }
    }
    if keep.is_empty() {
        return Err("no channel reads the same at all the routine's OUTs".into());
    }
    let Some(code) = run.code else { return Err(format!("the site {:04X}h never sounds", om.site)) };
    let peek = |a: u16| code[usize::from(a.wrapping_sub(om.anchor))];
    let wild: Vec<u16> = om.wild.iter().chain(&run.varies).copied().collect::<BTreeSet<u16>>().into_iter().collect();
    let n = keep.len();
    let entry = LearnedEntry {
        digest: digest(&peek, om.anchor, om.len, &wild, &[]),
        len: om.len,
        wild,
        reloc: Vec::new(),
        sites: om.sites.iter().map(|&off| (off, SND_SITE_TONAL, 0)).collect(),
        pass: 0,
        pos: vec![0; n],
        flips: false,
        observed: true,
        drum: 0,
        stale: None,
        level: vec![200; n],
        instrument: [0; SND_TONAL],
        progs: keep.iter().map(|&c| om.program(c)).collect(),
        timing: None,
    };
    let (stats, _) = replay_table(load, inputs, LISTEN_FRAMES, &entry, builtins);
    o.replay = Some(stats.clone());
    if builtins && stats.builtin_claimed > 0 {
        return Err(format!("replay: the built-in table claims {} of the {} frames it sounds in", stats.builtin_claimed, stats.routine_frames));
    }
    if stats.routine_frames == 0 || stats.claimed * 2 < stats.routine_frames {
        return Err(format!("replay: method B claims {} of the {} frames it sounds in (at least 50%)", stats.claimed, stats.routine_frames));
    }
    if !stats.music_gate {
        return Err("replay: not music (held notes, recurring pitches, no sweeps)".into());
    }
    Ok(entry)
}

/// What one run shows of an observed routine ([`observed_run`]).
struct ObservedRun {
    /// Per channel: the share of the frames two or more of the routine's `OUT`s sound in, in
    /// which its input reads the same at each of them (1 when it reads no register, or no frame
    /// has two).
    agree: Vec<f64>,
    /// The routine's bytes at the end of the last frame its site sounds in (None: it never does).
    code: Option<Vec<u8>>,
    /// The offsets of its bytes that differ between the frames its site sounds in (written by
    /// the music after the traced run's span): wild too.
    varies: Vec<u16>,
}

/// Runs the game for an observed routine `om` (see [`ObservedRun`]): registers are those at each
/// `OUT`'s last edge ([`Machine::out_sites`]), memory the frame's end.
fn observed_run(load: &dyn Fn() -> Machine, inputs: &Inputs, om: &ObservedModel) -> ObservedRun {
    let mut m = load();
    let outs: Vec<u16> = om.sites.iter().map(|&off| om.anchor.wrapping_add(off)).collect();
    let by_reg = |i: &Input| matches!(i, Input::Reg(_) | Input::RegHalf(..) | Input::Indexed(..));
    let (mut frames, mut same) = (0u32, vec![0u32; om.channels.len()]);
    let mut code: Option<Vec<u8>> = None;
    let mut varies = vec![false; usize::from(om.len)];
    for f in 0..LISTEN_FRAMES {
        let (rows, kemp) = inputs(f);
        m.set_keys(rows);
        m.set_kempston(kemp);
        m.run_frame();
        if m.edges().iter().any(|e| e.pc == om.site) {
            let now: Vec<u8> = (0..om.len).map(|i| m.peek(om.anchor.wrapping_add(i))).collect();
            if let Some(before) = &code {
                for (v, (a, b)) in varies.iter_mut().zip(now.iter().zip(before)) {
                    *v |= a != b;
                }
            }
            code = Some(now);
        }
        if outs.len() < 2 {
            continue;
        }
        let at: Vec<_> = m.out_sites().into_iter().filter(|s| outs.contains(&s.pc)).collect();
        if at.len() < 2 {
            continue;
        }
        frames += 1;
        let peek = |a: u16| m.peek(a);
        for (n, ch) in same.iter_mut().zip(&om.channels) {
            let v: Vec<Option<u32>> = at.iter().map(|s| ch.input.read(&peek, om.anchor, &s.regs)).collect();
            *n += u32::from(!by_reg(&ch.input) || v.windows(2).all(|w| w[0] == w[1]));
        }
    }
    let agree = same.iter().map(|&n| if frames == 0 { 1.0 } else { f64::from(n) / f64::from(frames) }).collect();
    let varies = (0..om.len).filter(|&i| varies[usize::from(i)]).collect();
    ObservedRun { agree, code, varies }
}

/// Which of the learned entries `cands` (in order of preference) a game's list keeps: each in
/// turn while the list holds fewer than [`SND_LEARNED_MAX`], their matching's work
/// ([`LearnedEntry::work`]) stays within [`SND_LEARNED_WORK_MAX`] and method B's decoder takes
/// the list; else the reason it is left out.
fn keep(cands: &[LearnedEntry]) -> Vec<Result<(), String>> {
    let mut list: Vec<LearnedEntry> = Vec::new();
    cands
        .iter()
        .map(|e| {
            if list.len() >= SND_LEARNED_MAX {
                return Err(format!("only {SND_LEARNED_MAX} learned routines a game: heard ones first, then those sounding longest"));
            }
            if list.iter().chain([e]).map(LearnedEntry::work).sum::<usize>() > SND_LEARNED_WORK_MAX {
                return Err(format!(
                    "the game's learned routines would match over {SND_LEARNED_WORK_MAX} signature bytes (sites x length): heard ones first, then those sounding longest"
                ));
            }
            list.push(e.clone());
            if SndEngines::with_learned(&encode_list(&list)).is_err() {
                list.pop();
                return Err("method B's decoder refuses its entry".into());
            }
            Ok(())
        })
        .collect()
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
        roles: Vec::new(),
        observation: None,
        scores: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::{builtin_block, keep};
    use crate::gate::BUILTIN;
    use crate::{LearnedEntry, Op};
    use std::collections::BTreeSet;
    use zx_sys::SND_SITE_TONAL;

    #[test]
    fn builtin_roles_go_to_the_builtin_block() {
        // a sound effect's OUT (refused: a sweep) heard in more frames than the built-in's, both
        // sounding in the built-in's frames: the roles go to the block refused as the built-in
        let sweep = Err("a sweep, not music".to_string());
        let builtin = Err(format!("{BUILTIN} at 03B5h"));
        let learned = Ok(());
        let outs: BTreeSet<u16> = [0x9000, 0x03D1, 0x8014].into();
        let blocks = [(0x8014, &learned), (0x9000, &sweep), (0x03D1, &builtin)];
        assert_eq!(builtin_block(&blocks, &outs), Some(2));
        // the judge's refusal (no address) marks a built-in entry too
        let judged = Err(BUILTIN.to_string());
        assert_eq!(builtin_block(&[(0x9000, &sweep), (0x03D1, &judged)], &outs), Some(1));
        // no block refused as a built-in: the first refused block that sounded in its frames
        assert_eq!(builtin_block(&[(0x8014, &learned), (0x7000, &sweep), (0x9000, &sweep)], &outs), Some(2));
        // a built-in block whose site did not sound in its frames is not taken
        assert_eq!(builtin_block(&[(0x9000, &sweep), (0x1234, &builtin)], &outs), Some(0));
        assert_eq!(builtin_block(&[(0x8014, &learned)], &outs), None);
    }

    fn sized(len: u16, n: u16) -> LearnedEntry {
        LearnedEntry {
            digest: u64::from(len) << 8 | u64::from(n),
            len,
            wild: vec![],
            reloc: vec![],
            sites: (0..n).map(|k| (k, SND_SITE_TONAL, 0)).collect(),
            pass: 0,
            pos: vec![0],
            flips: false,
            observed: false,
            drum: 0,
            stale: None,
            level: vec![200],
            instrument: [0; 3],
            progs: vec![vec![Op::K(300)]],
            timing: None,
        }
    }

    #[test]
    fn keeps_the_matching_work_bounded() {
        // 2048 + 1536 kept; 1024 more would be 4608: left out; a smaller one after it still fits
        let got = keep(&[sized(512, 4), sized(512, 3), sized(512, 2), sized(256, 2)]);
        assert_eq!(got[..2], [Ok(()), Ok(())]);
        assert!(matches!(&got[2], Err(why) if why.contains("4096 signature bytes")), "{got:?}");
        assert_eq!(got[3], Ok(()));
        // the entry count's bound still holds
        let got = keep(&vec![sized(16, 1); 9]);
        assert!(got[..8].iter().all(Result::is_ok) && got[8].is_err(), "{got:?}");
    }
}
