<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# FM music from the beeper (Plans 5-7)

**Status:** approved by the owner, 2026-11-13; stage 1 delivered (Plan 5), 2026-11-14; stage 2 designed in §9, 2026-11-14. A new milestone after Plan 4. It changes the main design
(`2026-11-11-zx48k-fm1-design.md`): "the FM-1's synth side is used only to play the Spectrum's
own sound (the beeper)" no longer holds once this lands; the main design's §6 records it.

## 1. Intent

Spectrum games push the one-bit beeper into multi-voice music with drums by timing tricks. The
owner's idea (2026-11-13): reconstruct that music from what the Spectrum outputs and play it
through FM synthesis on the FM-1, "similar but much more high fidelity".

A throwaway spike (2026-11-13, host only, outside the repository) showed it works: decoders that
recover the voices from the beeper's edge stream reached 92-99% note accuracy with 0-1 wrong
notes on Castaway, Escape from Colony 8 and Cave Runner, once a decoder existed for each game's
**mixing style**; the owner listened to the FM renders and approved ("yep, sounds good").

**The owner's requirements (2026-11-13):**
- "decoders need to be able to adapt to any game, not only these few specific ones we have":
  no per-game configuration; the firmware works out what it hears.
- Three methods, **each a choice in the menu**: A (decode the sound), B (recognise known music
  engines in memory and read their notes), C (watch the CPU's sound loop).
- The choices are for **comparison**, not chained: each method is pure; what it cannot handle
  plays on the beeper.

## 2. Stages

Each stage is its own plan, lands on its own, and adds one menu choice:

| Stage | Plan | Adds | Menu after it |
|---|---|---|---|
| 1 | Plan 5 | FM synth, beeper/FM switching, the menu item, the local game-corpus report, method A | Beeper · FM: sound |
| 2 | Plan 6 | method B: engine table, notes read from RAM (first entries: the three engines the spike mapped) | + FM: engines |
| 3 | Plan 7 | method C, after its own throwaway spike proves it; if the spike fails, stage 3 stops there | + FM: CPU |

This document designs stage 1 in full and fixes the shared parts stages 2-3 build on. Stages 2
and 3 get their own design updates before their plans.

## 3. Behaviour (all stages)

- **Menu:** the HOME menu gains a "Sound" item that cycles through the available choices
  (like the LCD speed item). Default **Beeper** (the builder's call: no change for anyone who
  doesn't ask, and every existing sound test stays valid). The choice holds across games in a
  session; it is not saved to flash.
- **Beeper:** exactly today's path, byte for byte, with no added delay.
- **An FM choice:** every frame, the chosen method looks at the frame and says, for each voice
  it found, "note at this pitch and loudness" with a confidence. While the method is confident
  that it understands the sound as music, the FM voices play it; otherwise the frame's own
  beeper sound plays. The switch is **per moment**, not per game (title music in FM, effects in
  play on the beeper), with hysteresis so it does not flutter, and a short crossfade (a few ms)
  so it never clicks.
- **Delay (the owner's choice, 2026-11-13: "go with 60 ms"):** a method needs a short window to
  be sure of a note. In an FM choice all sound, FM and beeper alike, is delayed by one fixed
  amount, **3 frames (60 ms)**, the same for every style and in the browser page, so the switch is
  seamless; the method's look-ahead is that same window (one ring of the last frames' edges and
  beeper samples, not two mechanisms). Beeper mode has no delay. Accepted consequence: music
  too sparse to confirm within 60 ms (Cave Runner's in-game tune: a 9 ms burst about every
  78 ms) stays on the beeper under method A; method B (stage 2) can cover it. Stage 1 does so by
  rule: a beep filling less than 60% of its frame is no note, so the in-game bursts make no pitch
  change of their own and play on the beeper once the 2 s of music context a tune before them
  leaves have run out (`core/sound/decoder.c`). The owner's local corpus run (from
  the title tune into the game, 82.9% of its sounding frames in FM, 0 wrong notes) mixes title
  and in-game frames, so it is no measure of the in-game tune.
- **The bar is confidence, not coverage.** A wrong note in FM is worse than the gritty original;
  a method that is unsure must fall back.
- **FM voices:** up to three tonal voices and one noise/drum voice, each a simple 2-operator FM
  voice with an envelope; the patch is picked automatically from the voice's role (for example
  low voice = bass, high = lead, noise burst or fast falling pitch = drum). Pitch is the measured
  pitch, never snapped to semitones (Castaway's notes sit between semitones). Loudness follows
  the pulse width or density the method measured. No patch editing.
- **The browser page** runs the same firmware, so it gets the menu item and the sound with no
  page change.

## 4. Stage 1 (Plan 5): foundation and method A

### 4.1 Method A: decode the sound

From the frame's beeper edges alone (`zx_edge`: T-state and EAR/MIC level), with no knowledge of
the game:

1. **Style detection:** decide which mixing style, if any, the recent edge stream shows. Stage 1
   covers the three styles the spike found:
   - **time slots:** a fixed engine loop gives each voice its own slot (Castaway, Colony 8);
   - **speaker flips:** the voices' counters each flip the speaker when they wrap (XOR mixing,
     Cave Runner);
   - **short pulses:** each voice emits a short pulse per period (pin-pulse engines; tested on a
     synthetic signal in the spike).
   None of these → not music this moment → beeper.
