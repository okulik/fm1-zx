<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Learned sound entries: the pack learns each game's music

Status: design, 2026-11-15. The first of four projects the owner asked for on 2026-11-15:

1. this one: learned music entries carried in the pack, and the analyser that learns them;
2. richer FM instruments, chosen per channel when the pack is built;
3. sound effects translated to FM;
4. tape playback on the FM-1 itself.

Projects 2 and 3 build on this one; the pack format leaves room for them (section 6). Project 4
is independent.

## 1. Intent

Today FM: engines (method B, `core/sound/engines.c`) plays a game's music in FM only if a
hand-written entry knows its music routine: a code fingerprint plus a C reader function compiled
into the firmware (`core/sound/engines_builtin.h`). Each entry took hours of hand analysis, so
only five exist (Castaway, Colony 8, ROM BEEP, Cave Runner, Unison).

The owner wants this turned around. When `tools/make-personal-pack` builds a pack, a host tool
works out each game's music routines by itself and writes what it learned into the pack. On the
FM-1, FM: engines plays those routines as FM, exactly as it plays the built-in ones. Adding FM
music for a new game becomes rebuilding the pack: minutes, not hours of hand analysis ("turn hours
into minutes").

The heavy work (tracing, code analysis, formula fitting) happens on the host, where it is cheap.
The device only recognises a routine, reads a few bytes, applies a formula and checks the result
against the speaker, as it does today.

## 2. Goals, non-goals, anti-goals

Goals:

- **Unattended (owner's decision).** `make-personal-pack` learns entries with no input from the
  owner: no key scripts required, no review step.
- **Unsure means beeper (owner's decision).** A routine the tool cannot pin down gets no entry;
  the game plays it on the beeper as today.
- **Hear, and read the code (owner's choice, option C).** The tool learns from what it hears in an
  automatic run (title, attract mode, a generic start-up, random play) *and* from reading the
  code of every routine that drives the speaker, so music it never heard can still get an entry.
- **Music first.** Tonal channels and simple noise drums.
- **Fast.** Analysing a game takes seconds on the host; the owner's folder (29 games) takes
  minutes.
- **Reported.** Per game: routines found, learned (heard or code-only), and left out with the
  reason.

Non-goals (this project):

- Sound effects (project 3) and instrument choice (project 2). The format reserves room for both.
- Converting the built-in C entries to data. They stay as they are: tested, and they cover
  Castaway's slides, Colony 8's drums and ROM BEEP's per-call notes.
- A new menu choice. FM: engines plays built-in and learned entries alike.
- Pitch slides, unless the code reading handles a slide cleanly (an exactly modelled step);
  otherwise the routine is left on the beeper.
- 128K games (already refused by the personal pack).
- The repo's own pack (`games/games.toml`): unchanged. Its music is covered by built-in entries.

Anti-goals (a failure even if it technically works):

- **Wrong notes.** A learned entry that plays a wrong pitch, an octave off, or a sound effect as
  music. Every gate below leans towards leaving sound on the beeper.
- **Regressions for games without learned entries.** With no learned entries, FM: engines must
  play sample for sample as today; Beeper and FM: sound are untouched.
- **A bad section breaking games.** A malformed or unknown sound section means no learned
  entries; the games still load and run.
- **Third-party material in the repo.** Tests in the repo use the repo's games and generated test
  programs only. The owner's games are measured locally (as music-corpus does today): digests,
  offsets and formulas may appear in notes, game bytes never.
- **Slow pack builds.** Analysis that turns a pack build into tens of minutes.

## 3. Constraints

- **RAMTEXT:** 48,456 of 49,152 bytes used (696 free). The interpreter and the generic reader run
  from flash, as `app/pixmodes.c` does, under the `app.ld` reach rules (`check_reach.py`).
- **Frame budget:** worst frame 2.50M of a 3.6M instruction cap (Border, scroll). Reading a
  learned entry should cost no more than a hand-written reader. fw_budget counts instructions
  only, so the device speed of flash-run code stays a hardware-checklist item.
- **Flash:** the app slot is 581,564 B. The image is about 98 KB and the pack takes the rest.
  Learned entries are tens of bytes per routine, negligible beside the games.
- **Firmware and pack ship together** in one `.fwsc` package (`tools/make-packages`), so the pack
  format version can change with the firmware.
- **Core:** the Z80 core is shared by the firmware and the host. Any analysis hook is host-only
  and compiled out of the firmware (as `core/zx/tapesig.c` is filtered out today). The
  firmware's core binary must not change because of it.
- **No hardware yet.** Everything is verified on the host and in emu-gate. The device rule
  stands: never flash, SysEx or open MIDI without the owner's explicit yes per step.
- **The method C spike** (2026-11-14) is the prior art. Its code is in `build/spike-cpu-c/`
  (untracked, about 1,300 lines of Rust); its core hook lived in a now-stale worktree (+62 lines
  in `core/zx/timing.c`, `zx.h`, `machine.c`, `zx-sys`). Findings, condensed:
  - It learned exact period formulas unaided for Cave Runner (title, in game), Colony 8 and
    Unison, with 0 wrong notes, by reading the values each reload instruction reads.
  - Formula forms that sufficed: `a·x + b`, `a/x + b`, `a·x·y + c·x + b`, `a·x + c·x/y + b`.
  - Failures: Castaway's slides and waveform-wrap multiplier; an octave ambiguity (one channel
    toggled from two OUT sites vs two XOR channels in unison: Star Pilot vs Unison) that the CPU
    alone cannot settle; no drums; one effect frame let through by its music gate.
  - On the device it would have cost 0.15-0.4M cycles a frame. On the host, at pack time, that
    cost does not matter.
- **No Z80 disassembler or code analyser exists in the repo.** One is new work.

## 4. Approaches

How the tool gets to hear in-game sound when nobody plays:

| | Approach | Trade-off |
|---|---|---|
| A | Only what plays by itself (title, attract mode) | Simplest. In-game music and effects mostly stay on the beeper. |
| B | A, plus a generic start-up (common start keys, random play) | More coverage, still unattended. Some games never get past their menus. |
| C | B, plus reading the code of every speaker routine, so routines never heard can still get entries; the runtime check guards them | Most coverage. Needs a Z80 code analyser, the biggest new piece. |

**The owner chose C.**

How learned entries reach the device (decided in the conversation, the builder's details in
section 7): entries are **data in the pack**, applied by one generic reader in the firmware.
The alternative, generating C entries and rebuilding the firmware per pack, was rejected. A
personal pack must not need a firmware build, and RAMTEXT has no room for per-game code.

## 5. Design

### 5.1 Overview

```
make-personal-pack
  └─ fmzx-pack personal
       ├─ load each game (tapeload::prepare, unchanged)
       ├─ learn (new): listen → read code → fit and gate → entries + report lines
       └─ write games.bin, now with a sound section

FM-1, a game starts
  └─ method B's table = built-in entries + this game's learned entries
       └─ a learned entry's reader = the generic reader running its program
            └─ the existing check against the speaker edges decides every frame
```

### 5.2 Host: the learner

A new host component (crate or module: the builder's call) called by `fmzx-pack personal` for
every game it packs. It uses `zx-sys` (the same C core as the firmware) with a host-only trace
hook.

**Trace hook (core, host-only).** Events with the T-state of the frame:

- instruction start: pc and registers;
- memory read: address, value, the reading instruction's pc;
- OUT to an even port (the ULA): value, pc;
- interrupt accepted.

Compiled out of the firmware. A host test asserts the firmware's core objects are unchanged
(for example by RAMTEXT size and image hash before and after).

**Step 1: listen.** From the loaded game (time 0 as music-corpus defines it):

- 40 s untouched: title and attract-mode music;
- then a generic start-up: each of ENTER, SPACE, 0, 1 and Kempston fire pressed in turn, each
  followed by a few seconds of watching;
- then random play for a fixed time: QAOP, cursor keys, Kempston directions and fire, SPACE,
  changing every 100-500 ms from a fixed seed (the same pack gives the same result);
- if a `<name>.keys` file sits beside the game (music-corpus's key script format), it runs
  instead of the generic start-up and random play.

The total emulated time per game is a constant chosen for the speed goal (proposed 3 minutes).
Every OUT to the speaker is recorded with its pc and the trace window around it.

**Step 2: read the code.** Find the candidate routines:

- every OUT site heard in step 1;
- plus a static scan of the 48K memory image at the snapshot (and at the end of the run, after
  any code was unpacked or moved): `OUT (n),A` with even `n`, and `OUT (C),r` where the code
  before it sets C to an even value.

For each candidate, decode the surrounding code into basic blocks and find the loop(s) holding
the OUT. Work out:

- each path's T-states;
- which registers, memory bytes or operand bytes reload the loop's counters;
- how the speaker bit is formed: a level written per channel (time-slot engines) or XOR flips
  (Cave Runner, Unison style). Per-call notes (ROM BEEP style, one call per note) are not a family
  the learner handles: the built-in ROM BEEP entry covers the main one;
- **which OUT sites belong to which channel, from the counters:** two OUT sites fed by one shared
  reload counter are one channel toggled from two places; separate counters are separate channels
  (Unison's XOR channels in unison). This settles the spike's octave doubt (Star Pilot vs Unison),
  which the CPU's behaviour and the speaker edges alone cannot settle. When the structure does not
  settle it either, the routine is left out;
- **the note's timing:** the counter that measures the note's duration, if the loop has one, so the
  entry can say how long the note has held at the site's last edge (B's `since` and `row`) and
  whether the engine moved on to its next note after that edge (`stale` 1). Without it, the entry
  reports the frame's whole span as not known (`SND_B_SINCE_ALL`): notes then change a frame late,
  and the code-only gate below refuses such a routine;
- where channels begin, one pass's length and each channel's OUT position in a pass: the data B's
  check already uses (`pass`, `pos`, `sites`, `flips`, `min_len`).

From that, a period formula per channel in terms of its inputs (memory byte or word, register at
the site's last edge, operand byte at an offset from the anchor), plus the silence condition (for
example a rest mask of 0). The fingerprint is a window of code around the anchor. Its wildcard
bytes are the operands the music rewrites; its relocated words are absolute addresses inside the
window. This is the same `digest`, `wild` and `reloc` scheme as the built-in entries.

Anything the reader does not fully understand means no entry for that routine:

- an undecoded instruction;
- a computed jump in the loop;
- self-modification it does not model;
- a loop shape outside the known families.

**Step 3: fit and gate.**

- **Heard routines.** Fit the formula to the heard periods the spike's way: period samples from
  flips, minus contention, against the values the reload instructions read. The code-derived
  formula and the fitted one must agree. Requirements:
  - at least 3 distinct notes spanning at least half a semitone;
  - at least 85% inliers;
  - periods under the 5 kHz limit.
  
  Then **replay**: the recorded run goes through the real method B (`snd_b`, via `zx-sys`) with
  the candidate entry. The entry is kept only if both hold:
  - B claims most of the routine's sounding frames (proposed: 90%), so the formula is not
    partial;
  - the music gate passes: notes recur, the music is in runs of equal notes, and it is not
    monotonic step sequences (the spike's lessons, section 7 of its findings).

  The channel partition (and so the octave) comes from the code reading's counters, not from the
  replay: on identical edges both readings would pass B's check. If the code reading did not
  settle it, the routine is left out.
- **Code-only routines.** Never heard, so no fit and no replay. An entry is written only when:
  - the code reading is complete;
  - the formula is exact from instruction timings (including the contention of the routine's
    address range);
  - the note's timing is known (a duration counter was found);
  - the routine is evidently a music player: its notes come from a data table walked by a
    pointer, not computed by arithmetic. This keeps effect routines (project 3) out.
  
  On the device the runtime check guards these entries every frame.
- **Drums.** A heard routine whose output is noise, between notes of a learned tonal routine, and
  whose code is a separate routine (B's `SND_SITE_DRUM`), gets a drum site on the tonal entry.
- **Built-in entries win.** A routine a built-in entry already matches gets no learned entry.

**Step 4: write.** The game's learned entries go into the pack's sound section (5.3). One line
per game goes into the `make-personal-pack` output (for example
`Bee Line: music learned (1 routine, heard, 2 channels)` or
`Glass Keep: no music learned (loop shape not understood at 9C40h)`). The details go to
`<out>/sound-report.txt`:

- per routine: the anchor, the formula, heard or code-only, the gates' figures;
- per routine left out: the reason.

**Speed.** Proposed target: under 10 s of host time per game, under 5 minutes for the owner's
folder. Analysis of games in a folder may run in parallel. The learned entries are cached by the
game file's CRC and the learner's version, so an unchanged game is not analysed again.

### 5.3 The pack: a sound section

The pack format (`docs/pack-format.md`) gains a sound section: an offset and length in the
header's reserved bytes, covered by the pack's CRC, with a version byte of its own. The pack
version changes to 2.

The section holds, per game (by its index in the entry table), a list of learned entries. Each
entry carries what a built-in `snd_engine` carries, as data:

- a name for the report and the stats (for example "learned 1");
- digest, signature length, wildcard offsets and relocated-word offsets;
- sites (offset, role, channel), channel count, `pass`, `pos` per channel, `flips`, `min_len`,
  drum strength;
- per channel: the program that computes its period and level from the machine at the frame's
  end and the registers at the site's last edge (5.4);
- the timing program: how long the note has held at the site's last edge (`since`, `row`) and
  whether the engine moved on after it (`stale` 1), or a flag that it is not known;
- the stale rule: when the inputs are not to be read (the code that sets up the next note is
  running), as a pc range relative to the anchor, as `read_castaway` does today;
- **reserved for project 2:** an instrument number per channel (0 = today's patch);
- **reserved for project 3:** an entry kind (0 = music).

The firmware checks the section before using it:

- every offset and length within the section;
- counts and offsets within the limits the firmware's tables allow;
- programs within the interpreter's limits (length, stack depth, input addresses).

Anything wrong means no learned entries for that game, or for the whole pack if the section
header is bad. The games still run.

### 5.4 The firmware: the generic reader

**The interpreter.** A program is a short bytecode for a small stack machine. The opcode set is
the builder's call, but it must express:

- the spike's four forms: `a·x + b`, `a/x + b`, `a·x·y + c·x + b`, `a·x + c·x/y + b`;
- inputs: memory byte or word at an absolute address or relative to the anchor or to IX/IY;
  register values at the site's last edge or at the frame's end;
- the silence condition;
- the level;
- the timing outputs (`since`, `row`, moved on) from the duration counter's value at the site's last
  edge and at the frame's end, as `read_castaway` computes them from IX.

Bounded at run time: no loops, a fixed maximum length and stack depth. A division by zero means
silence. The result is a period in `snd_voice.period_q8` units (T-states × 256).

The interpreter and the generic reader live in flash, not RAMTEXT (constraint 3).

**Wiring.**

- When a game starts (`game_start`), method B's table becomes the built-in entries followed by
  this game's learned entries, decoded into RAM `snd_engine` records. The proposed limit is 8
  learned entries per game.
- `snd_engine.read` today takes no pointer to its entry. The generic reader needs one (to find its
  programs), so the reader signature or the entry record changes. The builder chooses how; the
  built-in readers must behave identically.
- The check, the decision, the hold, the delay and the mixer are unchanged. Learned entries are
  checked against the speaker edges every frame, exactly as built-in ones.
- **No regression:** a game with no learned entries runs method B with exactly the built-in table,
  sample for sample as today.

### 5.5 What the owner sees

- `make-personal-pack` prints a learned-music line per game and writes `sound-report.txt`.
- On the FM-1, FM: engines now plays music for more games. Nothing else changes: same menu, same
  60 ms delay, same fallback to the beeper.
- `docs/controls.md` (Sound row) and BUILDING.md ("Your own games") say that the personal pack
  learns music, and where the report is.

## 6. Later projects (not designed here)

- **2. Instruments.** The reserved instrument number per channel; the synth (`core/sound/fm.c`,
  one 2-op patch today) gains a small set of patches; the learner proposes one per channel (bass,
  lead, by register and role), and a per-game override in the games file.
- **3. Effects.** The reserved entry kind; the learner keeps the routines it now excludes as
  effects (arithmetic pitch, sweeps, noise) and translates the ones it can model. Effects
  happen in play, so coverage depends on step 1's random play, and on code reading for unheard
  ones.
- **4. Tapes on the device.** Independent of this design.

## 7. Testing and success

**Inner loop (host only, seconds).**

- **Unit tests** for the code reader and the fitter on generated Z80 routines. `fmzx-pack` already
  generates test programs (`testprogs.rs`): add engines of each family (time-slot, XOR flips,
  table-driven players), a one-channel toggler from two OUT sites and two XOR channels in unison
  (each must get its own partition), and an effect sweep that must be refused.
- **Built-in agreement:** with the built-in table disabled, the learner run on the repo's Castaway
  and Colony 8 must either produce entries whose formulas agree with the hand-written ones, or
  leave the routine out with a reason (Castaway's slides are expected to be left out). These are
  repo games, so this is a repo test.
- **No-regression:**
  - `fm_b_unclaimed_is_delayed_beeper`, `engines_claim_only_known_engines` and the mixer tests pass
    unchanged with an empty learned set;
  - non-music test programs (border, scroll, ZX Dino, One-Way, Sopwith in play) get no learned
    entries.
- **Corpus:** music-corpus gains a learned column. The FM: engines share with learned entries,
  per game, plus truth scores where truth readers exist (Cave Runner, Unison: the learned entries
  must score 0 wrong notes against them).

**Once per plan (firmware).** One emu-gate test: a pack built by `fmzx-pack personal` with a
learned entry for a repo test program plays it in FM in FM: engines, and fw_budget stays within
its margins. The full emu-gate suite runs once at the end, not per task.

**Success (proposed, the owner agreed to the outline).**

- On the owner's folder, games that have no FM: engines music today get it: Bee Line, Night Raid II
  and Star Pilot first, then whatever else the learner pins down. Each has 0 wrong notes where
  truth or a hand check exists.
- No false FM: no learned entry claims a sound effect in the corpus run.
- The folder analysis takes minutes; an unchanged game is not analysed again.
- Adding FM music for a new game is just rebuilding the pack.

## 8. Calls made in review (not the owner's)

- The octave doubt is settled by the code reading's counters (5.2, step 2), a required feature
  because Star Pilot is a target. The spike showed the edges cannot settle it.
- Learned entries carry note timing like the built-in readers (`since`, `row`, moved on) where a
  duration counter is found. Code-only entries require it. Per-call engines are not a learned
  family.

## 9. Left to the builder

- Crate and module layout; the trace hook's exact event set and buffer.
- The Z80 decoder (written for this or taken from an existing permissively licensed crate,
  checked against `reuse lint` and the project's licences).
- The loop families the code reader supports first, provided Bee Line, Night Raid II and Star Pilot
  are among the targets; which families come first is the builder's call.
- The bytecode's opcodes and encoding; the sound section's exact byte layout (documented in
  `docs/pack-format.md`).
- How `snd_engine.read` gets its entry; how learned entries are decoded into RAM.
- The thresholds marked "proposed" (run length, inlier share, claim share, entry limit, time
  budget), tuned on the repo's games and checked on the owner's corpus.
- The cache's location and format.
