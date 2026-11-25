// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SID tunes assembled here (no collection files): PSID and RSID files with small 6502 players,
//! played and turned into notes.

use chipmusic::notes::{DRUM, EVENTS_PER_FRAME, FRAME_T};
use chipmusic::{chip_notes, ChipNotes, Note, SidFile};
use std::collections::{BTreeMap, HashMap, HashSet};

const PAL: f64 = 985_248.0;
const FRAME: u64 = 19_656;

/// A small 6502 assembler: bytes, labels, and absolute, relative and immediate-byte fixups.
struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: HashMap<String, u16>,
    fixups: Vec<(usize, String, char)>,
}

impl Asm {
    fn new(org: u16) -> Asm {
        Asm { org, code: Vec::new(), labels: HashMap::new(), fixups: Vec::new() }
    }
    fn b(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }
    fn label(&mut self, l: &str) -> &mut Self {
        self.labels.insert(l.to_string(), self.org + self.code.len() as u16);
        self
    }
    /// `op` with a 16-bit operand: the label's address.
    fn abs(&mut self, op: u8, l: &str) -> &mut Self {
        self.code.push(op);
        self.fixups.push((self.code.len(), l.to_string(), 'a'));
        self.code.extend_from_slice(&[0, 0]);
        self
    }
    fn rel(&mut self, op: u8, l: &str) -> &mut Self {
        self.code.push(op);
        self.fixups.push((self.code.len(), l.to_string(), 'r'));
        self.code.push(0);
        self
    }
    /// `op` with the label's low (`'<'`) or high (`'>'`) byte as its immediate operand.
    fn imm(&mut self, op: u8, part: char, l: &str) -> &mut Self {
        self.code.push(op);
        self.fixups.push((self.code.len(), l.to_string(), part));
        self.code.push(0);
        self
    }
    fn addr(&self, l: &str) -> u16 {
        self.labels[l]
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, l, kind) in std::mem::take(&mut self.fixups) {
            let a = *self.labels.get(&l).unwrap_or_else(|| panic!("label {l}"));
            match kind {
                'a' => self.code[at..at + 2].copy_from_slice(&a.to_le_bytes()),
                '<' => self.code[at] = a as u8,
                '>' => self.code[at] = (a >> 8) as u8,
                _ => {
                    let from = self.org + at as u16 + 1;
                    let off = a.wrapping_sub(from) as i16;
                    assert!((-128..128).contains(&off), "branch to {l} too far");
                    self.code[at] = off as u8;
                }
            }
        }
        self.code
    }
}

/// The SID register writes of each play call: (register, value), in order.
type Frames = Vec<Vec<(u8, u8)>>;

/// A song's writes and the call its loop goes back to after the last (None: silence after).
struct Song {
    frames: Frames,
    loop_to: Option<usize>,
}

/// The write-list player: `init` points $FB/$FC at the song A selects (A = subtune - 1);
/// `play` performs one call's writes (`reg, value` pairs up to $FF; $FE `lo hi` jumps on).
fn player(a: &mut Asm, songs: &[Song]) {
    a.label("init").b(&[0x0A, 0xAA]).abs(0xBD, "table").b(&[0x85, 0xFB]).abs(0xBD, "table1").b(&[0x85, 0xFC, 0x60]);
    a.label("play").b(&[0xA0, 0x00]);
    a.label("loop").b(&[0xB1, 0xFB, 0xC9, 0xFF]).rel(0xF0, "done").b(&[0xC9, 0xFE]).rel(0xF0, "wrap");
    a.b(&[0xAA, 0xC8, 0xB1, 0xFB, 0x9D, 0x00, 0xD4, 0xC8]).abs(0x4C, "loop");
    a.label("done").b(&[0xC8, 0x98, 0x18, 0x65, 0xFB, 0x85, 0xFB, 0x90, 0x02, 0xE6, 0xFC, 0x60]);
    a.label("wrap").b(&[0xC8, 0xB1, 0xFB, 0xAA, 0xC8, 0xB1, 0xFB, 0x85, 0xFC, 0x86, 0xFB]).abs(0x4C, "play");
    a.label("table");
    for s in 0..songs.len() {
        a.fixups.push((a.code.len(), format!("s{s}f0"), 'a'));
        a.b(&[0, 0]);
    }
    let table1 = a.addr("table") + 1;
    a.labels.insert("table1".into(), table1);
    for (s, song) in songs.iter().enumerate() {
        for (k, f) in song.frames.iter().enumerate() {
            a.label(&format!("s{s}f{k}"));
            for &(r, v) in f {
                a.b(&[r, v]);
            }
            a.b(&[0xFF]);
        }
        let to = match song.loop_to {
            Some(k) => format!("s{s}f{k}"),
            None => {
                a.label(&format!("s{s}end")).b(&[0xFF]);
                format!("s{s}end")
            }
        };
        a.b(&[0xFE]);
        a.fixups.push((a.code.len(), to, 'a'));
        a.b(&[0, 0]);
    }
}