2. **Decode** with that style's decoder into voices (pitch, loudness, on/off) and drum events.
3. **Confidence** per moment from the decoder's own consistency (for example how well the edges
   fit the decoded voices, pitch stability, voice continuity); below the threshold → beeper.

More styles are added later as the corpus report (4.3) shows music falling back to the beeper.

### 4.2 Where it runs

- Portable C in `core/` beside the Spectrum machine (same code on the device, in the host tests
  and in the browser's wasm build), behind a small interface: feed a frame's edges, get the
  frame's samples. Hot code in RAMTEXT where the firmware needs it.
- Core 0 only (the main design: some units misread memory on core 1). The work runs in the main
  loop per frame, where the beeper is rendered today, not in the audio interrupt.
- **Budget:** today the busiest game uses about 1.5 M of the 4.8 M instructions a frame allows
  (`BUILDING.md`). FM rendering plus method A must keep every game's worst frame inside the
  budget with margin, RAMTEXT inside 49,152 bytes, and the audio interrupt unchanged;
  `fw_budget` checks all three with an FM choice active.

### 4.3 The local game-corpus report

How "adapts to any game" is measured, not just claimed. The owner has games that cannot go in
the repository (for example Cave Runner, Unison). A host tool takes a folder of games given at run
time — `.tap`, `.sna`, `.z80`, and `.tzx` when every block is standard-speed (converted on the
fly) — runs each for a while (title screen by default; an optional per-game key script beside the
file for in-game music), and reports per game: how much of the time sound was playing, how much
of that went to FM, which styles were detected, and, where a ground-truth note reader exists for
the game's engine (the spike's RAM addresses for Castaway, Colony 8 and Cave Runner; stage 2's
table later), note accuracy and wrong notes. It can also write the original and FM renders as
WAV files for listening. Nothing from the folder is ever copied into the repository; reports go
to `build/`.

### 4.4 Tests (committed)

- Synthetic edge streams per style with known notes (generated, like the test programs).
- `tune.sna` (one voice, known scale) and the pack's music: Castaway's title tune and Colony 8's
  music, scored against note readers at the RAM addresses the spike found (test code).
