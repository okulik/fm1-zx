<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# FM Music, Stage 1 (Plan 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Sound" choice in the HOME menu (Beeper · FM: sound) that, in FM, re-synthesises the games' beeper music as FM voices decoded from the beeper's edge stream alone (method A), falling back to the beeper whenever unsure, plus a local tool that measures it over any folder of games.

**Architecture:** A new portable C library `core/sound/` (integer arithmetic only, so host, device and wasm give identical samples): an edge-history decoder with a style selector and three style decoders produces, per frame, voices and a confidence; an FM synth renders voices; a mixer delays FM and beeper alike by 3 frames and switches between them per moment with hysteresis and a crossfade. The firmware's frame loop calls the mixer instead of the plain beeper renderer when an FM choice is active. A new host crate `host/crates/music` holds ground-truth note readers, synthetic test signals, scoring and the corpus tool, all driving the C code through `zx-sys`.

**Tech Stack:** C (core, firmware; JieLi clang in Docker via `tools/fm1-docker`), Rust host workspace, fm1-emu via emu-gate.

**Spec:** `docs/superpowers/specs/2026-11-13-fm-music-design.md` (approved). Reference: the throwaway spike in `build/spike-beeper-fm/` (Rust source `src/`, `out/findings.txt`: algorithms, thresholds, scores, RAM addresses). It is a reference to port from, not code to copy wholesale; it is not committed.

## Global Constraints

- Commit messages: a single line, no trailers (no Co-Authored-By). Never name projects other than this one and the games' upstreams in files or commits.
- Device rule: never flash an FM-1, send it SysEx or open MIDI ports. Everything runs on the host and in fm1-emu.
- `core/sound/` is portable C, integer/fixed-point only (no float, no libm), no dynamic allocation, built with `-Wall -Wextra -Werror`, compiled into the host (`zx-sys` build.rs), the firmware (CMake) and the wasm build exactly as `core/zx/` is. Host and device output must match sample for sample.
- **Beeper mode is byte-identical to today**: same samples, no delay. Every existing test keeps passing unchanged.
- FM choice: one fixed delay of `SND_DELAY_FRAMES = 3` frames (60 ms) for all sound; the decoder's look-ahead is that same window.
- Voices: `SND_TONAL = 3` tonal voices plus one drum voice. Pitch is the measured pitch, never snapped. 2-operator FM, fixed-point, sine table.
- Confidence over coverage: below the threshold the frame plays the beeper. Zero FM on non-music sound (gates in Task 4).
- Switching per moment with hysteresis and a crossfade of at least 2 ms; no sample step at a switch larger than the larger of the two signals' own largest step in the surrounding 5 ms.
- Firmware budgets: every game's worst frame within `fw_budget`'s cap with FM active; RAMTEXT ≤ 49,152 bytes; audio interrupt ≤ 8,000 instructions (unchanged path).
- Menu: the "Sound" item is always shown, cycles Beeper → FM: sound → Beeper; default Beeper; the choice survives reset game, back to the picker and starting another game; not saved to flash.
- Third-party games (Cave Runner, Unison, any owner file) never enter the repository; the corpus tool reads them from a folder given at run time and writes only to `build/`.
- Commands:
  - host: `cd host && cargo test --workspace --exclude emu-gate`
  - device builds (C changes in `core/`, `app/`, `platform/`): `tools/fm1-docker cmake --build --preset fm1 && tools/fm1-docker cmake --build --preset fm1-preview`, then `FMZX_SKIP_DEVICE_BUILD=1 tools/make-packages --offline`
  - firmware: `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate`
  - licences: `reuse lint`; browser: `tools/web-demo --check --skip-firmware`

## Interfaces (fixed for all tasks)