#[derive(Default)]
struct Header {
    rsid: bool,
    version: u16,
    load: u16,
    init: u16,
    play: u16,
    songs: u16,
    start: u16,
    speed: u32,
    flags: u16,
    second_sid: u8,
    title: &'static str,
    author: &'static str,
    released: &'static str,
}

/// A PSID/RSID file of `h` and the data loaded at `h.load` (an RSID's load address goes in
/// the data, as the format wants).
fn sid_file(h: &Header, data: &[u8]) -> Vec<u8> {
    let v = if h.version == 0 { 2 } else { h.version };
    let offset: u16 = if v == 1 { 0x76 } else { 0x7C };
    let mut f = Vec::new();
    f.extend_from_slice(if h.rsid { b"RSID" } else { b"PSID" });
    for x in [v, offset, if h.rsid { 0 } else { h.load }, h.init, h.play, h.songs.max(1), h.start.max(1)] {
        f.extend_from_slice(&x.to_be_bytes());
    }
    f.extend_from_slice(&h.speed.to_be_bytes());
    for s in [h.title, h.author, h.released] {
        let mut b = s.as_bytes().to_vec();
        b.resize(32, 0);
        f.extend_from_slice(&b);
    }
    if v >= 2 {
        f.extend_from_slice(&h.flags.to_be_bytes());
        f.extend_from_slice(&[0, 0, h.second_sid, 0]);
    }
    if h.rsid {
        f.extend_from_slice(&h.load.to_le_bytes());
    }
    f.extend_from_slice(data);
    f
}

/// A PSID of the write-list player at $1000, timed by the vertical blank.
fn psid(songs: &[Song]) -> Vec<u8> {
    let mut a = Asm::new(0x1000);
    player(&mut a, songs);
    let (init, play) = (a.addr("init"), a.addr("play"));
    sid_file(
        &Header { load: 0x1000, init, play, songs: songs.len() as u16, flags: 0x14, title: "Test", ..Header::default() },
        &a.finish(),
    )
}

/// The frequency register for `hz`.
fn freq(hz: f64) -> u16 {
    (hz * 16_777_216.0 / PAL).round() as u16
}

/// The period in T a frequency register plays.
fn period(reg: u16) -> u32 {
    (3_500_000.0 / (f64::from(reg) * PAL / 16_777_216.0)).round() as u32
}

/// Spectrum T of `cycles`, as the notes round them.
fn t_of(cycles: u64) -> u64 {
    ((cycles as f64 * 3_500_000.0 / PAL / 64.0).round() as u64) * 64
}

/// Frames: `n` empty calls.
fn frames(n: usize) -> Frames {
    vec![Vec::new(); n]
}

/// Writes setting voice `v`'s frequency.
fn pitch(f: &mut Vec<(u8, u8)>, v: u8, reg: u16) {
    f.push((7 * v, reg as u8));
    f.push((7 * v + 1, (reg >> 8) as u8));
}

/// The common start: volume 15, every voice's ADSR with sustain 15.
fn setup(f: &mut Vec<(u8, u8)>) {
    f.push((24, 0x0F));
    for v in 0..3 {
        f.push((7 * v + 5, 0x00));
        f.push((7 * v + 6, 0xF0));
    }
}

fn play(file: &[u8], song: u16, seconds: f64) -> ChipNotes {
    chip_notes(file, Some(song), seconds).unwrap_or_else(|e| panic!("cannot be played: {e}"))
}

