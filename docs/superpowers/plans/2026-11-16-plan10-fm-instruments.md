# FM Instruments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** each FM voice plays a bass, lead or pad instrument (the owner's "D, chip-plus" set) by its role in the tune, decided live on the FM-1 and, where known, stored in the personal pack.

**Architecture:** the sound start-up code moves to flash to make room; the 2-operator synth (`core/sound/fm.c`) gains a patch per role, a modulator ratio, vibrato, a detuned second carrier and a new drum, and latches a voice's role at each note start; a live rule in the mixer ranks voices by pitch; method B passes stored roles from learned entries (`instrument[]`) and from new kind-1 records for built-in entries; the personal pack builder analyses roles and reads overrides from `personal-games.toml`.

**Tech Stack:** C (core/sound, platform/fm1; pi32v2 clang for the device, host cc via zx-sys), Rust host crates (zx-sys, music, soundlearn, fmzx-pack, emu-gate).

**Spec:** `docs/superpowers/specs/2026-11-16-fm-instruments-design.md` (read it first; it is the authority).

## Global Constraints

- No decoder decision may change: method A's and method B's per-frame decisions (confident, engine, voices, events, drum) are identical before and after, on the music corpus fixtures and the owner's folder. Only how voices sound changes.
- A voice's instrument changes only when a note starts on it, never mid-note.
- RAMTEXT ends at most 48,864 bytes (`tools/fm1-docker cmake --build --preset fm1` prints it); `check_reach.py` passes; the worst frame stays under 3,600,000 instructions (emu-gate `fw_budget`).
- Loudness: over the corpus, the FM output's peak and RMS stay within 1 dB of today's, measured on the mix (whole tunes); the mix saturates as today, no new clipping.
- Device audio stays sample-exact against the host's C (emu-gate `fw_audio`, `fw_personal`).
- The D patch values are exactly the spec's table (step 1b); roles: 0 none/auto, 1 bass, 2 lead, 3 pad.
- No pack version bump (pack and firmware ship together).
- The owner's games never enter the repository (digests, offsets, formulas only; no game names in committed code, tests or docs). Never open Music.zip. Never flash, SysEx or open MIDI. Never name the reference project. Commit messages: one line, no trailers.
- Inner loop is host-only (`cd host && cargo test --release --workspace --exclude emu-gate`, `music-corpus`); the emu-gate suite runs once, at the end.

## Review Focus

1. A tune whose bass climbs above its lead for a phrase: the roles don't swap (live rule hysteresis) — test in Task 3.
2. A voice that rests for longer than the window and comes back: it keeps its role — test in Task 3.
3. A frame where a routine with stored roles claims only some voices (or role 0 on a channel): the others fall back to the live rule — test in Task 4.
4. A kind-1 record whose digest names no built-in entry, an `instrument` byte above 3, an override naming an unknown OUT address or role: ignored / treated as 0 / a reported error — tests in Tasks 4 and 5.
5. Switching Sound mode while a tune plays (the synth is reset at an FM turn-on): no click, roles restart from the live rule — test in Task 3.

---

### Task 1: Room in fast memory (sound start-up code to flash)

**Files:**
- Create: `core/sound/sound_init.c`
- Modify: `core/sound/mixer.c`, `core/sound/fm.c`, `core/sound/engines.c`, `core/sound/decoder.c`, `core/sound/sound.h`, `core/CMakeLists.txt`, `platform/fm1/CMakeLists.txt`, `platform/fm1/main.c`, `platform/fm1/ramcalls.h`
- Test: existing suites (behaviour must be byte-identical)

