// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The live rule (`core/sound/roles.c`): each tonal voice gets its role by its pitch, ranked
//! against the voices that sounded recently (the lowest bass, the highest lead, a third the pad;
//! two voices bass and lead; a voice alone lead), smoothed so a brief crossing or a rest keeps the
//! roles; a role a frame already carries is kept. The mixer runs it on the frames the synth
//! renders, afresh at each FM turn-on, and the synth takes a role only when a note starts.

use music::{colony8, period_q8, RATE};
use zx_sys::{
    Beeper, Machine, SndEngines, SndFrame, SndMixer, SndRoles, SndVoice, SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B, SND_ROLE_AUTO, SND_ROLE_BASS,
    SND_ROLE_LEAD, SND_ROLE_PAD, SND_TONAL,
};

const B: u8 = SND_ROLE_BASS;
const L: u8 = SND_ROLE_LEAD;
const P: u8 = SND_ROLE_PAD;

/// Frames per second (the decisions come one per 20 ms frame).
const FPS: usize = 50;

/// A confident frame with the tonal voices at `hz` (0: off), level 255, every role AUTO.
fn voices(hz: [f64; SND_TONAL]) -> SndFrame {
    let mut fr = SndFrame { confident: 1, ..Default::default() };
    for (v, &f) in fr.tone.iter_mut().zip(&hz) {
        if f > 0.0 {
            *v = SndVoice { period_q8: period_q8(f), level: 255 };
        }
    }
    fr.role = [SND_ROLE_AUTO; SND_TONAL];
    fr
}

/// Runs `frames` frames of `hz` through `r`; returns the roles each frame got.
fn run(r: &mut SndRoles, hz: [f64; SND_TONAL], frames: usize) -> Vec<[u8; SND_TONAL]> {
    (0..frames)
        .map(|_| {
            let mut fr = voices(hz);
            r.frame(&mut fr);
            fr.role
        })
        .collect()
}

/// The roles of the voices sounding in `hz` (`-` for a silent one).
fn sounding(roles: [u8; SND_TONAL], hz: [f64; SND_TONAL]) -> String {
    roles.iter().zip(&hz).map(|(&r, &f)| if f > 0.0 { ["?", "B", "L", "P"][r as usize] } else { "-" }).collect()
}

/// Every frame of `got` has the roles `want` on the voices sounding in `hz`.
fn all(got: &[[u8; SND_TONAL]], hz: [f64; SND_TONAL], want: &str, what: &str) {
    for (i, &g) in got.iter().enumerate() {
        assert_eq!(sounding(g, hz), want, "{what}: frame {i}");
    }
}

#[test]
fn three_voices_ranked() {
    // ranked from the first frame (the first notes by their own pitch) and held there, whatever
    // order the voices come in
    for (hz, want) in [([110.0, 440.0, 880.0], "BPL"), ([880.0, 110.0, 440.0], "LBP"), ([440.0, 880.0, 110.0], "PLB")] {
        let mut r = SndRoles::new();
        all(&run(&mut r, hz, 5 * FPS), hz, want, &format!("{hz:?}"));
    }
}

#[test]
fn two_voices_bass_and_lead() {
    for (hz, want) in [([660.0, 0.0, 220.0], "L-B"), ([0.0, 110.0, 130.0], "-BL"), ([300.0, 600.0, 0.0], "BL-")] {
        let mut r = SndRoles::new();
        all(&run(&mut r, hz, 5 * FPS), hz, want, &format!("{hz:?}"));
    }
}

#[test]
fn lone_voice_is_lead() {
    for hz in [[110.0, 0.0, 0.0], [0.0, 60.0, 0.0], [0.0, 0.0, 2000.0]] {
        let mut r = SndRoles::new();
        let want: String = hz.iter().map(|&f| if f > 0.0 { "L" } else { "-" }).collect();
        all(&run(&mut r, hz, 5 * FPS), hz, &want, &format!("{hz:?}"));
    }
    // a bass whose partners stop: bass while they sounded recently, then lead, alone
    let mut r = SndRoles::new();
    run(&mut r, [110.0, 440.0, 880.0], 5 * FPS);
    let alone = [110.0, 0.0, 0.0];
    let got = run(&mut r, alone, 10 * FPS);
    let first_lead = got.iter().position(|g| g[0] == L).expect("the bass alone becomes lead");
    println!("alone: lead after {first_lead} frames");
    all(&got[..first_lead], alone, "B--", "partners recent");
    all(&got[first_lead..], alone, "L--", "alone");
    assert!((3 * FPS..=5 * FPS).contains(&first_lead), "lead after {first_lead} frames: the window is a few seconds");
}