fn voice(n: &ChipNotes, v: u8) -> Vec<Note> {
    n.notes.notes.iter().filter(|x| x.voice == v).copied().collect()
}

/// The events of each frame as the pack counts them (an independent count).
fn events_per_frame(notes: &[Note], intro_t: u64, loop_t: u64) -> BTreeMap<(usize, u64), usize> {
    let mut per = BTreeMap::new();
    for (sec, (from, len)) in [(0, intro_t), (intro_t, loop_t)].into_iter().enumerate() {
        if len == 0 {
            continue;
        }
        let (to, lim) = (from + len, len.div_ceil(64));
        let mut ps = Vec::new();
        for n in notes {
            if n.end_t <= from || n.start_t >= to || (sec == 1 && n.voice == DRUM && n.start_t < from) {
                continue;
            }
            let s = (n.start_t.max(from) - from + 32) / 64;
            let e = (n.end_t.min(to) - from + 32) / 64;
            let cut = n.end_t >= to || e >= lim;
            if s >= lim || (!cut && e <= s) {
                continue;
            }
            ps.push((s, e.min(lim), n.voice, cut));
        }
        let starts: HashSet<(u8, u64)> = ps.iter().map(|p| (p.2, p.0)).collect();
        for &(s, e, v, cut) in &ps {
            *per.entry((sec, s * 64 / FRAME_T)).or_default() += 1;
            if !cut && (v == DRUM || !starts.contains(&(v, e))) {
                *per.entry((sec, e * 64 / FRAME_T)).or_default() += 1;
            }
        }
    }
    per
}

#[test]
fn three_voices_gates_legato_and_retriggers() {
    // voice 1 (SID voice 1): a high note gated for 10 calls, off for 5, then again for 10
    // voice 2: a low note, legato to another after 8 calls, off at 20
    // voice 3: a middle note retriggered (gate off and on within one call) at call 6
    let (hi, hi2, lo, lo2, mid) = (freq(880.0), freq(987.77), freq(110.0), freq(146.83), freq(440.0));
    let mut f = frames(30);
    setup(&mut f[0]);
    pitch(&mut f[0], 0, hi);
    f[0].push((4, 0x41));
    pitch(&mut f[0], 1, lo);
    f[0].push((11, 0x21));
    pitch(&mut f[0], 2, mid);
    f[0].push((18, 0x11));
    f[6].extend([(18, 0x10), (18, 0x11)]);
    pitch(&mut f[8], 1, lo2);
    f[10].push((4, 0x40));
    pitch(&mut f[15], 0, hi2);
    f[15].push((4, 0x41));
    f[20].push((11, 0x20));
    f[25].extend([(4, 0x40), (18, 0x10)]);
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 2.0);
    // lead: SID voice 1 (highest), bass: SID voice 2 (lowest), pad: SID voice 3
    assert_eq!(n.notes.sid_voice, [Some(0), Some(1), Some(2)]);
    let t = |k: u64| t_of(k * FRAME);
    let lead = voice(&n, 0);
    assert_eq!(
        lead.iter().map(|x| (x.start_t, x.end_t, x.period_t)).collect::<Vec<_>>(),
        vec![(0, t(10), period(hi)), (t(15), t(25), period(hi2))]
    );
    let bass = voice(&n, 1);
    assert_eq!(
        bass.iter().map(|x| (x.start_t, x.end_t, x.period_t)).collect::<Vec<_>>(),
        vec![(0, t(8), period(lo)), (t(8), t(20), period(lo2))]
    );
    let pad = voice(&n, 2);
    assert_eq!(pad.iter().map(|x| (x.start_t, x.end_t)).collect::<Vec<_>>(), vec![(0, t(6)), (t(6), t(25))]);
    assert!(n.notes.notes.iter().all(|x| x.level == 255 && x.voice < 3));
    // no loop: the notes end at the last note's end
    assert_eq!((n.notes.intro_t, n.notes.loop_t), (t(25), 0));
}

