<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# FM music from the beeper (Plans 5-7)

**Status:** approved by the owner, 2026-11-13. A new milestone after Plan 4. It changes the main design
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
  78 ms) stays on the beeper under method A; method B (stage 2) can cover it.
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
