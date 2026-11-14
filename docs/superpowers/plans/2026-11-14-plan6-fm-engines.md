<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# FM Music, Stage 2 (Plan 6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third Sound choice, "FM: engines" (method B), that recognises known music routines (Castaway, Colony 8, Cave Runner's two, Unison, the ROM's BEEP) wherever they sit in memory, reads their notes from memory and registers, checks them against the beeper's edges, and plays them as FM; anything else plays on the beeper.

**Architecture:** The core records, for every speaker edge, the address of the `OUT` that made it, and per frame a few `OUT` sites with the registers at their last `OUT`. A new `core/sound/engines.c` matches each site's surrounding bytes against a table of engine entries (digest at offsets from the `OUT`), reads the voices through the entry's reader, checks them against the frame's edges, and produces per-frame results with note events positioned inside the frame, delayed 3 frames like method A. The frame result gains an event list that the FM synth renders at its in-frame positions; method A emits none, so stage 1's output is unchanged. The mixer runs both methods every frame and plays the chosen one.

**Tech Stack:** C (core, firmware; JieLi clang in Docker via `tools/fm1-docker`), Rust host workspace, fm1-emu via emu-gate.

**Spec:** `docs/superpowers/specs/2026-11-13-fm-music-design.md`, §9 (stage 2, approved 2026-11-14); §1-§8 are stage 1 and still bind. Reference: the spike's `build/spike-beeper-fm/out/findings.txt` (§6 "approach 2": engine addresses and formulas; §8 Unison) and `host/crates/music/src/truth.rs` (the Rust note readers, which are the formulas in working form).

## Global Constraints

- Commit messages: a single line, no trailers (no Co-Authored-By). Never name projects other than this one and the games' upstreams in files or commits.
- Device rule: never flash an FM-1, send it SysEx or open MIDI ports. Everything runs on the host and in fm1-emu.
- `core/` C is portable, integer/fixed-point only (no float, no libm, no 64-bit division), no dynamic allocation, `-Wall -Wextra -Werror`, compiled into the host (`zx-sys` build.rs), the firmware (CMake) and the wasm build. Host and device output must match sample for sample.
- **Beeper mode is byte-identical to today; FM: sound (method A) output is byte-identical to today** (pinned by a golden hash in Task 2). Every existing test keeps passing unchanged, except where a signature change forces a mechanical call-site edit.
- The core change is observation only: emulation, timing, the beeper samples and every existing golden stay the same.
- The 60 ms delay (`SND_DELAY_FRAMES = 3`) is shared by every FM choice. In FM: engines a claimed frame enters FM at once (no `SND_ENTER_FRAMES` wait); the exit rule and crossfades are unchanged.
- B is pure: it never decodes pitch from the edges; it uses them only for the consistency check (spec 9.4) and for event positions. It never uses method A's decoder state.
- Confidence over coverage: a frame with any speaker edge from code outside every entry plays on the beeper; a frame failing the consistency check plays on the beeper. Zero FM on non-music sound, as stage 1's gates.
- Engine hold: a recognised engine counts as playing for 120 ms (6 frames) after its last `OUT`; those frames are B's confident silence unless other code drives the speaker.
- ROM BEEP: a BEEP whose whole length is ≥ 20 ms is a note; shorter ones stay on the beeper.
- Third-party code (Cave Runner, Unison) never enters the repository: only digests, offsets and formulas. Their files are read only by the local corpus tool and local runs; the owner's copies are in `/private/tmp/claude-501/-Users-orest-dev-projects-fm-1-zx/0522dac6-16c0-42de-8f3a-cc920c1a5675/scratchpad/corpus-owner/` (`Cave Runner.tzx`, `Cave Runner in game.tzx` + `.keys`, `Unison.z80`).
- Menu: Sound cycles Beeper → FM: sound → FM: engines → Beeper; default Beeper; holds across reset, picker and new games; not saved.
- Firmware budgets: every game's worst frame within `fw_budget`'s cap in every mode; RAMTEXT ≤ 49,152 bytes; audio interrupt unchanged.
- Commands:
  - host: `cd host && cargo test --workspace --exclude emu-gate`
  - device builds (C changes in `core/`, `app/`, `platform/`): `tools/fm1-docker cmake --build --preset fm1 && tools/fm1-docker cmake --build --preset fm1-preview`, then `FMZX_SKIP_DEVICE_BUILD=1 tools/make-packages --offline`
  - firmware: `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate`
  - licences: `reuse lint`; browser: `tools/web-demo --check --skip-firmware`