#[test]
fn levels_follow_volume_and_sustain() {
    let mut f = frames(10);
    f[0].extend([(24, 0x05), (6, 0x40)]);
    pitch(&mut f[0], 0, freq(440.0));
    f[0].push((4, 0x41));
    f[5].push((4, 0x40));
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 1.0);
    // 255 * sqrt(5/15 * (0.35 + 0.65 * 4/15))
    let want = (255.0_f64 * (5.0_f64 / 15.0 * (0.35 + 0.65 * 4.0 / 15.0)).sqrt()).round() as u8;
    assert_eq!(voice(&n, 0)[0].level, want);
}

#[test]
fn arpeggios_fold_into_their_root() {
    // a C major chord cycling every call for 30 calls, then a held note
    let chord = [freq(261.63), freq(329.63), freq(392.0)];
    let mut f = frames(50);
    setup(&mut f[0]);
    f[0].push((4, 0x41));
    for (k, frame) in f.iter_mut().enumerate().take(30) {
        pitch(frame, 0, chord[k % 3]);
    }
    pitch(&mut f[30], 0, freq(523.25));
    f[45].push((4, 0x40));
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 1.5);
    let t = |k: u64| t_of(k * FRAME);
    let lead = voice(&n, 0);
    assert_eq!(
        lead.iter().map(|x| (x.start_t, x.end_t, x.period_t)).collect::<Vec<_>>(),
        vec![(0, t(30), period(chord[0])), (t(30), t(45), period(freq(523.25)))]
    );
    assert_eq!((n.notes.arp_runs, n.notes.raw_notes), (1, 31));
}

#[test]
fn a_fast_run_of_new_pitches_is_not_an_arpeggio() {
    // eight different pitches, two calls each: no pitch recurs within four, nothing folds
    let mut f = frames(20);
    setup(&mut f[0]);
    f[0].push((4, 0x41));
    for k in 0..8 {
        pitch(&mut f[2 * k], 0, freq(220.0 * 2f64.powf(k as f64 / 12.0)));
    }
    f[16].push((4, 0x40));
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 1.0);
    assert_eq!((voice(&n, 0).len(), n.notes.arp_runs), (8, 0));
}

#[test]
fn noise_makes_drum_hits() {
    // voice 3: a noise hit every 10 calls (noise for 1 call, then a falling pulse tail for 3,
    // gate off); voice 1 plays a held note
    let mut f = frames(60);
    setup(&mut f[0]);
    pitch(&mut f[0], 0, freq(440.0));
    f[0].push((4, 0x11));
    for h in 0..5 {
        let k = 10 * h + 2;
        pitch(&mut f[k], 2, 0x3000);
        f[k].push((18, 0x81));
        for (j, hz) in [300.0, 200.0, 120.0].iter().enumerate() {
            pitch(&mut f[k + 1 + j], 2, freq(*hz));
            f[k + 1 + j].push((18, 0x41));
        }
        f[k + 4].push((18, 0x40));
    }
    f[55].push((4, 0x10));
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 1.5);
    let t = |k: u64| t_of(k * FRAME);
    let drums = voice(&n, DRUM);
    assert_eq!(drums.iter().map(|x| x.start_t).collect::<Vec<_>>(), (0..5).map(|h| t(10 * h + 2)).collect::<Vec<_>>());
    assert!(drums.iter().enumerate().all(|(h, x)| x.period_t == 0 && x.end_t == t(10 * h as u64 + 3)), "{drums:?}");
    // the tails are the drum's: only the held note is tonal
    assert_eq!(n.notes.notes.iter().filter(|x| x.voice < DRUM).count(), 1);
}

#[test]
fn drum_hits_close_together_keep_a_gap() {
    // noise held for 4 calls but retriggered after 2: the first hit ends before the second
    let mut f = frames(12);
    setup(&mut f[0]);
    pitch(&mut f[0], 0, 0x2000);
    f[0].push((4, 0x81));
    f[2].extend([(4, 0x80), (4, 0x81)]);
    f[6].push((4, 0x80));
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 0.5);
    let d = voice(&n, DRUM);
    assert_eq!(d.len(), 2);
    assert!(d[0].end_t + 2048 <= d[1].start_t, "{d:?}");
}