- Confidence: non-music beeper sound (the border tone, sound effects from the pack's games)
  must stay on the beeper.
- Beeper mode output byte-identical to today; existing sound tests unchanged.
- Firmware under fm1-emu: the menu item cycles the choices; with FM chosen the audio of Castaway's
  title shows the decoded pitches; no audio underruns; budgets.

## 5. Goals, non-goals, anti-goals

**Goals:** §3 and §4 for stage 1; the shared interface stages 2-3 plug into.

**Non-goals (stage 1):** methods B and C (stages 2-3); user-editable FM patches; a "best"
chained choice (may come once all three exist); AY or 128K music; MIDI out; saving the sound
choice; USB game upload.

**Anti-goals:**
- Wrong notes played confidently in FM.
- Clicks, gaps or flutter at the switch between FM and beeper.
- Any change to Beeper mode, or a game slowed down by the FM work.
- Any third-party game file in the repository for testing (the corpus stays local).

## 6. Approaches considered

- **A, decode the sound:** general, no game knowledge, proven by the spike on three styles;
  uncovered styles fall back to the beeper. Stage 1.
- **B, recognise known engines:** exact and instant for engines in a table, which then work in
  every game that uses them; per-engine work. Stage 2.
- **C, watch the CPU's sound loop:** potentially exact and general; unproven research. Stage 3,
  spike first.
- The owner chose all three as menu choices, pure, for comparison (2026-11-13).

## 7. Settled after the builder check (the designer's calls; the owner may overrule)

- **Starting point is the spike.** Its source (Rust, ours, GPL) and `findings.txt` are kept in
  `build/spike-beeper-fm/` (untracked; the plan copies what it needs). Stage 1 ports the style
  selector and the time-slot and speaker-flip decoders to C and hardens them, and writes a
  short-pulse decoder from the spike's synthetic test; the spike's spectral decoder is not
  ported (it played 20-50% wrong notes). The spike's thresholds are starting points, not
  requirements.
- **Done for stage 1** is the committed tests passing plus one corpus report run on the owner's
  local folder, recorded in the plan's report. An unseen engine falling back to the beeper is an
  acceptable outcome; there is no coverage bar on the local corpus.
- **Gates for the committed tests:** on the pack's music (Castaway's title, Colony 8) at least
  90% frame precision and no wrong notes outside pitch slides, scored against the RAM note
  readers; on `tune.sna` every note right; on non-music sound (the border tone, every pack
  game's sound effects during its demo) **no FM at all** — zero false positives. The spike's
  Cave Runner results are not a committed gate (it cannot be in the repository); the corpus
  report covers it locally.
- **State and memory:** the decoder keeps its history across frames in its own state struct
  (statically allocated, sized for the 3-frame window); the FM voices render a frame's samples at
  once, where the beeper renders today, into the existing FIFO path. The builder states the RAM
  used in the plan and `fw_budget` checks it.
- **If the budget is tight:** fixed-point FM with a sine table is expected anyway; cut in this
  order: drum voice detail, then the third tonal voice, never correctness of the notes kept.
- **Corpus tool:** a new Rust binary under `host/` that drives the C core and the C decoders
  through `zx-sys` (no second implementation in Rust); the standard-speed `.tzx` conversion is
  new, small code (the spike has a working one).
- **Menu:** the Sound item is always shown (Beeper and FM: sound in stage 1). The choice survives
  reset game, back to the picker and starting another game, until power off.

## 8. Left to the builder

File and function names, the decoders' algorithms (starting from the spike, recorded in its
`findings.txt` and to be summarised into the plan), the confidence measure and thresholds, the
exact delay, voice patches, the corpus tool's command line and key-script format, test names.

## 9. Stage 2 (Plan 6): method B, "FM: engines"

### 9.1 Intent and the owner's choice

Method B recognises a known music routine (an **engine**) in the running game and reads the notes
it plays from its variables and registers, so it does not guess. It is the second FM choice in the
Sound menu, for comparison with method A, and it is pure: anything it does not recognise plays on
the beeper (§1). Its use beside A: it is exact and instant where it applies, and it covers music
A leaves on the beeper by rule (the sparse Cave Runner in-game tune, §3).