## Interfaces (fixed for all tasks)

```c
/* core/zx/zx.h */
typedef struct {
    uint32_t tstate;   /* frame T-state of the OUT's I/O cycle */
    uint8_t level;     /* (OUT 0xFE value >> 3) & 3: bit0 MIC, bit1 EAR */
    uint16_t pc;       /* NEW: address of the OUT instruction's first byte (opcode or ED prefix) */
} zx_edge;             /* still 8 bytes */

#define ZX_OUT_SITES 4u
typedef struct {
    uint16_t pc;       /* address of an OUT that changed the speaker level this frame */
    uint16_t count;    /* speaker edges it made this frame */
    zx_regs regs;      /* the registers at its last such OUT (pc field = the OUT's address) */
} zx_out_site;
/* The first ZX_OUT_SITES distinct OUT addresses that made speaker edges in the frame run last
 * (edges from further addresses still carry their pc). Same lifetime as zx_edges(). */
const zx_out_site *zx_out_sites(const zx_machine *m, uint32_t *count);

/* core/sound/sound.h */
enum snd_style { SND_STYLE_NONE, SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES, SND_STYLE_ENGINE };
enum snd_mode { SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B, SND_MODE_COUNT }; /* Plan 7 adds FM_C */

#define SND_EVENTS 8u
typedef struct {
    uint32_t t;           /* frame T-state, 0 .. ZX_FRAME_TSTATES - 1, ascending within a frame */
    uint32_t period_q8;   /* tonal: the voice's new period (0 = off); drum: ignored */
    uint8_t voice;        /* 0 .. SND_TONAL-1 tonal, SND_TONAL = the drum voice */
    uint8_t level;        /* tonal: loudness; drum: strength (0 = drum off) */
} snd_event;

typedef struct {
    snd_voice tone[SND_TONAL];   /* the state at the frame's start */
    uint8_t drum;
    uint8_t style;
    uint8_t confident;
    uint8_t engine;              /* NEW: 0 = none, else 1 + index in the engine table */
    uint8_t n_ev;                /* NEW: events applied in order after the frame's start; method A: 0 */
    snd_event ev[SND_EVENTS];
} snd_frame;

/* core/sound/engines.c */
typedef struct snd_engine snd_engine;    /* one table entry; fields the builder's (body in sound.h) */
extern const snd_engine snd_engine_table[];
extern const uint32_t snd_engine_count;
const char *snd_engine_name(const snd_engine *table, uint32_t index);   /* e.g. "Castaway", "ROM BEEP" */

typedef struct snd_b snd_b;              /* method B's state: matches, holds, result ring */
void snd_b_init(snd_b *b, const snd_engine *table, uint32_t n);
/* Feeds frame t (zm right after zx_run_frame; zm may be NULL: nothing is B's); fills *out with
 * the decision for frame t - SND_DELAY_FRAMES. */
void snd_b_frame(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out);

/* mixer (changed signature: zm added) */
void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_machine *zm, const zx_edge *e, uint32_t n_edges,
                     uint8_t start_level, const int16_t *beeper, uint32_t n, int16_t *out);
void snd_mixer_set_engines(snd_mixer *m, const snd_engine *table, uint32_t n);   /* tests; init uses the built-in table */

/* core/sound/engines_test.c: compiled by zx-sys build.rs only, never by the firmware CMake */
extern const snd_engine snd_engine_test_table[];   /* the built-in entries plus the "Test burst" fixture entry */
extern const uint32_t snd_engine_test_count;
```

Rust (`zx-sys`): `ZxEdge` gains `pc`; `Machine::out_sites() -> Vec<OutSite>`; `SndFrame` mirrors the new layout (with its layout check) and gains `SndEvent`; `SndMixer::frame(mode, m: Option<&Machine>, edges, start_level, beeper)`; `SndMixer::set_test_engines()`; new `SndEngines` (`new()` with the built-in table, `new_test()` with the test table, `frame(&mut self, m: &Machine, edges, start_level) -> SndFrame`, `name(engine: u8) -> String`).