#[test]
fn a_cia_timed_song_plays_at_its_timer_rate() {
    // speed bit set; init sets CIA 1 timer A to 9851 (a call every 9852 cycles, 100 Hz)
    let mut f = frames(40);
    setup(&mut f[0]);
    f[0].push((4, 0x41));
    for (k, frame) in f.iter_mut().enumerate().take(30) {
        pitch(frame, 0, freq(if k / 5 % 2 == 0 { 440.0 } else { 660.0 }));
    }
    f[30].push((4, 0x40));
    let mut a = Asm::new(0x1000);
    a.label("start").b(&[0x48, 0xA9, 0x7B, 0x8D, 0x04, 0xDC, 0xA9, 0x26, 0x8D, 0x05, 0xDC, 0x68]).abs(0x4C, "init");
    player(&mut a, &[Song { frames: f, loop_to: None }]);
    let (init, play_at) = (a.addr("start"), a.addr("play"));
    let file =
        sid_file(&Header { load: 0x1000, init, play: play_at, songs: 1, speed: 1, flags: 0x04, ..Header::default() }, &a.finish());
    let n = play(&file, 1, 1.0);
    let t = |k: u64| t_of(k * 9852);
    let lead = voice(&n, 0);
    assert_eq!(lead.len(), 6);
    assert_eq!(lead.iter().map(|x| x.start_t).collect::<Vec<_>>(), (0..6).map(|k| t(5 * k)).collect::<Vec<_>>());
    assert_eq!(n.notes.intro_t, t(30));
}

/// A PSID with play address 0: init installs an IRQ handler at $0314 (the player, then
/// `JMP $EA31`) and starts CIA 1's timer A at `latch`.
fn irq_tune(f: Frames, latch: u16) -> Vec<u8> {
    let mut a = Asm::new(0x1000);
    a.label("start").abs(0x20, "init");
    a.imm(0xA9, '<', "handler").b(&[0x8D, 0x14, 0x03]).imm(0xA9, '>', "handler").b(&[0x8D, 0x15, 0x03]);
    a.b(&[0xA9, latch as u8, 0x8D, 0x04, 0xDC, 0xA9, (latch >> 8) as u8, 0x8D, 0x05, 0xDC]);
    a.b(&[0xA9, 0x11, 0x8D, 0x0E, 0xDC, 0xA9, 0x81, 0x8D, 0x0D, 0xDC, 0x60]);
    a.label("handler").abs(0x20, "play").b(&[0x4C, 0x31, 0xEA]);
    player(&mut a, &[Song { frames: f, loop_to: None }]);
    let init = a.addr("start");
    sid_file(&Header { load: 0x1000, init, play: 0, songs: 1, speed: 1, flags: 0x04, ..Header::default() }, &a.finish())
}

fn steps(n: usize, every: usize) -> Frames {
    let mut f = frames(n);
    setup(&mut f[0]);
    f[0].push((4, 0x41));
    for (k, frame) in f.iter_mut().enumerate().take(n - 4) {
        pitch(frame, 0, freq(if (k / every).is_multiple_of(2) { 440.0 } else { 660.0 }));
    }
    f[n - 4].push((4, 0x40));
    f
}

#[test]
fn a_tune_with_its_own_cia_interrupt() {
    // the handler runs every 14778 cycles (about 66.7 Hz)
    let n = play(&irq_tune(steps(34, 6), 14777), 1, 1.0);
    let lead = voice(&n, 0);
    assert_eq!(lead.len(), 5);
    for (k, x) in lead.iter().enumerate() {
        assert!(x.start_t.abs_diff(t_of(6 * k as u64 * 14778)) <= 128, "{k}: {x:?}");
    }
}