#[test]
fn crossing_bass_keeps_role() {
    // the bass climbs above the lead for 1 s, a note per 100 ms, and comes back: no role changes
    let mut r = SndRoles::new();
    let base = [110.0, 330.0, 660.0];
    all(&run(&mut r, base, 5 * FPS), base, "BPL", "before");
    for k in 0..10 {
        let hz = [700.0 + 20.0 * k as f64, 330.0, 660.0];
        all(&run(&mut r, hz, FPS / 10), hz, "BPL", "the bass above the lead");
    }
    all(&run(&mut r, base, 5 * FPS), base, "BPL", "after");
}

#[test]
fn rest_longer_than_window_keeps_role() {
    // the bass rests 6 s (longer than the window: the two others are bass and lead meanwhile)
    // and comes back on a high note, between the pad and the lead: it is the bass again at
    // once, its history kept (a first note ranked by its own pitch would be the pad)
    let mut r = SndRoles::new();
    let base = [110.0, 330.0, 660.0];
    all(&run(&mut r, base, 5 * FPS), base, "BPL", "before");
    let rest = [0.0, 330.0, 660.0];
    let got = run(&mut r, rest, 6 * FPS);
    assert_eq!(sounding(got[0], rest), "-PL", "the bass just rested");
    assert_eq!(sounding(got[got.len() - 1], rest), "-BL", "the bass out of the ranking");
    let back = [400.0, 330.0, 660.0];
    all(&run(&mut r, back, FPS), back, "BPL", "back");
}

#[test]
fn stored_role_respected() {
    // voice 1 carries a stored role (pad, though it is the lowest): it keeps it, the others are
    // ranked among themselves (two: bass and lead); its pitch is tracked all the same, so when
    // the stored role stops it is the bass at once. Voice 1 lies within the margin of voice 0
    // (400 against 440 Hz): untracked, it would be new when the stored role stops and stay above
    // voice 0 (the pad), so only its tracked history makes it the bass
    let mut r = SndRoles::new();
    let hz = [440.0, 400.0, 880.0];
    for i in 0..5 * FPS {
        let mut fr = voices(hz);
        fr.role[1] = P;
        r.frame(&mut fr);
        assert_eq!(fr.role, [B, P, L], "frame {i}");
    }
    let all_auto = run(&mut r, hz, FPS);
    all(&all_auto, hz, "PBL", "stored role gone");
}

#[test]
fn newcomer_ranked_by_own_pitch() {
    // a melody plays alone 5 s, then a harmony joins 2 or 2.5 semitones below it (within the
    // margin): the newcomer's first notes are ranked by their own pitch, so it is the lower role
    // and the melody keeps the lead; with a bass already playing, the newcomer is the pad
    for semis in [2.0, 2.5] {
        let below = 440.0 * f64::powf(2.0, -semis / 12.0);
        let mut r = SndRoles::new();
        let alone = [440.0, 0.0, 0.0];
        all(&run(&mut r, alone, 5 * FPS), alone, "L--", "the melody alone");
        let joined = [440.0, below, 0.0];
        all(&run(&mut r, joined, 5 * FPS), joined, "LB-", &format!("{semis} semitones below joins"));

        let mut r = SndRoles::new();
        let two = [440.0, 0.0, 110.0];
        all(&run(&mut r, two, 5 * FPS), two, "L-B", "melody and bass");
        let joined = [440.0, below, 110.0];
        all(&run(&mut r, joined, 5 * FPS), joined, "LPB", &format!("{semis} semitones below joins the two"));
    }
}

// ---- in the mixer, on the corpus ----------------------------------------------------------------

fn max_step(s: &[i16]) -> i32 {
    s.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0)
}

/// Colony 8 through a mixer in the mode `mode_of(frame)`, the machine given to it as the
/// firmware does: per frame its output, the mixer's live roles (and each slot's frames heard) and
/// its synth's voices' roles, and whether it plays FM.
struct Run {
    out: Vec<Vec<i16>>,
    live: Vec<[u8; SND_TONAL]>,
    avg: Vec<[i32; SND_TONAL]>,
    heard: Vec<[u8; SND_TONAL]>,
    synth: Vec<[u8; SND_TONAL]>,
    fm_on: Vec<bool>,
    beeper: Vec<Vec<i16>>,
}

fn mix_colony8(frames: usize, mode_of: impl Fn(usize) -> u8) -> Run {
    let mut m: Machine = colony8();
    let mut beeper = Beeper::new(RATE.0, RATE.1);
    let mut mixer = SndMixer::new(RATE.0, RATE.1);
    let mut r = Run { out: vec![], live: vec![], avg: vec![], heard: vec![], synth: vec![], fm_on: vec![], beeper: vec![] };
    for i in 0..frames {
        m.run_frame();
        let start_level = m.edges_start_level();
        beeper.set_level(start_level);
        let edges = m.edges().to_vec();
        let samples = beeper.render(&edges);
        r.out.push(mixer.frame(mode_of(i), Some(&m), &edges, start_level, &samples));
        let (live, avg) = mixer.roles();
        r.live.push(live);
        r.avg.push(avg);
        r.heard.push(mixer.roles_heard());
        r.synth.push(std::array::from_fn(|k| mixer.voice_role(k)));
        r.fm_on.push(mixer.fm_on());
        r.beeper.push(samples);
    }
    r
}