**The engine table (the owner's choice, 2026-11-14: "your pick" of the options offered):**
- the four engines the spike mapped: Castaway's title tune, Escape from Colony 8's music, Cave
  Miner's two routines (title, in game), and Unison's;
- **the ROM's BEEP routine** (BEEPER, 03B5h-03F7h in the 48K ROM), which BASIC's `BEEP` and many
  machine-code games call: one small entry that covers far more games than any single engine.

Published shared engines (Beepola and other 1-bit tracker players) are **not** in stage 2; they
can join the table later through the same entry format, each with its licence checked and a test
tune.

### 9.2 Behaviour

- **Menu:** Sound cycles Beeper · FM: sound · FM: engines (`SND_MODE_FM_B`, before
  `SND_MODE_COUNT`). Same holding rules as §7.
- **Delay and switching:** shared: 60 ms in every FM choice, the same mixer and crossfades. B
  does not need the look-ahead; the delay stays so that the three choices compare like for like and
  switching between them is seamless (the designer's call; the owner may later ask for B without
  the delay). The mixer's 2-frame entry rule is A's guard against guessing; in FM: engines a frame
  B claims switches to FM at once (the exit rule and the crossfades are unchanged).
- **Timing inside the frame:** B knows when in the frame a note starts and stops (the T-states of
  the engine's `OUT`s), so its result carries note events with their position in the frame, and
  the FM voices start and stop there, not at the frame edge. A 9 ms burst is a 9 ms note; two BEEPs
  that meet inside one frame are two notes. Method A keeps its per-frame result (its events all sit
  at the frame start), so stage 1's output does not change.
- **A tune between bursts:** a recognised engine counts as still playing for up to 120 ms after its
  last `OUT` (the spike's hold across silence), so the gaps of a burst tune (Cave Runner in game: a
  9 ms burst about every 78 ms) are B's confident silence, in which the FM notes ring out on their
  envelopes, instead of flipping to the beeper between bursts. Any frame in which other code drives
  the speaker ends that.
- **Per frame:** B says either "engine E is playing these voices" (FM) or "not mine" (beeper). A
  frame is B's only when every speaker edge in it came from code of a recognised engine (9.3), or
  it falls in an engine's hold (above) with no edges from other code, **and** the voices B read
  agree with the frame's edges (9.4). An `OUT` that leaves the speaker bits unchanged (a border
  write) is no speaker edge and does not count. A frame where an engine and other code both drive
  the speaker (Cave Runner's jump sound over its tune) plays on the beeper, so no effect is lost. Rests inside a playing tune (the
  engine's loop runs with a channel muted) are confident silence for that voice.
- **Drums:** where an entry names its drum routines (Colony 8's 9496h tone burst and 94ADh ROM
  noise, Unison's four drums in its interrupt routine at 8257h, 826Ah, 827Eh and 8292h, so its
  drums-only intro and its drum frames between arpeggios are FM drums), frames driven from them are drum events on the noise voice;
  an entry without drum routines leaves those frames to the beeper.
- **Voices:** each engine channel keeps its own FM voice for as long as the tune plays (no voice
  swapping); the patch follows the role as in §3. Loudness follows the engine's own duty or mask
  where the entry knows it, a fixed level otherwise.
- **ROM BEEP:** a BEEP whose whole length is at least 20 ms (worked out at its first `OUT` from
  the registers: DE the cycles, H/L/IX the pitch) is a note in FM at the exact pitch, starting and
  ending where it does in the frame; a shorter one (key clicks, which the ROM makes through BEEPER, and blips) stays
  on the beeper. Accepted consequence: a game's effects built from long BEEPs play in FM, at
  their exact pitches.
- **Effects:** sound from code outside every entry (most games' effects) is never B's, so it plays
  on the beeper; the committed "no FM on non-music sound" gate (§7) applies to B unchanged.

### 9.3 Recognising an engine wherever it sits

The fixed-address fingerprints of Plan 5 (`host/crates/music/src/corpus.rs`) recognise an engine
only at the address one game put it. B anchors on the code that drives the speaker instead:

- **The core records, per frame,** the distinct addresses of `OUT` instructions that wrote port
  FEh (a small fixed number of them, with a count each) and the CPU registers at the last such
  `OUT` from each address. It is observation only: no change to emulation or timing, and Beeper
  mode's output stays byte-identical.
- **An entry** is: a signature (bytes at fixed offsets from the `OUT`'s address, self-modified
  operands left out, hashed with the existing FNV-1a 64 `digest`); the reader's addresses, each
  either relative to the matched code or taken from an operand inside it (so a moved engine still
  reads the right variables; Cave Runner's in-game note index at 845Bh is read through the
  instruction that loads it); the registers it reads (Colony 8's steps in BC/DE/SP, Cave Runner's
  IY, BEEP's H/L/IX); the pitch formula per channel (the spike's, §6 of its findings: for example
  Castaway f = 3.5 MHz / ((wrap + 1) x N x loop)); and its drum routines, if any.
- **Matching is per frame** on the recorded addresses only (no memory scan), so B's cost is a few
  digests and reads a frame. A match is remembered per address while the bytes stay the same.
- **The ROM BEEP entry** matches the ROM's own bytes the same way, so a machine with another ROM
  simply does not match.
- **Third-party engines:** only digests, offsets and formulas go in the repository, never a byte of
  Cave Runner's or Unison's code (as for Plan 5's fingerprints). Their entries are tested only on
  the owner's local corpus.

### 9.4 The check against what is heard

A routine that matches the signature but is a modified variant, or a reader formula that is wrong
in a case nobody tested, must not put wrong notes in FM. Each frame B claims is checked against
the frame's own beeper edges: the voices B read must be consistent with what the speaker did (for
example, the edge count or the spacing of edges the read pitches imply). A frame that fails stays
on the beeper. The exact check and its tolerance are the builder's; it uses the edges only as a
check and never decodes from them (B stays pure).

### 9.5 Tests (committed)

The truth readers that score method A read the same memory B reads, so scoring B against them alone
proves only that two implementations agree. The committed gates are therefore:

- **Readers against the Rust truth readers** (`music::truth`), on Castaway's title and Colony 8:
  the same notes on at least 99% of the frames the engine plays (an implementation check: C
  firmware code against Rust test code).
- **B against what is heard, independently:** on the frames where method A is confident, B and A
  agree on every voice within the existing tolerance (at least 95% of those frames, no wrong
  notes); and for ROM BEEP, B's pitch matches the period measured directly from the edges.
- **ROM BEEP test program:** a generated BASIC tape of `BEEP` statements (a known melody, host test
  fixture, not added to the pack): every note right in FM, each starting within 1 ms of its first
  beeper edge and lasting within 2 ms of the beeper's length; a `BEEP` shorter than 20 ms stays on
  the beeper.
- **Burst tune and mixed frames:** a test-built stream of 9 ms bursts every 78 ms from one engine
  plays in FM through the gaps; the same with another routine's edges in one frame puts that frame
  on the beeper.
- **Moved engine:** recognition does not depend on where the engine sits: a test that runs an
  engine from another address (for example a test-built memory image with Castaway's engine
  copied elsewhere) gets the same notes.
- **Variant rejection:** a frame whose edges disagree with the readout (for example the same
  engine with a reader operand altered in a test-built image) stays on the beeper.
- **No FM on non-music sound** (§7's gate) with B chosen: the border tone, every pack game's
  effects during its demo, noise.sna.
- Firmware under fm1-emu: the Sound item cycles three choices and survives as in Plan 5; with FM:
  engines chosen Castaway's title audio shows the read pitches; budgets with B active.

### 9.6 Local corpus

`music-corpus` reports B beside A for each game: FM share, engines recognised, and, where a truth
reader exists, notes found and wrong notes. A Unison truth reader (the spike's: operands 81CDh /
81DFh, read while the PC is in 81B4h-81E9h) joins `music::truth` for this. Cave Runner and Unison
stay local files, as before.

### 9.7 Settled here (the designer's calls; the owner may overrule)

- The 60 ms delay is kept for B (9.2).
- BEEPs under 20 ms stay on the beeper; long BEEP effects play in FM (9.2).
- The BEEP test program is a host fixture, not a pack entry (the pack's order and count stay).
- Stage 2 changes the core only by the observation in 9.3.
- B enters FM without the 2-frame rule; an engine holds for 120 ms after its last `OUT`; mixed
  frames play on the beeper; border-only writes do not count (9.2).
- B's result carries note events positioned inside the frame; A's are all at the frame start, so
  stage 1 is unchanged (9.2).
- All four of Unison's drums are B's (9.2).
- If a stage-1 rule (busy latch, music context, first-note rule) would hold B back, it does not
  apply to B: those rules exist to make A's guesses safe, and B does not guess; 9.4 is B's safety
  rule.

### 9.8 Left to the builder

The core's recording format and limits (it must tell each speaker edge's source address, for the
mixed-frame rule), the event format in the frame result, the entry format in C, the consistency check's form and
tolerance, how B's state lives beside the decoder, each entry's signature offsets, the tests'
names and the BEEP fixture's melody.