#[test]
fn an_rsid_with_a_raster_interrupt() {
    // init banks the KERNAL out ($01 = $35), points $FFFE at its handler, turns CIA 1's
    // interrupt off and VIC-II's raster interrupt on, then spins; the handler acknowledges,
    // plays and returns with RTI
    let mut a = Asm::new(0x1000);
    a.label("start").b(&[0x78, 0xA9, 0x35, 0x85, 0x01]);
    a.imm(0xA9, '<', "handler").b(&[0x8D, 0xFE, 0xFF]).imm(0xA9, '>', "handler").b(&[0x8D, 0xFF, 0xFF]);
    a.b(&[0xA9, 0x7F, 0x8D, 0x0D, 0xDC, 0xAD, 0x0D, 0xDC, 0xA9, 0x01, 0x8D, 0x1A, 0xD0, 0xA9, 0x80, 0x8D, 0x12, 0xD0]);
    a.b(&[0xA9, 0x1B, 0x8D, 0x11, 0xD0, 0xA9, 0x00]).abs(0x20, "init").b(&[0x58]);
    a.label("spin").abs(0x4C, "spin");
    a.label("handler").b(&[0x48, 0x8A, 0x48, 0x98, 0x48, 0xA9, 0x01, 0x8D, 0x19, 0xD0]).abs(0x20, "play");
    a.b(&[0x68, 0xA8, 0x68, 0xAA, 0x68, 0x40]);
    player(&mut a, &[Song { frames: steps(34, 6), loop_to: None }]);
    let init = a.addr("start");
    let file = sid_file(&Header { rsid: true, load: 0x1000, init, songs: 1, flags: 0x04, ..Header::default() }, &a.finish());
    let n = play(&file, 1, 1.0);
    let lead = voice(&n, 0);
    assert_eq!(lead.len(), 5);
    for (k, x) in lead.iter().enumerate() {
        assert!(x.start_t.abs_diff(t_of(6 * k as u64 * FRAME)) <= 128, "{k}: {x:?}");
    }
}

#[test]
fn a_looping_song_gives_its_intro_and_one_pass() {
    // an intro of 100 calls (2 s), then a loop of 75 calls (1.5 s) back to call 100
    let mut f = frames(175);
    setup(&mut f[0]);
    for k in (0..175).step_by(25) {
        pitch(&mut f[k], 0, freq(220.0 + k as f64));
        f[k].push((4, 0x41));
        f[k + 20].push((4, 0x40));
    }
    let n = play(&psid(&[Song { frames: f, loop_to: Some(100) }]), 1, 10.0);
    let t = |k: u64| t_of(k * FRAME);
    assert_eq!((n.notes.intro_t, n.notes.loop_t), (t(100), t(175) - t(100)));
    let lead = voice(&n, 0);
    assert_eq!(lead.len(), 7);
    assert_eq!(lead.last().map(|x| (x.start_t, x.end_t)), Some((t(150), t(170))));
    assert!(n.notes.notes.iter().all(|x| x.end_t <= t(175)));
}

#[test]
fn a_song_that_never_repeats_has_no_loop() {
    let mut f = frames(100);
    setup(&mut f[0]);
    for k in (0..100).step_by(10) {
        pitch(&mut f[k], 0, freq(200.0 + 10.0 * k as f64));
        f[k].push((4, 0x41));
        f[k + 5].push((4, 0x40));
    }
    let n = play(&psid(&[Song { frames: f, loop_to: None }]), 1, 4.0);
    assert_eq!((n.notes.loop_t, n.notes.intro_t), (0, t_of(95 * FRAME)));
}

#[test]
fn a_frame_holds_at_most_eight_events() {
    // CIA-timed at about 200 Hz: two voices change pitch every call (8 pitches, none recurring
    // within four) and voice 3 strikes noise every other call: over 8 events in a Spectrum frame
    let mut f = frames(100);
    setup(&mut f[0]);
    f[0].extend([(4, 0x41), (11, 0x41)]);
    for (k, frame) in f.iter_mut().enumerate().take(96) {
        pitch(frame, 0, freq(440.0 * 2f64.powf((k % 8) as f64 / 12.0)));
        pitch(frame, 1, freq(110.0 * 2f64.powf((k % 8) as f64 / 12.0)));
        if k % 2 == 0 {
            pitch(frame, 2, 0x4000);
            frame.extend([(18, 0x80), (18, 0x81)]);
        } else {
            frame.push((18, 0x80));
        }
    }
    let mut a = Asm::new(0x1000);
    a.label("start").b(&[0x48, 0xA9, 0x00, 0x8D, 0x04, 0xDC, 0xA9, 0x13, 0x8D, 0x05, 0xDC, 0x68]).abs(0x4C, "init");
    player(&mut a, &[Song { frames: f, loop_to: None }]);
    let (init, play_at) = (a.addr("start"), a.addr("play"));
    let file =
        sid_file(&Header { load: 0x1000, init, play: play_at, songs: 1, speed: 1, flags: 0x04, ..Header::default() }, &a.finish());
    let n = play(&file, 1, 1.0);
    let per = events_per_frame(&n.notes.notes, n.notes.intro_t, n.notes.loop_t);
    assert!(per.values().all(|&e| e <= EVENTS_PER_FRAME), "{per:?}");
    let (drums, tonal) = n.notes.dropped;
    assert!(drums > 0 && tonal > 0, "{:?}", n.notes.dropped);
    // the lead keeps every note
    assert_eq!(voice(&n, 0).len(), 96);
}