```c
/* core/sound/sound.h */
#define SND_DELAY_FRAMES 3u
#define SND_TONAL 3u
enum snd_style { SND_STYLE_NONE, SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES };
enum snd_mode { SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_COUNT }; /* Plans 6-7 add FM_B, FM_C before COUNT */

typedef struct {
    uint32_t period_q8;   /* full period in T-states, Q8 fixed point; 0 = voice off */
    uint8_t level;        /* 0-255 loudness */
} snd_voice;

typedef struct {          /* what the decoder concluded about one past frame */
    snd_voice tone[SND_TONAL];
    uint8_t drum;         /* 0 = none, else drum strength 1-255 */
    uint8_t style;        /* enum snd_style */
    uint8_t confident;    /* 1 = play FM for this frame */
} snd_frame;

typedef struct snd_decoder snd_decoder;   /* edge history (SND_DELAY_FRAMES + current) and per-style state */
typedef struct snd_fm snd_fm;             /* voice phases, envelopes */
typedef struct snd_mixer snd_mixer;       /* delay rings, hysteresis, crossfade; owns a decoder and an fm */

void snd_decoder_init(snd_decoder *d);
/* Feeds frame t's edges; fills *out with the decision for frame t - SND_DELAY_FRAMES. */
void snd_decoder_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out);

void snd_fm_init(snd_fm *f, uint32_t rate_num, uint32_t rate_den);
void snd_fm_render(snd_fm *f, const snd_frame *fr, int16_t *out, uint32_t n);

void snd_mixer_init(snd_mixer *m, uint32_t rate_num, uint32_t rate_den);
/* beeper: this frame's n beeper samples (from zx_beeper_render). Writes n samples to out:
 * SND_MODE_BEEPER -> a copy of beeper (no delay); FM modes -> 3-frame-delayed FM or beeper. */
void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_edge *e, uint32_t n_edges, uint8_t start_level,
                     const int16_t *beeper, uint32_t n, int16_t *out);
```

The struct bodies live in `sound.h` (so callers can allocate them statically); their fields are the builder's. `snd_voice.period_q8` is the voice's full period in Z80 T-states × 256 (frequency = 3,500,000 × 256 / period_q8). Rust wrappers in `zx-sys`: `SndDecoder`, `SndFm`, `SndMixer` mirroring the functions above, plus `SndFrame`/`SndVoice` as `#[repr(C)]` structs.

## Review Focus