fn starts(out: &[Vec<i16>]) -> Vec<usize> {
    let mut s = vec![0];
    for f in out {
        s.push(s.last().unwrap() + f.len());
    }
    s
}

#[test]
fn fm_turn_on_restarts_roles() {
    // FM: engines, the beeper from 12 s, FM: engines again from 16 s, mid-tune: from the turn-on
    // the roles are the live rule's afresh, exactly as in a mixer that never played FM before;
    // and the turn-on is the mixer's usual crossfade, no step above the signals' own
    let (off, on, end) = (12 * FPS, 16 * FPS, 22 * FPS);
    let again = mix_colony8(end, |i| if (off..on).contains(&i) { SND_MODE_BEEPER } else { SND_MODE_FM_B });
    // FM_B for the first frames only (nothing rendered before the delay fills), so the same
    // engine and hysteresis state throughout
    let fresh = mix_colony8(end, |i| if (3..on).contains(&i) { SND_MODE_BEEPER } else { SND_MODE_FM_B });
    assert!(again.fm_on[on - 1] && again.fm_on[off - 1], "Colony 8's music plays in FM around the switches");
    assert_ne!(again.avg[off - 1], fresh.avg[off - 1], "the run before the beeper built a history");
    for i in on..end {
        assert_eq!((again.live[i], again.avg[i]), (fresh.live[i], fresh.avg[i]), "frame {i}: the live rule afresh");
        assert_eq!(again.out[i], fresh.out[i], "frame {i}: the output as from a mixer new to FM");
    }

    let out: Vec<i16> = again.out.concat();
    let b: Vec<i16> = again.beeper.concat();
    let st = starts(&again.out);
    let delay = st[3];
    let w = (0.005 * music::sample_rate()) as usize;
    let fade = (0.0045 * music::sample_rate()) as usize;
    let s1 = st[on];
    // the live beeper fades out at the switch; the delayed path (FM, the music) fades in 60 ms
    // later: the steps there no larger than the beeper's own, or the FM's once it plays
    let own_live = max_step(&b[s1 - w..s1 + w]);
    let got_live = max_step(&out[s1 - w..s1 + w]);
    let own_fm = max_step(&out[s1 + delay + fade..s1 + delay + fade + 4 * w]);
    let got_in = max_step(&out[s1 + delay - w..s1 + delay + w]);
    println!("turn-on: live -> silence step {got_live} (beeper's own {own_live}); silence -> FM step {got_in} (FM's own {own_fm})");
    assert!(own_fm > 500, "the FM plays after the turn-on");
    assert!(got_live <= own_live.max(own_fm), "live -> silence: step {got_live}");
    assert!(got_in <= own_fm, "silence -> FM: step {got_in} above the FM's own {own_fm}");
}

#[test]
fn fm_method_switch_restarts_roles() {
    // FM: engines then FM: sound at 12 s, and the other way round: method A's slots are not
    // method B's channels, so the live rule starts afresh at the switch (every slot heard at
    // most in the switch frame), where a run that stays in one mode keeps its history
    let s = 12 * FPS;
    for (before, after) in [(SND_MODE_FM_B, SND_MODE_FM_A), (SND_MODE_FM_A, SND_MODE_FM_B)] {
        let run = mix_colony8(s + 1, |i| if i < s { before } else { after });
        assert!(run.fm_on[s - 1] && run.fm_on[s], "Colony 8's music plays in FM around the switch");
        assert!(run.heard[s - 1].iter().any(|&n| n > 100), "a history before the switch: {:?}", run.heard[s - 1]);
        assert!(run.heard[s].iter().all(|&n| n <= 1), "mode {before} -> {after}: the live rule afresh: {:?}", run.heard[s]);
        let stay = mix_colony8(s + 1, |_| before);
        assert!(stay.heard[s].iter().any(|&n| n > 100), "mode {before} throughout keeps its history: {:?}", stay.heard[s]);
    }
}

/// Colony 8's channels low to high, as the listening test ranked them by their mean pitch: 0, 2,
/// 1, so bass, lead, pad by channel.
const COLONY8_ROLES: [u8; SND_TONAL] = [B, L, P];
/// Colony 8's roles while channel 2, a second bass line, starts 2 semitones below channel 0: the
/// bass by its own pitch, channel 0 the pad, until it lies the margin above channel 0.
const COLONY8_ENTRY_ROLES: [u8; SND_TONAL] = [P, L, B];