#[test]
fn subtunes_are_chosen_by_a() {
    let song = |hz: f64| {
        let mut f = frames(10);
        setup(&mut f[0]);
        pitch(&mut f[0], 0, freq(hz));
        f[0].push((4, 0x41));
        f[5].push((4, 0x40));
        Song { frames: f, loop_to: None }
    };
    let file = psid(&[song(440.0), song(220.0), song(330.0)]);
    for (s, hz) in [(1, 440.0), (2, 220.0), (3, 330.0)] {
        assert_eq!(voice(&play(&file, s, 0.5), 0)[0].period_t, period(freq(hz)));
    }
    // the start song by default
    let n = chip_notes(&file, None, 0.5).unwrap();
    assert_eq!(n.subtune, 1);
    assert!(chip_notes(&file, Some(4), 0.5).unwrap_err().contains("no subtune 4"));
}

#[test]
fn headers() {
    let h = Header {
        version: 2,
        load: 0x1000,
        init: 0x1000,
        play: 0x1003,
        songs: 3,
        start: 2,
        speed: 0b100,
        flags: 0x08 | 0x20,
        title: "A \"quoted\" tune",
        author: "Someone",
        released: "1985 Nobody",
        ..Header::default()
    };
    let s = SidFile::parse(&sid_file(&h, &[0x60; 8])).unwrap();
    assert_eq!((s.rsid, s.version, s.load, s.init, s.play, s.songs, s.start_song), (false, 2, 0x1000, 0x1000, 0x1003, 3, 2));
    assert_eq!((s.title.as_str(), s.author.as_str(), s.released.as_str()), ("A \"quoted\" tune", "Someone", "1985 Nobody"));
    assert!(s.ntsc());
    assert_eq!(s.clock_hz(), 1_022_727);
    assert_eq!(s.sid_model(), "8580");
    assert_eq!((s.cia_timed(1), s.cia_timed(3)), (false, true));
    let info = s.info();
    assert!(info.starts_with("PSID v2, 3 songs, start song 2\n"), "{info}");
    assert!(info.contains("clock: NTSC (1022727 Hz)") && info.contains("speed: CIA timer for songs 3"), "{info}");
    // version 1, load address in the data, init 0 = the load address
    let mut d = vec![0x00, 0xC0];
    d.extend([0x60; 4]);
    let s = SidFile::parse(&sid_file(&Header { version: 1, play: 0xC001, ..Header::default() }, &d)).unwrap();
    assert_eq!((s.version, s.load, s.init, s.data.len(), s.clock_name()), (1, 0xC000, 0xC000, 4, "unknown (played as PAL)"));
    // an RSID
    let s = SidFile::parse(&sid_file(&Header { rsid: true, load: 0x0801, ..Header::default() }, &[0x60])).unwrap();
    assert_eq!((s.rsid, s.load, s.format()), (true, 0x0801, "RSID"));
}