Test fixtures (`host/crates/fmzx-pack/src/testprogs.rs`, generated, never added to `games/games.toml` or the pack):
- `burst_sna(base: u16) -> Vec<u8>`: a tiny engine assembled at `base` that plays a fixed melody as 9 ms square-wave bursts every 78 ms (4 frames minus 2 ms), with the note period self-modified into an operand of its loop and the melody in a table it reads through an absolute operand. Constants `BURST_HZ: [f64; N]`, `BURST_MS`, `BURST_GAP_MS`.
- `beep_tap() -> Vec<u8>`: a BASIC tape (autostarting as `tape.tap` does) of `BEEP` statements: a melody of at least 8 notes of 100-300 ms, then 5 `BEEP 0.008,n` blips. Constants `BEEP_NOTES: [(f64 secs, i8 semitone); N]`.

## Review Focus

1. **A recognised engine plus a sound effect in the same frame** (Cave Runner's jump over its tune): that frame plays on the beeper, the effect is heard, and FM resumes on the next clean frame without a click. Test in Task 3 (`mixed_frame_beeper`).
2. **A routine that matches but reads wrong** (a variant, or a formula wrong in an untested case): the consistency check keeps it on the beeper. Test in Task 3 (`edges_disagree_beeper`).
3. **Engine code that is reloaded or overwritten** (a game loading its next level over the old engine, a snapshot load, reset): a remembered match is dropped when the bytes change and on reset/load; no FM from stale matches. Test in Task 3 (`overwritten_engine_forgotten`) and Task 6 (reset/new game).
4. **BASIC programs that beep for input** (key clicks go through BEEPER, `PIP` = 0 makes very short beeps; a long `PIP` makes longer clicks): clicks under 20 ms never FM. Test in Task 4 (`short_beeps_stay_beeper`).
5. **Switching between the two FM choices mid-tune** (FM: sound → FM: engines in the menu): no 60 ms gap or repeat (the delay ring keeps running), only the FM/beeper crossfade. Test in Task 2 (`switch_between_fm_choices`).

---

### Task 1: The core records which code drives the speaker

**Files:**
- Modify: `core/zx/zx.h` (`zx_edge.pc`, `zx_out_site`, `ZX_OUT_SITES`, `zx_out_sites`, the site table in `zx_machine`), `core/zx/ula.c` (fill them in `zx_port_out`, reset per frame with the edges, pending edges carry their pc and sites into the next frame), the place that knows the current instruction's address (`core/zx/timing.c` or `machine.c`: record the PC at each instruction's first fetch, prefixes included)
- Modify: `host/crates/zx-sys/src/lib.rs` (`ZxEdge.pc`, `OutSite`, `Machine::out_sites`)
- Test: `host/crates/zx-tests/tests/out_sites.rs` (new)

**Interfaces:** Produces `zx_edge.pc`, `zx_out_site`, `zx_out_sites()` exactly as in the Interfaces block.

- [ ] **Step 1: Failing tests** in `out_sites.rs`:
  - `edge_pc_is_an_out`: for 300 frames each of `tune.sna`, `border.sna` and Castaway's tape (title, from frame 400), every edge's `pc` addresses an `OUT` instruction: `peek(pc) == 0xD3`, or `peek(pc) == 0xED` and `peek(pc+1) & 0xC7 == 0x41`.
  - `tune_one_site`: during `tune.sna`'s melody every frame has exactly one site, its `count` equals the frame's edge count, and its `regs.pc` equals its `pc`.
  - `site_regs_at_the_out`: in `tune.sna`, the site's `regs` equal a register snapshot taken when the CPU was at that `OUT` (check: `regs.sp` and `regs.af`'s A, whose bits 3-4 give the level of the site's last edge).
  - `sites_capped`: a generated memory image with 6 `OUT (0xFE),A` instructions at different addresses each flipping the speaker in one frame gives 4 sites, and all 6 edges still carry their own `pc`.
  - `beeper_unchanged`: the beeper samples of 600 frames of Castaway's title and 300 of `border.sna` hash to the same values as at BASE (compute the hashes at BASE in Step 1 and write them into the test).