**Interfaces:**
- Produces: `snd_mixer_init`, `snd_mixer_set_engines`, `snd_fm_init`, `snd_b_init`, `snd_decoder_init` defined in `core/sound/sound_init.c`, same signatures as today. On the device they are in a library `zx_sound_init` (its archive name does not match `app.ld`'s `*libzx_sound.a` pattern, so it links into XIP `.text`); `main.c` calls `snd_mixer_init` and `snd_mixer_set_engines` directly (drop `mixer_init` and `set_engines` from `struct ram_calls`).
- The FM turn-on reset inside `snd_mixer_frame` (today a call of `snd_fm_init`, mixer.c ~line 135) must not call flash code from RAMTEXT: split out a RAMTEXT `void snd_fm_reset(snd_fm *f)` (voices and drum to rest, rate and `kfreq` kept) and call that; `snd_fm_init` computes `kfreq` then calls the reset logic of its own (no cross call: duplicate the few lines or keep the reset loop in both).
- Helpers the moved functions use (e.g. `udiv64` in fm.c) get a copy in `sound_init.c` (static), so nothing in flash calls RAMTEXT directly.

- [ ] **Step 1:** Move the five functions; adjust `core/CMakeLists.txt` (exclude `sound_init.c` from `ZX_SOUND_SOURCES`, `add_library(zx_sound_init STATIC sound/sound_init.c)` linking `zx_sound`, same warnings) and link `zx_sound_init` wherever `zx_sound` is linked and an init function is called (firmware, bench). zx-sys's `build.rs` compiles every `core/sound/*.c` already; check it does.
- [ ] **Step 2:** `cd host && cargo test --release --workspace --exclude emu-gate` — all 445 pass with no pinned hash changed.
- [ ] **Step 3:** `tools/fm1-docker cmake --build --preset fm1 && tools/fm1-docker cmake --build --preset fm1-preview` — both link, `check_reach.py` passes, RAMTEXT ≤ 47,964 (at least 900 below 48,864). Record the number in the report.
- [ ] **Step 4:** Commit `Sound: the start-up functions run from flash (RAMTEXT -N bytes)`.

### Task 2: The D instruments in the synth

**Files:**
- Modify: `core/sound/fm.c`, `core/sound/sound.h`, `host/crates/zx-sys/src/lib.rs` (expose what tests need)
- Test: `host/crates/music/tests/instruments.rs` (create); re-pin `host/crates/music/tests/mixer.rs` and corpus hashes

**Interfaces:**
- Produces in `sound.h`: `enum snd_role { SND_ROLE_AUTO, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD };` and `uint8_t role[SND_TONAL]` in `snd_frame` (the role of each tonal voice for this frame; `SND_ROLE_AUTO` renders as lead until Task 3 fills it). `snd_fm_voice` gains the latched role and the new oscillator state.
- Semantics: a voice takes `fr->role[k]` only when a note (re)starts on it (the existing restart rule: voice starting, or period moving by more than 1/32); a glide or a held note keeps its latched role. A note event in the frame uses the frame's role for its voice.
- Patch values: the spec's table (bass/lead/pad: ratio 1/2/1; index 0.7→0.5 100 ms / 1.6→1.3 200 ms / 0.4→0.3 200 ms; attack 3/4/10 ms; decay τ 200/150/200 ms to 0.8/0.8/0.85; release 50/50/100 ms; lead vibrato 18 cents, 6.5 Hz, 120 ms delay, 200 ms ramp; pad second carrier +4 cents, half and half; gains 1.2/0.8/0.75). Drum: one-pole low-passed noise (coefficient 0.5), τ 70 ms, weight 0.8, plus a 190 Hz sine, τ 30 ms, weight 0.4; strength from the drum level; release as today. Overall gain calibrated per the loudness constraint.

- [ ] **Step 1: Write the failing tests** in `instruments.rs`, rendering one held note (e.g. 440 Hz, level 255) per role through `SndFm` for 1 s at 44,100 Hz:
  - `lead_has_strong_second_harmonic_and_vibrato`: lead's spectrum energy at 2f and 3f relative to f matches the 2:1-ratio FM with index 1.3 (settled) within 2 dB of a float reference computed in the test; the instantaneous frequency (zero crossings) after 400 ms swings ±18 cents ±3 at 6.5 Hz ±0.3; before 120 ms it does not swing.
  - `pad_beats_at_detune`: the pad's amplitude envelope beats at f × (2^(4/1200) − 1) Hz ±10%.
  - `envelopes_match_table`: per role, time to peak = attack ±0.5 ms; level at attack + τ_decay within 3% of sus + (1 − sus)/e; after a stop, silence after the release time ±1 ms.
  - `role_latched_at_note_start`: a note started as bass whose frames then say lead keeps the bass patch until its next note start; a glide (period change < 1/32) keeps it too.
  - `drum_has_tone_and_noise`: the drum's spectrum has a peak at 190 Hz ±5 and broadband noise; it decays per τ.
- [ ] **Step 2:** Run `cargo test --release -p music --test instruments` — FAIL.
- [ ] **Step 3:** Implement in `fm.c` (patch table as `static const` data; the modulator phase at `ratio × inc`; vibrato on the 8 kHz tick; pad's second carrier; the drum). Keep the integer/fixed-point style and the existing comment header updated.
- [ ] **Step 4:** Loudness: add `loudness_matches_today` to `instruments.rs` — renders the repo's tune fixtures (`games/test/tune.sna`, Castaway, Colony 8 from `games/`) through `SndMixer` in FM: engines and FM: sound with the new synth and compares peak and RMS against values measured from the current main build (record those constants in the test with a comment saying where they came from): within 1 dB, no sample at ±32767 that wasn't before. Calibrate the overall gain until it passes.
- [ ] **Step 5:** Decisions unchanged: add `decisions_unchanged` (music crate) — for the repo's corpus fixtures, method A's and B's per-frame decisions (the `snd_frame` minus `role`) hash to the same values as before this task (pin the pre-change hashes, taken from main, in the test).
- [ ] **Step 6:** Re-pin the mixer/corpus output hashes that change because the sound changed (only output-sample hashes; any decision hash changing is a bug). `cargo test --release --workspace --exclude emu-gate` all pass.
- [ ] **Step 7:** Device builds; RAMTEXT ≤ 48,864 (if not: the spec's order — more cold code to flash first, then cut the pad's second carrier, then the drum's tone; report any cut). Commit `Sound: the D instruments (bass, lead, pad, drum) in the FM synth, a voice's role latched at its note start`.

### Task 3: The live rule

**Files:**
- Create: `core/sound/roles.c`
- Modify: `core/sound/sound.h`, `core/sound/mixer.c`, `host/crates/zx-sys/src/lib.rs`
- Test: `host/crates/music/tests/roles.rs` (create)

**Interfaces:**
- Produces: `typedef struct snd_roles {...} snd_roles;` (per slot: a slow average of log-pitch while sounding, time last sounded, current role), `void snd_roles_init(snd_roles *r)`, `void snd_roles_frame(snd_roles *r, snd_frame *fr)` — fills every `fr->role[k]` that is `SND_ROLE_AUTO` from the ranking (lowest average bass, highest lead, a third pad; two voices bass and lead; a voice alone lead); tracks every sounding voice's pitch whatever decided its role. `snd_mixer` owns one (`m->roles`), initialised in `snd_mixer_init` and at an FM turn-on, and calls `snd_roles_frame` on each frame before `snd_fm_render`. `snd_roles_init` lives in `sound_init.c` (flash) on the device; `snd_roles_frame` in RAMTEXT.
- Starting points (tune on the corpus, record the final values in the code comments): averaging time constant 2 s; a ranking change needs a margin of 3 semitones; "recently sounded" window 4 s (a voice silent longer leaves the ranking, its history kept); a voice's first notes, before any history, ranked by their own pitch.

- [ ] **Step 1: Write the failing tests** in `roles.rs` (synthetic `snd_frame` sequences through `snd_roles_frame`):
  - `three_voices_ranked`: voices at 110, 440, 880 Hz → bass, pad, lead.
  - `two_voices_bass_and_lead`, `lone_voice_is_lead`.
  - `crossing_bass_keeps_role` (Review Focus 1): the bass climbs above the lead for 1 s → roles unchanged.
  - `rest_longer_than_window_keeps_role` (Review Focus 2): the bass rests 6 s then returns → bass.
  - `stored_role_respected`: a frame with `role[1] = SND_ROLE_PAD` keeps it; the others get ranked.
  - `fm_turn_on_restarts_roles` (Review Focus 5): through `SndMixer`, switching Beeper → FM: engines mid-tune: roles come from the live rule afresh and the output has no step larger than the existing crossfade allows (reuse the mixer tests' click measure).
  - `corpus_roles_stable`: on the repo's Colony 8 run in FM: engines, roles never change mid-note and settle within the first 2 s to bass/pad/lead on its three channels (the listening test's ranking: low→high channels 0, 2, 1).
- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run — PASS; full host suite passes (re-pin output hashes only).
- [ ] **Step 5:** Device builds; RAMTEXT ≤ 48,864. Commit `Sound: the live rule gives each FM voice its role by pitch`.

### Task 4: Stored roles in the firmware decoder

**Files:**
- Modify: `core/sound/sound.h`, `core/sound/learned.c`, `core/sound/engines.c`, `core/sound/engines_builtin.h`, `platform/fm1/main.c`, `host/crates/zx-sys/src/lib.rs`, `docs/pack-format.md`
- Test: `host/crates/music/tests/learned.rs`

**Interfaces:**
- `snd_engine` gains `uint8_t role[SND_TONAL]` (0 for every built-in initialiser).
- `snd_learned_decode` sets a learned entry's `role[c]` from `instrument[c]` (values above 3 → 0), and accepts list entries of **kind 1** — `u16 size, u8 kind = 1, u8 channels, u64 digest, u8 role[3]`, zero padding to a multiple of 4 (size ≥ 16) — producing no engine; kind-1 entries count towards `SND_LEARNED_MAX` and `SND_LEARNED_WORK_MAX` bounds as size-checked entries (no sites, no work).
- New: `void snd_learned_roles(const uint8_t *list, uint32_t avail, snd_engine *table, uint32_t n)` — for each kind-1 record of a list the decoder accepted, sets `role[]` on the table entry whose `digest` equals the record's (only a non-learned entry, i.e. one whose `read` is not the learned reader); no match: ignored. Runs from flash on the device (in `learned.c`).
- B: when a routine claims a frame, `decide` writes `out->role[k] = en->role[k]` for k < channels (others `SND_ROLE_AUTO`); the mixer's live rule (Task 3) fills the rest.
- `main.c` `learned_start`, zx-sys `SndEngines::with_learned` and `SndMixer::set_learned` call `snd_learned_roles` on the table after it is assembled (the built-ins copied first).

- [ ] **Step 1: Write the failing tests** in `learned.rs`: `instrument_bytes_become_roles` (a learned entry with instrument [1,2,0] → B's claimed frames carry roles [bass, lead, auto]); `instrument_above_3_is_auto`; `kind1_sets_builtin_roles` (the burst test engine's digest via `new_test` table, or a built-in's digest, gets roles through a kind-1 record); `kind1_unknown_digest_ignored`; `kind1_counts_towards_bounds` (9 entries of which some kind-1 → rejected as today's 9-entry list); `partial_stored_roles_fall_back` (Review Focus 3: role 0 channels and unclaimed frames get the live rule's roles, through `SndMixer`).
- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement; document kind 1 and `instrument[]` in `docs/pack-format.md` (no version change; say why). **Step 4:** PASS; full host suite.
- [ ] **Step 5:** Device builds; RAMTEXT ≤ 48,864. Commit `Learned sound: stored roles from instrument bytes and kind-1 records for built-in entries`.

### Task 5: Roles analysed at pack build, overrides, report

**Files:**
- Modify: `host/crates/soundlearn/src/entry.rs` (encode `instrument[]`, kind 1), `host/crates/soundlearn/src/learn.rs` and/or `report.rs`, `host/crates/fmzx-pack/src/personal.rs`, `host/crates/fmzx-pack/src/manifest.rs` (the `[game.instruments]` table), `BUILDING.md` ("The music": overrides), `docs/controls.md` (Sound row: instruments)
- Test: `host/crates/soundlearn/tests/entry.rs`, `host/crates/fmzx-pack/tests/personal_learn.rs`

**Interfaces:**
- Analysis: during the learner's existing 3-minute run (no extra emulation), per routine that claims frames (learned or built-in), per channel, the mean of log-pitch over the frames it claims and sounds; rank → roles as the live rule (lowest bass, highest lead, a third pad, lone lead). A routine with fewer claimed frames than the music gate's minimum gets none. Learned → `instrument[]`; built-in → one kind-1 record (its `digest`, `channels`, roles).
- Overrides: `[game.instruments]` maps an OUT address (hex, 4 digits, as `sound-report.txt` names the routine's OUT) to a list of role names (`"bass"`, `"lead"`, `"pad"`) by channel. Override wins. An address that names no routine of that game or a role name not in the list: the builder fails with the game's name and the bad key (no pack written).
- `sound-report.txt`: per routine, each channel's role and its source (`analysis`, `override`); the cache key includes the overrides (bump `LEARNER_VERSION` to 5).

- [ ] **Step 1: Write the failing tests:** `entry.rs`: `encodes_instrument_bytes`, `encodes_kind1_record` (bytes as the format says, round-trip through `SndEngines::with_learned` → B's frames carry the roles); `personal_learn.rs`: `tune_gets_lead_role` (the repo's tune: one channel → lead stored), `override_wins_and_is_reported`, `bad_override_fails_with_game_name` (Review Focus 4).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS; full host suite; `music-corpus` on `~/Downloads/zx` runs and its decisions columns (`FM %`, `B FM %`, `L FM %`, truth) are unchanged from main.
- [ ] **Step 5:** Commit `Personal pack: each routine's channel roles analysed and stored, overrides in personal-games.toml`.

### Final (controller, after the final review)

- [ ] The emu-gate suite once: `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release --no-fail-fast -p emu-gate` after `tools/make-packages --offline` — all pass (sample-exact audio, budget under 3.6M, RAMTEXT ≤ 48,864).
- [ ] Listening page from the real C synth: `music-corpus --wav` on the owner's folder from main and from the branch, the same four tunes (Cave Runner, Star Pilot, Colony 8, Night Walk) side by side in `build/web/listen/` (today vs new); the owner confirms before merging.
- [ ] Merge to main and push (owner's standing workflow), rebuild the personal pack and the test page.