#[test]
fn notes_toml() {
    let mut f = frames(10);
    setup(&mut f[0]);
    pitch(&mut f[0], 0, freq(440.0));
    f[0].push((4, 0x41));
    f[5].push((4, 0x40));
    let mut a = Asm::new(0x1000);
    player(&mut a, &[Song { frames: f, loop_to: None }]);
    let (init, play_at) = (a.addr("init"), a.addr("play"));
    let h = Header {
        load: 0x1000,
        init,
        play: play_at,
        songs: 1,
        flags: 0x04,
        title: "Back\\slash \"q\"",
        author: "Nobody",
        released: "1984",
        ..Header::default()
    };
    let n = play(&sid_file(&h, &a.finish()), 1, 0.5);
    let text = n.toml("dir/tune.sid");
    let v: toml::Table = text.parse().unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(v["file"].as_str(), Some("dir/tune.sid"));
    assert_eq!(v["title"].as_str(), Some("Back\\slash \"q\""));
    assert_eq!((v["subtune"].as_integer(), v["clock_hz"].as_integer()), (Some(1), Some(985_248)));
    assert_eq!(v["seconds"].as_float().or(v["seconds"].as_integer().map(|i| i as f64)), Some(0.5));
    assert_eq!((v["intro_t"].as_integer(), v["loop_t"].as_integer()), (Some(t_of(5 * FRAME) as i64), Some(0)));
    let notes = v["note"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    let x = notes[0].as_table().unwrap();
    let keys: Vec<&str> = x.keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["end_t", "level", "period_t", "start_t", "voice"]);
    assert_eq!(x["period_t"].as_integer(), Some(i64::from(period(freq(440.0)))));
}

#[test]
fn refusals() {
    let ok = Header { load: 0x1000, init: 0x1000, play: 0x1001, songs: 1, ..Header::default() };
    let err = |file: Vec<u8>| chip_notes(&file, None, 1.0).unwrap_err();
    assert!(err(b"MThd and more bytes than a header needs".repeat(4)).contains("no PSID or RSID magic"));
    assert!(err(vec![0x50; 20]).contains("shorter than a header"));
    assert!(err(sid_file(&Header { flags: 1, ..ok }, &[0x60, 0x60])).contains("Sidplayer"));
    assert!(err(sid_file(&Header { rsid: true, flags: 2, ..ok }, &[0x60])).contains("BASIC"));
    assert!(err(sid_file(&Header { version: 3, second_sid: 0x42, ..ok }, &[0x60, 0x60])).contains("2 SIDs"));
    assert!(err(sid_file(&Header { load: 0xFFF0, ..ok }, &[0x60; 32])).contains("runs past $FFFF"));
    // init calls the KERNAL's CHROUT
    let e = err(sid_file(&ok, &[0x20, 0xD2, 0xFF, 0x60]));
    assert!(e.contains("calls the KERNAL at $FFD2"), "{e}");
    // a JAM
    assert!(err(sid_file(&ok, &[0x02])).contains("JAM opcode at $1000"));
    // play spins forever
    let e = err(sid_file(&Header { play: 0x1001, ..ok }, &[0x60, 0x4C, 0x01, 0x10]));
    assert!(e.contains("play routine at $1001 does not return"), "{e}");
    // init spins forever
    let e = err(sid_file(&ok, &[0x4C, 0x00, 0x10]));
    assert!(e.contains("init routine at $1000 does not return"), "{e}");
    // a tune that writes nothing
    assert!(err(sid_file(&ok, &[0x60, 0x60])).contains("wrote nothing to the SID"));
    // a BRK
    assert!(err(sid_file(&ok, &[0x00, 0x00])).contains("BRK"));
}

#[test]
fn play_routines_ending_as_interrupt_handlers_return() {
    // play ends with JMP $EA81 (the KERNAL's register restore and RTI) after pushing A, X, Y
    // as the KERNAL's entry would; the call returns there
    let mut f = frames(10);
    setup(&mut f[0]);
    pitch(&mut f[0], 0, freq(440.0));
    f[0].push((4, 0x41));
    f[5].push((4, 0x40));
    let mut a = Asm::new(0x1000);
    a.label("irqplay").abs(0x20, "play").b(&[0x4C, 0x81, 0xEA]);
    player(&mut a, &[Song { frames: f, loop_to: None }]);
    let (init, play_at) = (a.addr("init"), a.addr("irqplay"));
    let file = sid_file(&Header { load: 0x1000, init, play: play_at, songs: 1, ..Header::default() }, &a.finish());
    let n = play(&file, 1, 0.5);
    assert_eq!(voice(&n, 0).len(), 1);
}