#[test]
fn corpus_roles_stable() {
    // Colony 8 in FM: engines, 40 s: the synth's roles change only at a note start (its latch, on
    // a real run), and from 2 s into the music every sounding channel has its role and keeps it:
    // channels 0 and 1 play from the start, bass and lead; channel 2 enters at about 11.5 s, 2
    // semitones below channel 0, and is the bass from its first frame (ranked by its own pitch),
    // channel 0 the pad; the two swap once, when channel 2 lies the margin above channel 0 (about
    // 35 s), to their lasting roles
    let frames = 40 * FPS;
    let run = mix_colony8(frames, |_| SND_MODE_FM_B);
    // the decisions the mixer renders (method B's, the same engine state)
    let mut m = colony8();
    let mut b = SndEngines::new();
    let dec: Vec<SndFrame> = (0..frames)
        .map(|_| {
            m.run_frame();
            b.frame(&m, m.edges(), m.edges_start_level())
        })
        .collect();
    let first = run.fm_on.iter().position(|&x| x).expect("Colony 8 plays in FM");
    println!("FM from frame {first}");

    // a role change in the synth happens on a frame that starts a note on that voice: one off
    // before, a new period (more than 1/32 away) or an event on the voice
    let mut changes = 0;
    let mut last = [0u32; SND_TONAL];
    for t in 1..frames {
        let fr = &dec[t];
        let rendered = run.fm_on[t] && fr.confident != 0;
        for k in 0..SND_TONAL {
            let p = fr.tone[k].period_q8;
            let starts_note = rendered
                && ((p != 0 && (last[k] == 0 || p.abs_diff(last[k]) > last[k] / 32)) || fr.events().iter().any(|e| e.voice as usize == k));
            if run.synth[t][k] != run.synth[t - 1][k] {
                changes += 1;
                assert!(starts_note, "frame {t}: voice {k}'s role changed from {} to {} with no note start", run.synth[t - 1][k], run.synth[t][k]);
            }
            if rendered {
                last[k] = fr.events().iter().filter(|e| e.voice as usize == k).last().map_or(p, |e| e.period_q8);
            }
        }
    }
    println!("synth role changes: {changes}");

    // channel 2's entry, and the one swap after it
    let entry = (first..frames)
        .find(|&t| run.fm_on[t] && dec[t].confident != 0 && (dec[t].tone[2].period_q8 != 0 || dec[t].events().iter().any(|e| e.voice == 2)))
        .expect("channel 2 enters");
    let swap = (entry..frames).find(|&t| run.live[t][2] == P).expect("channel 2 becomes the pad");
    println!("channel 2 enters at frame {entry}, the pad from frame {swap}");
    assert!((11 * FPS..12 * FPS).contains(&entry), "channel 2 enters at about 11.5 s: frame {entry}");
    assert!((30 * FPS..38 * FPS).contains(&swap), "channels 0 and 2 swap once, at about 35 s: frame {swap}");

    // settled 2 s after the music starts: every sounding channel's live role is its phase's, and
    // the synth plays it from each voice's next note
    let settle = first + 2 * FPS;
    let early: Vec<String> = (first..settle).step_by(10).map(|t| sounding(run.live[t], dec[t].tone.map(|v| v.period_q8 as f64))).collect();
    println!("live roles in the first 2 s (every 200 ms): {early:?}");
    let want_at = |t: usize| if (entry..swap).contains(&t) { COLONY8_ENTRY_ROLES } else { COLONY8_ROLES };
    for t in settle..frames {
        if !run.fm_on[t] || dec[t].confident == 0 {
            continue;
        }
        let hz = dec[t].tone.map(|v| if v.period_q8 != 0 && v.level != 0 { 1.0 } else { 0.0 });
        assert_eq!(sounding(run.live[t], hz), sounding(want_at(t), hz), "frame {t}: the live roles");
    }
    // the synth: in each phase, every voice reaches the phase's role and keeps it to the phase's
    // end (channel 2 enters in the second, so it has no role to reach in the first)
    for (from, to, want, voices) in [(settle, entry, COLONY8_ROLES, 0..2), (entry, swap, COLONY8_ENTRY_ROLES, 0..3), (swap, frames, COLONY8_ROLES, 0..3)] {
        let mut synth_ok = [false; SND_TONAL];
        for t in from..to {
            for k in voices.clone() {
                if synth_ok[k] {
                    assert_eq!(run.synth[t][k], want[k], "frame {t}: voice {k} plays its role");
                } else {
                    synth_ok[k] = run.synth[t][k] == want[k];
                }
            }
        }
        assert!(voices.clone().all(|k| synth_ok[k]), "frames {from}..{to}: every voice plays its role after the settling: {synth_ok:?}");
    }
}