- [ ] **Step 2: Run** `cd host && cargo test -p zx-tests --test out_sites` — FAIL.
- [ ] **Step 3: Implement.** Only speaker-level changes create edges and sites (an `OUT` that changes only the border is neither). Pending (frame-overshooting) edges keep their pc and count toward the next frame's sites.
- [ ] **Step 4: Run** the host command — PASS, all existing tests unchanged. Run the device builds (the core is in RAMTEXT): they build.
- [ ] **Step 5: Commit** `git commit -m "Core: each speaker edge carries the address of its OUT; per-frame OUT sites with their registers"`

### Task 2: Note events, the FM: engines mode and the mixer plumbing

**Files:**
- Modify: `core/sound/sound.h` (Interfaces block: `snd_event`, `snd_frame` fields, `SND_STYLE_ENGINE`, `SND_MODE_FM_B`, `snd_engine`/`snd_b` bodies, mixer signature and `snd_mixer_set_engines`), `core/sound/fm.c` (render events at their positions), `core/sound/mixer.c` (run A and B every frame; play the chosen mode's result; FM_B enters on one claimed frame), `core/sound/decoder.c` (set `engine = 0`, `n_ev = 0`)
- Create: `core/sound/engines.c` (stub: `snd_b_frame` always "not mine", `snd_engine_table` empty, `snd_engine_name`), `core/sound/engines_test.c` (empty test table for now)
- Modify: `core/sound/CMakeLists.txt` (engines.c; not engines_test.c), `host/crates/zx-sys/build.rs` (both), `host/crates/zx-sys/src/lib.rs` (wrappers in the Interfaces block)
- Modify: `app/menu.c` (`sound_names` gets "FM: engines"), `platform/fm1/main.c` (pass `m` to `snd_mixer_frame`), every Rust caller of `SndMixer::frame` (add `None` or the machine)
- Test: `host/crates/music/tests/fm.rs`, `host/crates/music/tests/mixer.rs`

**Interfaces:** Consumes Task 1's `zx_edge.pc`. Produces everything in the Interfaces block under `sound.h`, the stub `snd_b`, and the Rust wrappers.

- [ ] **Step 1: Failing tests** (compute the A golden at BASE first):
  - `fm_a_unchanged` (mixer.rs): the `SND_MODE_FM_A` output of 600 frames of Castaway's title and 600 of Colony 8's demo hashes to the value computed at BASE.
  - `event_starts_mid_frame` (fm.rs): a frame with all voices off and one event at `t = ZX_FRAME_TSTATES / 2` turning voice 0 on at 440 Hz: the samples before `n / 2 - 2` are silent, those after `n / 2 + 2` are not, and the second half's pitch is 440 Hz within 0.5% (render 5 frames, the event in the first, the voice held after).
  - `event_stops_mid_frame` (fm.rs): a voice on at the frame start, an event turning it off at `t = 9 ms` (31,500 T): its envelope is in release from that sample (amplitude falls monotonically after it and reaches 0 within the release time).
  - `two_notes_in_one_frame` (fm.rs): voice 0 at 440 Hz, an event at mid-frame changing it to 660 Hz: zero-crossing pitch of each half within 1%.
  - `fm_b_stub_is_delayed_beeper` (mixer.rs): in `SND_MODE_FM_B` with the stub, the output equals FM_A's never-confident output (frame t = beeper frame t-3).
  - `switch_between_fm_choices` (mixer.rs): switching FM_A → FM_B → FM_A mid-Castaway produces no block of silence and no repeated 60 ms block (the delayed beeper stream continues sample for sample).
- [ ] **Step 2: Run** `cd host && cargo test -p music` — FAIL.
- [ ] **Step 3: Implement.** Event sample position = `t * n / ZX_FRAME_TSTATES` (32-bit safe: `t < 69,888`, `n ≤ 1024`). A tonal event behaves as a frame whose voice changed at that sample (attack on a new period, release on 0); a drum event as the drum on/off. The mixer keeps one hysteresis; FM_B enters after 1 claimed frame. `snd_mixer_init` uses the built-in table; `snd_mixer_set_engines` re-inits B with another table.
- [ ] **Step 4: Run** the host command and the device builds — PASS; `fw_audio`/`fw_controls` still pass (menu now has three choices; any test asserting exactly two choices is updated to the new cycle, nothing else).
- [ ] **Step 5: Commit** `git commit -m "Sound: in-frame note events, the FM: engines mode, the mixer runs both methods"`

### Task 3: Method B: matching, reading, the check, hold and mixed frames (Castaway, Colony 8)

**Files:**
- Modify: `core/sound/engines.c`, `core/sound/sound.h` (`snd_engine`, `snd_b` bodies), `core/sound/engines_test.c` (the "Test burst" entry for `burst_sna`)
- Modify: `host/crates/fmzx-pack/src/testprogs.rs` (`burst_sna`, its constants; not in `PROGRAMS`)
- Test: `host/crates/music/tests/engines.rs` (new)

**Interfaces:** Consumes Task 1's sites, Task 2's frame/event types and stub. Produces the working `snd_b_frame` and the entries "Castaway" and "Colony 8" (the table's first two); Tasks 4-5 add entries through the same `snd_engine` format.

An entry (fields the builder's) holds: one or more **sites** (each an `OUT` the engine drives the speaker from: signature offsets and length around the `OUT`, wildcard offsets for self-modified bytes, FNV-1a 64 digest of the rest, role tonal or drum); the **reader** (a function per entry is fine: given the machine, the matched site's address and its `zx_out_site` registers, it returns up to `SND_TONAL` voices and their channels), and the **formula** in the reader. Every address a reader uses is either relative to the matched site or read from an operand inside the matched code (spec 9.3). Matches are remembered per site address while a re-digest still matches (re-check a remembered match at least every frame it is used — cheap); `snd_b_init` and a frame where the site's bytes no longer match drop it.

Per frame: (1) if any edge's `pc` is not inside a matched (or newly matched) site's signature span → not B's (beeper), engine hold ends; (2) else read the voices (tonal sites) or drum (drum sites); (3) consistency check (spec 9.4, form the builder's; it must reject `edges_disagree_beeper` and accept every pack-music frame the gates need); (4) events: a note change at the first edge after it, an off after an engine's last edge plus one half-period when the engine stops in the frame; (5) a frame with no edges within 6 frames after the engine's last edge, with no other code's edges, is confident silence (voices released through events/off state); (6) results go into a ring and come out `SND_DELAY_FRAMES` later, `style = SND_STYLE_ENGINE`, `engine = 1 + index`.

Readers (from `truth.rs` and the spike's findings §6, ported to relative addressing):
- Castaway: two channels, N at the loop's operands (C86Dh/C87Ah in the pack), masks C875h/C885h, wrap C871h/C881h; f = 3.5 MHz / ((wrap+1) × N × 156.6). Level from the mask (0 = rest).
- Colony 8: three channels, steps from the site registers (BC/DE/SP as the engine uses them at that `OUT`; mind the `EXX` swap), duty thresholds 9500h/950Ah/9512h (0 = silent); f = step × 3.5 MHz / (155.9 × 65536). Drum sites: 9496h (tone burst) and 94ADh (ROM noise) → drum events.

- [ ] **Step 1: Failing tests** in `engines.rs`:
  - `castaway_matches_truth`: Castaway's title, 1,500 frames, built-in table: on frames where `truth::castaway` reads notes, B's voices at the frame's end (state + events) equal truth's within 0.1 semitone on ≥ 99% of them; B's `engine` names "Castaway".
  - `colony8_matches_truth`: the same for Colony 8's music (the run `tests/slots.rs` uses), ≥ 99%; drum frames report a drum.
  - `agrees_with_a`: Castaway and Colony 8 runs feeding both `SndDecoder` and `SndEngines`: on frames where A is confident, B is confident and every voice agrees within 0.5 semitone on ≥ 95% of them, and no frame has a B voice A contradicts by more than 0.5 semitone except within 2 frames of a note change.
  - `moved_engine`: `burst_sna(0x8000)` and `burst_sna(0xB123)` with the test table give the same voice sequence, matching `BURST_HZ` within 0.5%.
  - `burst_holds_through_gaps`: `burst_sna` through `SndMixer` in FM_B with test engines: after the first burst, `fm_on()` stays true through every gap for the whole melody; each burst's FM note starts within 1 ms of its first edge.
  - `mixed_frame_beeper`: `burst_sna` with one extra `OUT` from code outside the engine in one gap frame and one burst frame: those two frames are not confident; the frames after them are.
  - `edges_disagree_beeper`: frames of the Castaway run whose edge lists are replaced by a regular square wave at a pitch 3 semitones off B's read voice (same `pc`s) are not confident.
  - `overwritten_engine_forgotten`: run `burst_sna`, then overwrite its loop bytes with `NOP`s and drive the speaker from them (same addresses): no confident frame after the overwrite.
  - `effects_not_b`: the pack's effects demos (`no_false_fm.rs`'s `effects_games_demo` runs: Sopwith, Dino, One-Way, Colony 8 in play, Castaway in play) and `border.sna` 60 s and `noise.sna` give zero confident B frames outside music the truth readers see.
- [ ] **Step 2: Run** `cd host && cargo test -p music --test engines` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the host command — PASS. Run the device builds.
- [ ] **Step 5: Commit** `git commit -m "Method B: engine matching by OUT site, Castaway and Colony 8 readers, the check against the edges, hold and mixed frames"`

### Task 4: The ROM BEEP entry

**Files:**
- Modify: `core/sound/engines.c` (entry "ROM BEEP"), `host/crates/fmzx-pack/src/testprogs.rs` (`beep_tap`, `BEEP_NOTES`; not in `PROGRAMS`)
- Test: `host/crates/music/tests/engines.rs`

**Interfaces:** Consumes Task 3's entry format. Produces the "ROM BEEP" entry.

The site is BEEPER's own `OUT (0xFE),A` inside 03B5h-03F7h of the 48K ROM (find its address from the ROM bytes in the repository; the signature digests the ROM's bytes so another ROM does not match). Pitch from the site registers H, L (already shifted by the routine) and IX (which encodes L's low bits as the entry point into the NOP slide); derive the half-period in T-states from the loop's instruction timings and check it against measured edges. Whole length = remaining DE × period at the BEEP's first `OUT` (a new BEEP: DE jumps up or the pitch changes); under 20 ms → not B's for that BEEP's frames.

- [ ] **Step 1: Failing tests:**
  - `beep_pitch_exact`: `beep_tap()` run through `SndEngines`: each melody note's read period equals the period measured from that note's edges (mean edge spacing × 2) within 0.2%, and its frequency matches `440 × 2^(semitone/12)` within the ROM's own rounding (0.5%).
  - `beep_notes_in_fm`: every melody note is confident, starts within 1 ms of its first edge and ends within 2 ms of its last edge plus a half-period (from the events); between notes (the interpreter's gaps) voices are off.
  - `short_beeps_stay_beeper`: the five `BEEP 0.008` blips are never confident; after the program ends (report "0 OK"), pressing a letter key for 5 frames makes the ROM's key click (BEEPER with the default `PIP`): never confident.
  - `beep_nothing_from_other_rom`: the same run with one byte of the BEEPER routine's signature changed in a copied ROM (host test builds a machine with that ROM): no B frame.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** the host command — PASS; device builds.
- [ ] **Step 5: Commit** `git commit -m "Method B: the ROM BEEP entry, with a generated BASIC BEEP test tape"`

### Task 5: Cave Runner and Unison entries, the corpus report

**Files:**
- Modify: `core/sound/engines.c` (entries "Cave Runner title", "Cave Runner in game", "Unison" with its four drum sites), `host/crates/music/src/truth.rs` (Unison reader), `host/crates/music/src/corpus.rs` and `src/bin/music-corpus.rs` (B columns), `host/crates/music/tests/corpus.rs`
- Never commit: any byte of the owner's files; digests, offsets and formulas only.

**Interfaces:** Consumes Task 3's format. The corpus report gains, per game, B's FM share (through the mixer's FM_B path, as A's is computed), the engines B recognised (names), and with a truth reader B's frame precision/recall and wrong notes, beside A's existing columns; `--wav` also writes `<name>_<ext>_fm_b.wav`.

Readers (spike findings §6 and §8, `truth.rs`):
- Cave Runner title: loop 92F8h-930Dh, IY → [duration, N1, N2] (the site registers' IY), f = 3.5 MHz / (2 × N × 56.6), two channels.
- Cave Runner in game: burst loop 8852h, note index at 845Bh into the table at 858Ch (read both addresses from the operands of the instructions that use them), f = 3.5 MHz / (2 × E × 40); flag 845Ch bit 1 = music off.
- Unison: loop 81C6h (81B4h-81E9h), N at 81CDh and N-1 at 81DFh (one voice doubled, one note per frame); drum sites at the `OUT`s inside 8257h, 826Ah, 827Eh, 8292h → drum events.
Signatures and digests come from the owner's local copies (path in Global Constraints), computed by a local, uncommitted run.

- [ ] **Step 1: Failing tests** (committed, no third-party bytes):
  - `corpus_reports_b` (corpus.rs): `corpus_runs_on_pack`'s run reports B columns for Castaway and Colony 8 with the engine names and B wrong notes 0.
  - `unison_truth_digest`: Unison joins the truth `SIGS` with its loop digest (a unit test that an unknown memory image is not recognised, as `engine_not_taken_from_the_name` does).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.**
- [ ] **Step 4: Run** the host command — PASS. Then the local corpus run (not committed): `cd host && cargo run --release -p music --bin music-corpus -- <owner folder> --wav`. Record in the report file: per game A and B FM share, B's engines, B precision/recall/wrong notes against truth. Expected: Cave Runner title and in game and Unison recognised; B wrong notes 0 on each; B's FM share on Cave Runner in game above A's (82.0%) and Unison above A's (60.0%). A miss here is fixed in this task, not deferred.
- [ ] **Step 5: Commit** `git commit -m "Method B: Cave Runner and Unison entries (digests only), the corpus report shows B beside A"`

### Task 6: The firmware: three Sound choices, FM: engines on the device path

**Files:**
- Modify: `host/crates/emu-gate/tests/fw_audio.rs`, `fw_controls.rs`, `fw_budget.rs`; `host/crates/emu-gate/tests/golden/menu*.png` only if the menu image changes (it should not: the label shows only the current choice)
- Modify (if needed): `platform/fm1/main.c`, `core/sound/CMakeLists.txt` (engines.c in RAMTEXT as the rest of the sound code)

**Interfaces:** Consumes Tasks 2-5 as built.

- [ ] **Step 1: Failing tests:**
  - `sound_menu_cycles_three` (fw_audio): from HOME, Enter on Sound three times gives Beeper → FM: sound → FM: engines → Beeper (screen text or the menu's reported choice, as `sound_menu_switches_to_fm` checks).
  - `fm_engines_castaway_title` (fw_audio): with FM: engines chosen, Castaway's title audio captured from fm1-emu shows the read pitches (truth's notes) in the zero-crossing windows `sound_menu_switches_to_fm` uses for FM: sound, and no underruns.
  - `sound_choice_survives_reset_and_new_game` (fw_controls): extended to FM: engines.
  - `fw_budget`: every game's worst frame and RAMTEXT within the caps with FM: engines active (add the mode to the existing loop over modes); noise.sna included.
- [ ] **Step 2: Run** the device builds, repack, then the firmware command — FAIL on the new tests.
- [ ] **Step 3: Implement** whatever the firmware needs (expected: nothing beyond Task 2's call change; RAMTEXT placement of engines.c).
- [ ] **Step 4: Run** the firmware command — all PASS. Record the worst frames per mode, RAMTEXT and audio ISR numbers in the report.
- [ ] **Step 5: Commit** `git commit -m "Firmware: the FM: engines Sound choice, tests and budgets"`

### Task 7: Docs and roll-up

**Files:** `docs/controls.md` (Sound choices), `docs/hardware-test.md` (an FM: engines check on Castaway's title and a BASIC `BEEP`), `README.md` (one line on FM: engines), `BUILDING.md` (budgets per mode; the corpus report's B columns and `_fm_b.wav`), the spec's status line ("stage 2 delivered (Plan 6)"), `LICENSING.md`/`REUSE.toml` only if new paths need it.

- [ ] **Step 1:** update the docs. **Step 2:** run host, firmware, `reuse lint`, `tools/web-demo --check --skip-firmware` — PASS. **Step 3: Commit** `git commit -m "Plan 6 roll-up: FM: engines docs, hardware checks, budgets"`.