1. **A mode switch mid-game.** Turning FM on or off in the menu while a game plays must not click, repeat or drop 60 ms of sound: switching to Beeper flushes the delay ring (a crossfade from the delayed signal to the live one), switching to FM starts with the beeper for the first 3 frames. Test in Task 1.
2. **Silence and rests inside music.** A rest of a few frames in decoded music must not be filled with held notes, and must not flip to the beeper and back with clicks (hysteresis keeps FM through short silences only if the decoder still calls them music; otherwise crossfade). Test in Task 2.
3. **Sound effects that look like music.** Steady tones (the border test's 976 Hz), pitch sweeps (Dino's jump, Sopwith's shots) must stay on the beeper. Gate in Task 4.
4. **Games that stop the sound loop for drums or speech** (Colony 8's drum routines stop the music loop): the decoder must not lose the voices for the rest of the tune; voices resume after the drum. Test in Task 3.
5. **Game load and reset.** `zx_init`/snapshot load/reset clears the decoder and the delay rings, so no tail of the previous game's music plays into the next. Test in Task 5.

---

### Task 1: The sound library skeleton: FM synth, mixer, delay, switching

**Files:**
- Create: `core/sound/sound.h`, `core/sound/fm.c`, `core/sound/mixer.c`, `core/sound/decoder.c` (stub: always `SND_STYLE_NONE`, not confident; filled in by Tasks 2-4), `core/sound/CMakeLists.txt` or additions to `core/CMakeLists.txt` (library `zx_sound` or added to `zx_core`, same flags and `-Werror` as `core/zx/`)
- Modify: `host/crates/zx-sys/build.rs` (compile `core/sound/*.c`, include dir), `host/crates/zx-sys/src/lib.rs` (wrappers)
- Create: `host/crates/music/` (new crate in the workspace: `Cargo.toml`, `src/lib.rs`, `tests/fm.rs`, `tests/mixer.rs`)
- Modify: `REUSE.toml` only if new paths need annotations (SPDX headers in every new file)

- [ ] **Step 1: Failing tests** in `host/crates/music/tests/`:
  - `fm_pitch`: render 1 s of one tonal voice at period_q8 for 440 Hz (and 110 Hz, 1,760 Hz) with `SndFm` at 750000/17; zero-crossing frequency of the fundamental (low-pass or count carrier crossings at ratio 1:1, index decayed) within 0.5% of the target.
  - `fm_silence`: a frame with all voices off and no drum renders all zeros after the release time (≤ 60 ms).
  - `beeper_mode_is_identical`: for 200 frames of `border.sna` and of Castaway's tape, `SndMixer` in `SND_MODE_BEEPER` returns exactly the beeper samples it was given, frame by frame (no delay).
  - `fm_mode_delays_three_frames`: in `SND_MODE_FM_A` with the stub decoder (never confident), the output for frame t equals the beeper samples of frame t−3 (frames 4+), and frames 1-3 are silence.
  - `mode_switch_no_click`: switch Beeper→FM and FM→Beeper mid-`border.sna`; at each switch the largest sample step within ±5 ms is no larger than the largest step in the beeper signal itself in those 10 ms (the crossfade constraint), and no sample is repeated as a block of 60 ms.
- [ ] **Step 2: Run** `cd host && cargo test -p music` — FAIL (no crate/library).
- [ ] **Step 3: Implement** the library: `snd_fm` (fixed-point 2-op FM, 1:1 ratio, index decaying per the spike's `synth.rs`: 2.5→1.0 for bass, 1.6→0.6 above, over 120 ms; ADSR 4 ms / 0.75 / 50 ms; drum = noise burst with decay; a 256- or 1024-entry sine table in `const` data); `snd_mixer` (delay rings for beeper samples and decoder output sized for 3 frames of the maximum samples per frame, the confident/beeper decision per frame with hysteresis, crossfades of ≥ 2 ms at decision changes and mode changes, `SND_MODE_BEEPER` a plain copy); the decoder stub.
- [ ] **Step 4: Run** the host command — PASS. **Step 5: Commit** `git commit -m "Add the sound library: fixed-point FM synth, 3-frame delay mixer with crossfades, beeper bypass"`

### Task 2: Edge history, the speaker-flip decoder, and confidence

**Files:**
- Modify: `core/sound/decoder.c` (edge history over `SND_DELAY_FRAMES + 1` frames with absolute T-states; the style selector skeleton; confidence and the frame decision), Create: `core/sound/flips.c`
- Create: `host/crates/music/src/synthetic.rs` (generators of edge streams with known notes per style), `host/crates/music/src/score.rs` (frame precision/recall and note-level scoring, ±0.5 semitone, ±1 frame, as the spike's `score.rs`), `host/crates/music/tests/flips.rs`
- Modify: `host/crates/zx-sys/src/lib.rs` if needed

The speaker-flip decoder ports the spike's `toggles.rs`: every edge is a half-period of one voice; take the shortest half-period whose chain runs through the window, remove those edges, repeat (up to 3 voices); 28 T tolerance; always choose the shortest chaining period (a 3:4 ratio must not come out as the joint period). **No hold across silence** (the spike's 120 ms hold was for Cave Runner's in-game bursts, which the 60 ms delay leaves on the beeper — design §3). The window is the decoder's 3-frame look-ahead plus the frame itself.

**Note timing adapts by itself** (spike finding on Unison, `findings.txt` Unison section): engines in this style play either long notes changing anywhere in a frame (Cave Runner's title: a 40 ms window and minimum note) or one note per 50 Hz frame from an interrupt sequencer (Unison: 20 ms frame-aligned windows and a 20 ms minimum note). Neither timing works for both: the frame-aligned one splits Cave Runner's long notes (44 wrong notes), the 40 ms one smears Unison (55% precision). The decoder detects which it is from the stream — whether pitch changes land on frame boundaries — and uses the matching timing, switching only with hysteresis. **Buzz drums** (a fixed pattern of short toggles, Unison's drum at 826Ah) must not be decoded as chords: frames that are a drum pattern report `drum` and no tonal voices, or are not confident.

- [ ] **Step 1: Failing tests** in `tests/flips.rs`:
  - `tune_exact`: run `games/test/tune.sna` 450 frames; every frame inside a note (excluding 2 frames at each seam) is confident, style FLIPS, one tonal voice within 0.5% of `TUNE_HZ[i]`.
  - `synthetic_xor_two_voices`: a generated XOR-mixed stream (two counters flipping one speaker, the spike's Cave Runner loop model: 56.6 T iterations, detuned unison and a 3:4 interval, 20 notes) → frame precision ≥ 95%, recall ≥ 90%, 0 wrong notes.
  - `rest_not_filled`: the same stream with 4-frame rests between notes: no voice reported during rests (after the 3-frame confirmation), and the decision does not flip to beeper-then-FM more than once per rest.
  - `synthetic_frame_notes`: a generated XOR stream in the Unison model (one voice doubled at N and N−1, 48.5 T per pass, a 4-note arpeggio changing pitch exactly at frame boundaries, one note per frame, with whole drum frames of a fixed buzz pattern in between) → frame precision ≥ 95%, recall ≥ 85%, no wrong notes on drum frames (drum frames report a drum and no tonal voice, or are not confident).
  - `timing_adapts`: the Cave Runner-model stream above and the Unison-model stream, back to back in one run: each part meets its own gate (the long-note part gives no split notes: ≤ 1 wrong note).
  - `steady_tone_not_music`: the border test's 976 Hz tone (`border.sna`) gives zero confident frames (a single constant tone with no note changes for > 2 s is a test tone or effect, not music — the confidence rule's builder-chosen criterion must reject it).
- [ ] **Step 2: Run** `cd host && cargo test -p music --test flips` — FAIL.
- [ ] **Step 3: Implement** history, selector (FLIPS for now, NONE otherwise), `flips.c`, confidence.
- [ ] **Step 4: Run** the host command — PASS. **Step 5: Commit** `git commit -m "Decode speaker-flip beeper music from the edge stream, with confidence"`

### Task 3: The time-slot decoder and the pack's music

**Files:**
- Create: `core/sound/slots.c`; Modify: `core/sound/decoder.c` (selector: a steady loop carrier → SLOTS)
- Create: `host/crates/music/src/truth.rs` (RAM note readers: Castaway and Colony 8 from `findings.txt` §6; Cave Runner title/in-game too, for the corpus tool), `host/crates/music/tests/slots.rs`

The time-slot decoder ports the spike's `demux.rs`: estimate the loop length P from the edge stream, average over a box of exactly P to get Σ wᵢ·sᵢ, learn the levels then the weights, decode each slot's channel state, measure each channel's period from its own edges. Colony 8's drum routines stop the loop (Review Focus 4): the decoder reports a drum and keeps the channel model so the voices resume.

- [ ] **Step 1: Failing tests** in `tests/slots.rs` (scored with `score.rs` against `truth.rs`):
  - `castaway_title`: Castaway's tape, autostart, frames from 3 s to 43 s: frame precision ≥ 90%, no wrong notes outside pitch slides (slides = truth pitch changing by ≥ 1 semitone within 3 frames; excluded from the wrong-note count), style SLOTS in ≥ 90% of music frames.
  - `colony8_music`: Colony 8 from 2.5 s to 31 s with the title's music (no keys): frame precision ≥ 90%, at most 1 wrong note, drums reported where the drum routine runs and the voices present within 3 frames after each drum.
  - `synthetic_slots_three_voices`: a generated 3-slot TDM stream (153 T loop, slots 34/49/70 T, 16-bit phase accumulators, 30 notes) → precision ≥ 95%, 0 wrong notes.
  - `flips_still_pass`: Task 2's tests still pass with the selector choosing between FLIPS and SLOTS.
- [ ] **Step 2: Run** `cd host && cargo test -p music --test slots` — FAIL.
- [ ] **Step 3: Implement** `slots.c`, the selector rule, `truth.rs`.
- [ ] **Step 4: Run** the host command — PASS. **Step 5: Commit** `git commit -m "Decode time-slot beeper music; Castaway and Colony 8 scored against their RAM notes"`

### Task 4: The short-pulse decoder and the no-false-FM gates

**Files:**
- Create: `core/sound/pulses.c`; Modify: `core/sound/decoder.c` (selector: speaker high under 20% of the busy time → PULSES)
- Create: `host/crates/music/tests/pulses.rs`, `host/crates/music/tests/no_false_fm.rs`

The short-pulse decoder ports the spike's `pulses.rs` (the flip chain search on rising edges).

- [ ] **Step 1: Failing tests:**
  - `synthetic_pinpulse`: a generated pin-pulse stream (3 voices, short pulses per period, 30 notes) → frame precision ≥ 94%, recall ≥ 45%, at most 1 wrong note per 30 (the spike's level; low recall falls back to the beeper, which is allowed).
  - `selector_picks_right`: on each synthetic style and on tune/Castaway/Colony 8 excerpts, the style reported in confident frames is the right one in ≥ 98% of them.
  - `no_false_fm` (Review Focus 3), zero confident frames on: `border.sna` 60 s; `scroll.sna`; `tape.tap`; each of ZX Dino, A World of One-Way, ZX Sopwith driven by its `games.toml` demo for 36 s (they have effects, no music); Castaway and Colony 8 during play after their demos start (effects only — Castaway's game is silent, Colony 8's in-game effects); a generated set of effect-like signals (single sweeps up/down of 50-300 ms, white-noise bursts, a 2-tone alternating siren, a constant tone for 3 s).
- [ ] **Step 2: Run** `cd host && cargo test -p music --test pulses --test no_false_fm` — FAIL.
- [ ] **Step 3: Implement** `pulses.c` and tune the confidence rule until every gate passes without lowering Task 2-3's thresholds; record the final thresholds and why in `core/sound/decoder.c`'s header comment.
- [ ] **Step 4: Run** the host command — PASS. **Step 5: Commit** `git commit -m "Decode short-pulse beeper music; no FM on effects, test tones or silence"`

### Task 5: The firmware: Sound menu item and the FM path

**Files:**
- Modify: `app/menu.h`, `app/menu.c` (a "Sound" item cycling `SND_MODE_*` names "Beeper" / "FM: sound", `CMD_SOUND`), `app/ui.c` if it draws menu items, `platform/fm1/main.c` (a static `snd_mixer`; in the frame function, after `zx_beeper_render`, call `snd_mixer_frame` with the current mode and queue its output; reset the mixer on game start, reset and snapshot load), `platform/fm1/*.ld` / section attributes so the decoder and FM hot code sit in RAMTEXT, `core/CMakeLists.txt` / `platform/fm1/CMakeLists.txt` (link `core/sound`)
- Modify: `host/crates/zx-tests/tests/app_*.rs` (menu unit tests via zx-sys if menu tests live there), `host/crates/emu-gate/tests/fw_audio.rs`, `fw_controls.rs`, `fw_budget.rs`

- [ ] **Step 1: Failing tests:**
  - host menu test: the Sound item exists, cycles Beeper → FM: sound → Beeper, and its value survives `CMD_RESET`, leaving to the picker and `CMD_START` of another game.
  - `fw_audio`: `sound_menu_switches_to_fm` — start Castaway, wait for its title music, HOME → Sound → FM: sound → resume; after 200 ms the firmware audio (captured as `tone_frequency` does) shows the decoded voices' fundamentals (compare against `truth.rs` pitches over 2 s: ≥ 90% of 20 ms windows within 1% for the strongest voice) and no underruns; switching back to Beeper restores the beeper signal (equal to the host beeper render of the same frames, delay-free) after the crossfade.
  - `fw_audio`: `beeper_default_unchanged` — every existing fw_audio test passes unchanged (Beeper is the default).
  - `fw_controls`: `sound_choice_survives_reset_and_new_game`, and (Review Focus 5) after switching games with FM on, the first 3 frames of the new game contain no samples from the old game's music.
  - `fw_budget`: run every pack game's demo with FM on; worst frame within the existing cap; RAMTEXT ≤ 49,152; audio interrupt ≤ 8,000.
- [ ] **Step 2: Run** the firmware command — FAIL.
- [ ] **Step 3: Implement**; device builds; repack.
- [ ] **Step 4: Run** host, device builds, repack, firmware command, `tools/web-demo --check --skip-firmware` — PASS; view any changed golden PNG (the menu golden will change). **Step 5: Commit** `git commit -m "Sound menu item: FM music from the beeper in the firmware, 60 ms delay, beeper by default"`

### Task 6: The local game-corpus tool

**Files:**
- Create: `host/crates/music/src/bin/music-corpus.rs`, `host/crates/music/src/tzx.rs` (standard-speed `.tzx` → `.tap` in memory: only block 0x10 carries data; 0x20-0x35 and 0x5A are skipped; anything else → the game is reported as "not loadable" and skipped), `host/crates/music/src/wav.rs`
- Modify: `BUILDING.md` (a "Music corpus" section: command, folder layout, key scripts, what the report means)

Command: `cd host && cargo run --release -p music --bin music-corpus -- <folder> [--seconds N] [--wav] [-o build/music-corpus]`. For every `.tap`/`.tzx`/`.sna`/`.z80` in the folder: load (tape autostart), optionally apply `<name>.keys` beside it (the `games.toml` demo syntax: `key.ENTER@8000`, `key.SPACE@9000+200`, …), run N seconds (default 40), feed the decoder, and report one row per game: seconds with sound, % of those frames in FM, styles seen (% each), and — if `truth.rs` recognises the engine (Castaway, Colony 8, Cave Runner) — frame precision, recall, notes found, wrong notes. `--wav` writes `<name>_beeper.wav` and `<name>_fm.wav` (the mixer's FM-mode output). Output: `report.txt` and `report.csv` in the output dir under `build/`. The tool never copies or writes input files elsewhere.

- [ ] **Step 1: Failing tests** in `host/crates/music/tests/corpus.rs`: `tzx_standard_converts` (a TZX built in the test from the pack's Castaway `.tap` blocks as 0x10 blocks, with a 0x30 text block, converts back byte-identical), `tzx_turbo_rejected` (a 0x11 block → not loadable), `corpus_runs_on_pack` (the tool's library entry point over a temp folder holding copies of the pack's `castawy.tap` and `games/test/tune.sna` produces two rows; Castaway's row has FM% > 50 and truth precision ≥ 90%).
- [ ] **Step 2: Run** `cd host && cargo test -p music --test corpus` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the host command — PASS. **Step 5: Commit** `git commit -m "music-corpus: measure FM music decoding over a local folder of games"`
- [ ] **Step 6 (controller):** run the tool with `--wav` on the owner's local folder of games (ask the owner for its path; known files: Cave Runner, Unison) and include the report in the final summary.

### Task 7: Docs and roll-up

**Files:**
- Modify: `docs/controls.md` (the Sound menu item), `docs/hardware-test.md` (FM checks on the device: Castaway's title and Colony 8 in FM, switching back and forth without clicks, effects staying on the beeper), `README.md` (feature line), `BUILDING.md` (re-run `fw_budget`/`fw_report` numbers with the new code), `docs/superpowers/specs/2026-11-13-fm-music-design.md` (status line: stage 1 delivered)

- [ ] **Step 1:** update the docs; **Step 2:** run host, firmware, `reuse lint`, `tools/web-demo --check --skip-firmware` — PASS; **Step 3: Commit** `git commit -m "Plan 5 roll-up: Sound menu docs, hardware checks, budgets"`.
- [ ] **Step 4 (controller):** headless Chrome: Castaway title in FM, toggle in the menu, no console errors; write the original/FM WAV pair from the page if practical, else from the corpus tool.
