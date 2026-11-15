<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Plan 9: Learned Sound Entries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `make-personal-pack` learns each game's music routines unattended and stores them in the
pack. The FM-1's FM: engines plays them as FM, checked against the speaker exactly like the
built-in entries.

**Architecture:**
- **New host crate `soundlearn`**, called by `fmzx-pack personal`. Per game it:
  - listens to the game through a host-only trace hook in the core;
  - reads the code of every speaker routine with a new Z80 decoder;
  - derives and fits period formulas;
  - gates the result by replaying the game through the real method B;
  - encodes the learned entries as data.
- **On the device:** the pack gains a sound section (pack version 2). One generic reader
  (`core/sound/learned.c`, run from flash) interprets each entry's small bytecode programs, and
  method B's table becomes the built-in entries plus the game's learned ones.

**Tech Stack:**
- C11, held to `-Wall -Wextra -Werror`;
- Rust 1.88 (workspace in `host/`), with `zx-sys` bindgen over `core/sound/sound.h`;
- the Docker device build (`tools/fm1-docker`);
- emu-gate.

**Spec:** `docs/superpowers/specs/2026-11-15-learned-sound-entries-design.md` (read it first; the
section numbers below are the spec's).

## Global Constraints

**Rules from the spec:**
- **Unattended:** the learner needs no input from the owner and has no review step.
- **Unsure means beeper:** any doubt means no entry for that routine (spec §2).
- **Built-ins stay:** the built-in C entries stay as they are, and a routine a built-in entry
  matches gets no learned entry.
- **No third-party game bytes in the repo.** Repo tests use repo games (`third_party/games/...`,
  `games/test/`) and programs generated in the tests. The owner's games (`~/Downloads/zx`) are
  measured locally only.
- **No regression:** with no learned entries, FM: engines plays sample for sample as today, and
  Beeper and FM: sound are untouched.
- **Core hook is host-only:** the trace hook is compiled only with `ZX_TESTING` (host build). The
  firmware's core must not change: the RAMTEXT figure from the device build is unchanged by Task 1.
- **RAMTEXT:** 48,456 of 49,152 bytes before this plan. The learned-entry code runs from flash (XIP,
  the `app/pixmodes.c` pattern), and the plan's total RAMTEXT growth stays under 250 B.
- **A bad sound section never breaks games.** It means no learned entries: for that game, or for
  the whole pack if the section header is bad.

**Proposed values (spec):**

| Setting | Value |
|---|---|
| Listening per game | 3 min of emulated time (9,000 frames) |
| Untouched part | 40 s |
| Start keys | ENTER, SPACE, 0, 1, Kempston fire |
| Inliers | 85% |
| Distinct notes | at least 3 |
| Pitch span | at least half a semitone |
| Highest frequency | 5 kHz |
| Replay claim share | 90% |
| Learned entries per game | at most 8 |
| Host time | under 10 s per game, under 5 min for 29 games |

**Project conventions:**
- Commit messages are a single line with no trailers, for example
  `Learned sound: the trace hook (host only)`.
- Never name other emulator projects in project files.
- Every new file carries the SPDX header (`reuse lint` must pass).
- Device rule: never flash, SysEx or open MIDI. There is no hardware.

**Speed rule (owner):**
- Each task's inner loop is host-only: `cd host && cargo test --release -p <crate>`.
- The device build runs only in Tasks 1, 4 and 11.
- The full emu-gate suite runs once, in Task 11.

**Commands:**

| What | Command |
|---|---|
| Host tests | `cd host && cargo test --workspace --exclude emu-gate` |
| Device builds | `tools/fm1-docker cmake --build --preset fm1 && tools/fm1-docker cmake --build --preset fm1-preview` |
| Packages | `FMZX_SKIP_DEVICE_BUILD=1 tools/make-packages --offline` |
| emu-gate | `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate` |
| Licences | `reuse lint` |

Work on branch `plan9-learned-sound`.

## Review Focus

1. **A malformed sound section:** truncated, wrong counts, a program running off its end, stack
   overflow, a division by zero. The firmware and the host decoder must reject it without reading
   outside the section. The game then plays as if it had no learned entries. Tested in Task 2
   (`decode_rejects_*`) and Task 3 (`sound_section_bad_*`).
2. **Code that moves or changes after learning:** the game unpacks or relocates its player. The
   signature digest and the relocated words must make a moved routine still match and a changed one
   stop matching. Tested in Task 2 (`learned_entry_moves_with_code`).
3. **A sound effect that looks like a tune:** a stepped sweep, or a two-tone siren. It must get no
   entry, heard or code-only. Tested in Task 8 (`refuses_sweep`, `refuses_siren`).
4. **A game whose learner run crashes or loops:** a halted CPU, a game that never makes a sound,
   a decoder that hits an undocumented opcode. The pack still builds and the game is reported
   "no music learned (reason)". Tested in Task 9 (`learn_never_fails_the_pack`).
5. **The octave doubt:** one channel toggled from two OUT sites vs two XOR channels in unison.
   Each gets its own correct partition, and an unresolvable one is refused. Tested in Task 7
   (`toggler_two_sites_one_channel`, `xor_unison_two_channels`, `ambiguous_partition_refused`).

---

## File Structure

| File | Responsibility |
|---|---|
| `core/zx/zx.h`, `core/zx/timing.c`, `core/zx/machine.c` | host-only trace hook (`ZX_TESTING`) |
| `host/crates/zx-sys/src/lib.rs` | `Machine::set_trace`, `TraceEv`; `SndEngines::with_learned`, `SndMixer::set_learned`; `pack_sound_at` |
| `core/sound/sound.h` | `snd_engine.read` gains its entry; `snd_engine.data`; learned-entry API |
| `core/sound/learned.c` (new) | bytecode interpreter, entry decoder, generic reader (flash on the device) |
| `core/sound/engines.c`, `engines_builtin.h`, `engines_test.c` | reader signature change only |
| `app/pack.h`, `app/pack.c`, `docs/pack-format.md` | pack version 2, sound section reader |
| `platform/fm1/main.c`, `platform/fm1/CMakeLists.txt`, `core/CMakeLists.txt` | learned table at game start; `learned.c` in XIP |
| `host/crates/soundlearn/` (new crate) | `prog.rs` bytecode builder; `entry.rs` encoder; `z80.rs` decoder; `listen.rs`; `model.rs` code reader; `fit.rs`; `gate.rs` replay; `learn.rs` pipeline; `report.rs` |
| `host/crates/soundlearn/tests/fixtures/mod.rs` | generated engine routines (hand-assembled, documented listings) |
| `host/crates/fmzx-pack/src/keys.rs` (moved from `music::corpus`) | key scripts |
| `host/crates/fmzx-pack/src/pack.rs`, `personal.rs` | sound section writer; learner call, report, cache |
| `host/crates/music/src/corpus.rs` | learned column |
| `host/crates/emu-gate/tests/fw_personal.rs` | learned tune plays FM on the firmware |
| `BUILDING.md`, `docs/controls.md` | owner docs |

---

### Task 1: The trace hook (host only)

Port the method C spike's hook (it was +62 lines; the listing is in the spike's worktree diff,
reproduced in the interfaces below). Every new field and call is inside `#ifdef ZX_TESTING`.

**Files:**
- Modify: `core/zx/zx.h` (types and fields inside the existing `#ifdef ZX_TESTING` part of `zx_machine`), `core/zx/timing.c`, `core/zx/machine.c`
- Modify: `host/crates/zx-sys/src/lib.rs`
- Test: `host/crates/zx-tests/tests/trace.rs` (new)

**Interfaces:**
- Produces (C, `ZX_TESTING` only):
  ```c
  enum { ZX_TR_INSN = 0, ZX_TR_READ = 1, ZX_TR_WRITE = 2, ZX_TR_OUT = 3, ZX_TR_INTA = 5 };
  typedef struct { uint8_t kind, val; uint16_t pc, addr; uint32_t t, cont; zx_regs regs; } zx_trace_ev;
  typedef void (*zx_trace_fn)(void *ctx, const zx_trace_ev *ev);
  void zx_trace_set(zx_machine *m, zx_trace_fn fn, void *ctx, uint8_t mask);   /* mask bit k: kind k */
  ```
  Field meanings:
  - `regs` is filled for INSN, OUT and INTA;
  - `pc` is the current instruction's first byte (`m->insn_pc`);
  - `t` is the frame T-state of the event;
  - `cont` is the running total of contention T-states (a new `cont_total` field, also
    `ZX_TESTING`).
  
  Behaviour:
  - `zx_init` keeps the hook across a re-init (the spike's keep/restore).
  - READ covers operand fetches (`cpu_mem_read` serves both).
- Produces (Rust): `pub struct TraceEv { pub kind: u8, pub val: u8, pub pc: u16, pub addr: u16, pub t: u32, pub cont: u32, pub regs: zx_regs }` and `impl Machine { pub fn set_trace(&mut self, f: Option<Box<dyn FnMut(&TraceEv)>>, mask: u8) }`. The closure lives as long as the machine or until replaced.

- [ ] **Step 1: Write the failing tests** in `zx-tests/tests/trace.rs`:
  - `trace_sees_out_and_reads`: on `tune.sna` (`games/test/`), with mask `1<<ZX_TR_OUT | 1<<ZX_TR_READ`, run 10 frames. Assert:
    - every OUT event has `pc == 0x8014` (its `OUT (FEh),A`);
    - the number of OUT events equals the speaker edges reported by `m.edges()` across those frames;
    - READ events with `pc == 0x801B` (the `LD B,(IX+2)`) read `addr == IX+2`.
  - `trace_off_changes_nothing`: 200 frames of `tune.sna` with and without a hook installed give identical edges and identical RAM.
- [ ] **Step 2: Run them, expect a compile failure** (`set_trace` missing): `cd host && cargo test --release -p zx-tests --test trace`
- [ ] **Step 3: Implement the hook** as in the spike: a `trace()` helper called in `cpu_fetch_opcode` (instruction start), `cpu_mem_read`, `cpu_mem_write`, `cpu_inta` and `cpu_out`; `delay()` adds to `cont_total`. Then the zx-sys binding (an `extern "C"` trampoline, as the spike's `run.rs` did). The addresses come from `TUNE_CODE` in `testprogs.rs`: OUT at 8014h, `LD B,(IX+2)` at 801Bh, `LD C,(IX+3)` at 801Eh.
- [ ] **Step 4: Run the tests, expect them to pass.**
- [ ] **Step 5: Device build: the firmware is unchanged.** Run the device builds. Expect `RAMTEXT: 48456 of 49152` in both, exactly as before.
- [ ] **Step 6: Commit** `Learned sound: the trace hook (host only)`

### Task 2: Learned entries in C: bytecode, decoder, generic reader

**Files:**
- Create: `core/sound/learned.c`
- Modify: `core/sound/sound.h`, `core/sound/engines.c`, `core/sound/engines_builtin.h`, `core/sound/engines_test.c` (reader signature)
- Modify: `host/crates/zx-sys/src/lib.rs`
- Create: `host/crates/soundlearn/` (crate skeleton; add it to the workspace `members`; it depends on `zx-sys` only), `src/lib.rs`, `src/prog.rs`, `src/entry.rs`
- Modify: `host/crates/music/Cargo.toml` (depend on `soundlearn`)
- Test: `host/crates/soundlearn/tests/entry.rs`; `host/crates/music/tests/learned.rs` (new: the tests that need `fmzx_pack::testprogs`, since `fmzx-pack` will depend on `soundlearn` in Task 9 and must not be a dependency of it)

**Interfaces (the binary format: both sides implement it exactly; `docs/pack-format.md` documents it in Task 3).**

**Program.** A bytecode for a stack machine of `uint32_t` values, at most 8 deep and at most 64
bytes long. Operands are little-endian. The context gives the machine (memory at the frame's
end), the anchor and a register set `ctx` (see the reader below).

| Op | Operand | Pushes or does |
|---|---|---|
| `01` | u8 k | k |
| `02` | u16 k | k |
| `03` | u32 k | k |
| `10` | u16 o | `peek(anchor + o)` |
| `11` | u16 o | `peek16(anchor + o)` |
| `12` | u16 a | `peek(a)` |
| `13` | u16 a | `peek16(a)` |
| `14` | u8 r, i8 d | `peek(reg16(r) + d)` |
| `20` | u8 r | register r of `ctx` |
| `30` | | add |
| `31` | | sub (wrapping) |
| `32` | | mul (wrapping) |
| `33` | | div: unsigned; a zero divisor ends the program as *silent* |
| `34` | | and |
| `35` | u8 s | shr by s |
| `36` | u8 s | shl by s |
| `40` | | pop x; x == 0 ends the program as *silent* |

Register numbers: 0 A, 1 F, 2 B, 3 C, 4 D, 5 E, 6 H, 7 L, 8 BC, 9 DE, 10 HL, 11 IX, 12 IY, 13 SP,
14 A', 15 BC', 16 DE', 17 HL'. Opcode `14` takes 8-11 for BC, DE, HL and IX, and 12 for IY.

A program ends at its length with exactly one value on the stack (the result). Anything else is
invalid: an unknown op, a stack underflow or overflow, an operand past the end, or another stack
depth at the end.

**Entry list** (per game; 4-byte aligned; all sizes in bytes):
```
u8 count (<= 8), u8 0, u16 0, then count entries:
  u16 size (multiple of 4, the whole entry), u8 kind (0 music), u8 channels (1-3)
  u64 digest
  u16 len, u8 n_wild, u8 n_reloc
  u8 n_sites, u8 flips, u8 drum, u8 timing (1: timing program present)
  u16 pass, u16 stale_from, u16 stale_len   (stale: anchor-relative pc range; len 0 = none)
  u8 level[3], u8 instrument[3] (reserved, 0)
  u8 prog_len[3], u8 timing_len
  u16 wild[n_wild], u16 reloc[n_reloc], {u16 off, u8 role, u8 chan} sites[n_sites], u16 pos[channels]
  channel programs (prog_len[c] bytes each, c < channels), timing program, zero padding to size
```

**Produces (C, `sound.h`):**
- `snd_engine.read` becomes `void (*read)(const zx_machine *m, const struct snd_engine *en, uint16_t anchor, uint32_t site, const zx_regs *r, snd_b_read *out);`. Every built-in reader gets the `en` parameter, unused, and behaves identically. `engines.c:707` passes `en`.
- `snd_engine` gains `const void *data;`: NULL for built-ins; a learned entry's `snd_learned_ext *`.
- 
  ```c
  #define SND_LEARNED_MAX 8u
  #define SND_PROG_MAX 64u
  typedef struct { const uint8_t *prog[SND_TONAL]; uint8_t prog_len[SND_TONAL]; const uint8_t *timing; uint8_t timing_len;
                   uint16_t stale_from, stale_len; uint8_t level[SND_TONAL]; } snd_learned_ext;
  /* 0 ok (*out the result), 1 silent, -1 invalid. */
  int snd_prog_eval(const uint8_t *p, uint32_t len, const zx_machine *m, uint16_t anchor, const zx_regs *ctx, uint32_t *out);
  /* Decodes a game's entry list (list[0..avail)) into en[0..*n) and ext[0..*n), pointers into list; ZX_OK or ZX_ERR_FORMAT
   * (then *n = 0). Validates every bound and every program (snd_prog_eval-shaped dry run of stack depth). */
  int snd_learned_decode(const uint8_t *list, uint32_t avail, snd_engine *en, snd_learned_ext *ext, uint32_t max, uint32_t *n);
  ```
- **Generic reader** (`snd_learned_read`, static in `learned.c`, set as `read` by the decoder):
  - For each channel c: evaluate `prog[c]` with `ctx` = the site registers `r`. A result `p > 0`
    gives `out->v[c] = { p, level[c] }`; silent gives period 0.
  - `stale`: if `stale_len` and the frame-end pc (`zx_get_regs`) lies in
    `[anchor + stale_from, + stale_len)`, then `out->stale = 2`.
  - Otherwise, with timing:
    - evaluate the timing program with `ctx = r` (giving `e_edge`) and with `ctx` = the frame-end
      registers (giving `e_now`);
    - if `e_now < e_edge`: `stale = 1`, `since = row = e_now`;
    - else `since = row = e_edge` (both in T-states).
  - Without timing, `since` and `row` stay `SND_B_SINCE_ALL`.
  - The name of every learned entry is `"learned"`.
- **Rust (`zx-sys`):**
  - `SndEngines::with_learned(list: &[u8]) -> Result<SndEngines, i32>` builds the built-in table
    followed by the decoded entries. It owns the table, the ext array and a copy of `list`.
  - `SndMixer::set_learned(&mut self, list: &[u8]) -> Result<(), i32>` does the same through
    `snd_mixer_set_engines`.
- **Rust (`soundlearn`):**
  - `pub enum Op { K(u32), PeekRel(u16), Peek16Rel(u16), Peek(u16), Peek16(u16), PeekReg(u8, i8), Reg(u8), Add, Sub, Mul, Div, And, Shr(u8), Shl(u8), ZeroSilent }`
  - `pub fn encode_prog(ops: &[Op]) -> Vec<u8>`: K picks the shortest of 01, 02 and 03.
  - `pub fn eval_prog(ops: &[Op], peek: impl Fn(u16) -> u8, anchor: u16, ctx: &zx_regs) -> Option<Option<u32>>`, the reference evaluator: `None` invalid, `Some(None)` silent.
  - `pub struct LearnedEntry { pub kind: u8, pub digest: u64, pub len: u16, pub wild: Vec<u16>, pub reloc: Vec<u16>, pub sites: Vec<(u16, u8, u8)>, pub pass: u16, pub pos: Vec<u16>, pub flips: bool, pub drum: u8, pub stale: Option<(u16, u16)>, pub level: Vec<u8>, pub progs: Vec<Vec<Op>>, pub timing: Option<Vec<Op>> }`
  - `pub fn encode_list(entries: &[LearnedEntry]) -> Vec<u8>`

- [ ] **Step 1: Write the failing tests** (`soundlearn/tests/entry.rs`):
  - `prog_matches_reference`: for 200 random valid programs (seeded generator over the op set,
    depth-correct) on a `Machine` with random RAM and registers, `eval_prog` agrees with the C
    evaluator. Reach the C side through a test-only zx-sys wrapper,
    `zx_sys::snd_prog_eval(&[u8], &Machine, u16, &zx_regs) -> i32` plus the value.
  - `decode_rejects_bad_lists`. Each must give `Err`:
    - a count of 9;
    - a size not a multiple of 4;
    - a size past the list;
    - `n_sites` arrays past `size`;
    - a program with a stack underflow;
    - a program with a final depth of 2;
    - a program with an operand past its end;
    - a program of 65 bytes;
    - channels 0 or 4.
  - `learned_burst_equals_builtin` (in `music/tests/learned.rs`): express the test table's "Test
    burst" entry (`engines_test.c`) as a `LearnedEntry`:
    - digest `0xd4b551f46da9de4c`, len `0x2C`, wild `[0x13]`, reloc `[0x02, 0x09]`;
    - sites `[(0x1A, SND_SITE_TONAL, 0)]`, pos `[0]`, pass 0, not flips;
    - program `[PeekRel(0x13), ZeroSilent, PeekRel(0x13), K(13), Mul, K(54), Add, K(512), Mul]`:
      the period `2·(13n + 54)·256`, silent for n = 0, as `read_burst`;
    - level 200, no timing.
    
    Run `burst_sna()` (`fmzx_pack::testprogs`) for 600 frames through
    `SndEngines::new_test()` and through `SndEngines::with_learned(list)`. Every decision's
    `confident`, `tone[]` and `drum` are identical.
  - `learned_entry_moves_with_code` (in `music/tests/learned.rs`): copy burst's code to another address (re-relocating its two
    absolute words, as the burst listing documents), then:
    - the learned entry still claims frames (confident decisions);
    - after one wildcard-excluded byte changes, it still matches;
    - after one signature byte changes, it claims none.
- [ ] **Step 2: Run, expect a failure.** `cd host && cargo test --release -p soundlearn --test entry && cargo test --release -p music --test learned`
- [ ] **Step 3: Implement** `learned.c` (interpreter, decoder, generic reader), the reader
  signature change across the built-ins, the zx-sys bindings, and `prog.rs` and `entry.rs`.
  `learned.c` must not be compiled into the firmware's RAMTEXT library. In `core/CMakeLists.txt`,
  exclude it from `ZX_SOUND_SOURCES` with a comment saying the firmware builds it from flash
  (Task 4). `zx-sys/build.rs` compiles every `core/sound/*.c`, so the host gets it.
- [ ] **Step 4: Run the tests, plus the whole music suite** (the built-in readers must be unchanged): `cargo test --release -p soundlearn -p music`. Expect all to pass, `fm_a_unchanged` included.
- [ ] **Step 5: Commit** `Learned sound: entries as data, the bytecode interpreter and the generic reader`

### Task 3: Pack version 2: the sound section

**Files:**
- Modify: `host/crates/fmzx-pack/src/pack.rs`, `app/pack.h`, `app/pack.c`, `docs/pack-format.md`, `host/crates/zx-sys/src/lib.rs`
- Test: `host/crates/fmzx-pack/tests/sound_section.rs` (new)

**Interfaces:**
- **Header:** version 2. Bytes 20-23 are `sound_offset`, bytes 24-27 `sound_len` (0 and 0: no
  section), bytes 28-31 reserved 0. The section lies after the last game's data, 4-aligned, inside
  `total_len`, and covered by the CRC.
- **Section:** `u8 version (1), u8 0, u16 count (= the pack's game count)`, then `u32 list_offset[count]` (relative to the section; 0 means no learned entries for that game), then the lists (Task 2's entry list format), each 4-aligned. A list's length is the distance to the next list offset or to the section's end.
- **Rust:**
  - `pub fn attach_sound(pack: &mut Vec<u8>, lists: &[Option<Vec<u8>>]) -> Result<(), String>` appends the section, rewrites the header and the CRC (`lists.len()` must equal the game count; all-`None` writes no section).
  - `pub fn sound_list(pack: &[u8], index: u32) -> Option<&[u8]>`.
  - `build_pack` writes version 2 with no section.
- **C:** `int pack_sound_at(const uint8_t *base, uint32_t index, const uint8_t **list, uint32_t *len);` returns `ZX_OK` (`*len 0`: none) or `ZX_ERR_FORMAT` for a bad section. `PACK_VERSION 2u`. zx-sys: `pub fn pack_sound_at(pack: &[u8], index: u32) -> Result<Option<Vec<u8>>, i32>`.

- [ ] **Step 1: Write the failing tests:**
  - `sound_roundtrip`: a 3-game pack (the repo's `games/test` programs through a manifest) with
    lists `[Some(a), None, Some(b)]`. Assert:
    - `sound_list` and the C `pack_sound_at` return `a`, nothing and `b`;
    - `pack_open` still accepts the pack;
    - every game still extracts.
  - `sound_section_bad_*`: each gives `ZX_ERR_FORMAT` from `pack_sound_at` while `pack_game_at`
    and extraction still work for every game:
    - an offset past `total_len`;
    - a count unequal to the game count;
    - a list offset past the section;
    - section version 2.
  - `no_section_is_none`: a pack from `build_pack` gives `Ok(None)` for every game.
- [ ] **Step 2: Run, expect a failure.** `cargo test --release -p fmzx-pack --test sound_section`
- [ ] **Step 3: Implement** both sides, and document the header change, the section and Task 2's
  entry list and program format in `docs/pack-format.md`, including the checks the firmware makes.
- [ ] **Step 4: Run** `cargo test --release -p fmzx-pack -p zx-tests`. Expect a pass, including every existing pack test (version 2 everywhere).
- [ ] **Step 5: Commit** `Learned sound: pack version 2 with a sound section`

### Task 4: The firmware plays learned entries

**Files:**
- Modify: `platform/fm1/main.c` (game start), `platform/fm1/CMakeLists.txt` (`learned.c` from flash), `platform/fm1/ramcalls.h` if the XIP copy needs RAMTEXT calls
- Test: device build; `host/crates/emu-gate/tests/fw_personal.rs` (extended in Task 9, once the learner exists)

**Interfaces:**
- Consumes:
  - `pack_sound_at` (Task 3);
  - `snd_learned_decode` and `snd_mixer_set_engines` (Task 2);
  - `snd_engine_table` and `snd_engine_count`.
- **At game start** (`game_start`, after `mixer_init`), if the game has a list:
  - decode it into static (RAM, `POOL`) arrays `snd_engine learned_table[SND_ENGINE_BUILTIN_MAX + SND_LEARNED_MAX]` and `snd_learned_ext learned_ext[SND_LEARNED_MAX]`;
  - fill the table with the built-in entries, then the decoded ones;
  - call `snd_mixer_set_engines(&mixer, learned_table, n)`.
  
  A decode error or no list leaves the built-in table. Add
  `#define SND_ENGINE_BUILTIN_MAX` (the built-in count, with a `_Static_assert`) next to
  `snd_engine_count`.
- **`learned.c` on the device:** compiled into the XIP app, not `ram/libzx_sound.a`. It calls
  `zx_peek` and `zx_get_regs`, which live in RAMTEXT. Route those calls the way
  `app/pixmodes.c`'s device copy does (`redefine_syms.py` renames and `fmzx_xip_*` wrappers in
  `main.c`). `check_reach.py` must pass.

- [ ] **Step 1: Write a host test of the composition** (`soundlearn/tests/entry.rs`, `builtin_then_learned_table`): `SndEngines::with_learned(list)` names its entries `["Castaway", "Colony 8", "ROM BEEP", "Cave Runner title", "Cave Runner in game", "Unison", "learned"]` for a one-entry list. This pins the order the firmware uses.
- [ ] **Step 2: Implement the firmware wiring and the build change.**
- [ ] **Step 3: Device builds.** Expect both to link, `check_reach.py` to pass, and RAMTEXT ≤ 48,706 (under 250 B of growth). Record the figure in the commit's task notes.
- [ ] **Step 4: Commit** `Learned sound: the firmware plays a game's learned entries`

### Task 5: The Z80 decoder

**Files:**
- Create: `host/crates/soundlearn/src/z80.rs`
- Test: `host/crates/soundlearn/tests/z80.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum Flow { Next, Jump(u16), CondJump(u16), Djnz(u16), Call(u16), CondCall(u16), Ret, CondRet, JumpIndirect, Rst(u16), Halt }
  pub struct Insn { pub pc: u16, pub len: u8, pub t: u8, pub t_taken: u8, pub flow: Flow, pub reads: u32, pub writes: u32,
                    pub mem_read: Option<MemRef>, pub mem_write: Option<MemRef>, pub port_out: Option<PortRef>, pub text: String }
  pub enum MemRef { Abs(u16), Reg16(u8), Indexed(u8, i8), Sp }
  pub enum PortRef { Imm(u8), RegC }
  pub fn decode(peek: impl Fn(u16) -> u8, pc: u16) -> Insn
  ```
  - `reads` and `writes` are bit sets over the register numbers of Task 2 (bits 0-17), plus bit
    20 for the flags as a condition.
  - `t` is the uncontended T-states for the not-taken or only path; `t_taken` the taken path's.
  - Undocumented opcodes are decoded (the IXH/IXL forms, SLL, the ED NOPs).
  - `text` is a disassembly for the report.

- [ ] **Step 1: Write the failing tests:**
  - `timing_matches_core`: for every opcode byte sequence in a generated set, place the
    instruction at 8000h in uncontended RAM with `zx_test_set_tstate` and run
    `Machine::test_step`:
    - all 256 unprefixed opcodes, all CB, ED, DD, FD and DDCB/FDCB forms;
    - conditional ones with flags set both ways.
    
    `test_step`'s T-states equal `t` or `t_taken` as the branch went. HALT and the interrupt
    instructions are excluded.
  - `lengths_match_core`: the PC after `test_step` equals `pc + len` for every non-jump.
  - `out_ports`: `D3 FE` gives `PortRef::Imm(0xFE)`; `ED 79` gives `PortRef::RegC`.
- [ ] **Step 2: Run, expect a failure.** `cargo test --release -p soundlearn --test z80`
- [ ] **Step 3: Implement** the table-driven decoder.
- [ ] **Step 4: Run, expect a pass.**
- [ ] **Step 5: Commit** `Learned sound: the Z80 decoder`

### Task 6: Listening

**Files:**
- Create: `host/crates/fmzx-pack/src/keys.rs`: move `Step`, `parse_keys` and `inputs` here from `music/src/corpus.rs`; music re-uses them
- Create: `host/crates/soundlearn/src/listen.rs`
- Modify: `host/crates/soundlearn/Cargo.toml`: depend on `zx-sys` only. `fmzx-pack` depends on `soundlearn` in Task 9, so `soundlearn` must not depend on `fmzx-pack`: it takes inputs as a closure.
- Test: `host/crates/soundlearn/tests/listen.rs`

**Interfaces:**
- Produces:
  ```rust
  pub type Inputs = Box<dyn Fn(u32) -> ([u8; 8], u8)>;     // frame -> key rows, Kempston bits
  pub fn schedule(seed: u64) -> Inputs                       // the generic run (below)
  pub const LISTEN_FRAMES: u32 = 9000;
  pub struct Heard { pub sites: Vec<SiteHeard>, pub frames: u32, pub sound_frames: u32 }
  pub struct SiteHeard { pub pc: u16, pub edges: u32, pub frames_with_edges: u32,
                         pub periods: Vec<PeriodSample>, pub snapshots: Vec<Snapshot> }
  pub struct PeriodSample { pub period_t: f64, pub at_frame: u32, pub snap: u32 }   // index into snapshots
  pub struct Snapshot { pub regs: zx_regs, pub reads: Vec<(u16, u8, u16, u8)> }       // (reader pc, index, addr, value)
  pub fn listen(m: &mut Machine, inputs: &Inputs, frames: u32) -> Heard
  ```
- **The generic schedule** (`schedule(seed)`):
  - frames 0-1999: nothing pressed;
  - then ENTER, SPACE, 0, 1 and Kempston fire in turn, each held 5 frames and followed by 150
    frames of nothing;
  - then random play to the end: every 5-25 frames (xorshift64 from `seed`), hold a random
    non-empty subset of {Q, A, O, P, keys 5-8, SPACE, Kempston up, down, left, right and fire},
    at most 3 at once.
  
  The same seed gives the same inputs. `fmzx-pack` passes the game file's CRC-32 as the seed, or
  the `.keys` script's inputs instead of the schedule.
- **Recording** (bounded memory, streamed; never the whole trace):
  - Per OUT site: flips of the speaker bit (bit 4) from that site.
  - Each period sample is `t(k) − t(k−2) − contention between`, from `cont` (the spike's method).
  - At the site's next OUT after a flip, take a snapshot: the registers plus every value read
    since the previous flip of that site, keyed by (reader pc, read index in that instruction).
  - Keep at most 4,096 period samples and 4,096 snapshots per site, and 64 sites. Drop the oldest
    beyond that.

- [ ] **Step 1: Write the failing tests:**
  - `listen_tune`: `listen` on `games/test/tune.sna` (read from the repo) for 600 frames with no inputs. Assert:
    - exactly one site, pc 8014h;
    - its period samples cluster on the 8 `TUNE_HZ` pitches (period `3.5e6 / hz` T within 0.5%);
    - at least 8 distinct clusters;
    - snapshots contain reads by pc 801Bh and 801Eh (the B and C reloads).
  - `schedule_is_deterministic`: two `schedule(7)` give equal inputs for frames 0-8999, and
    `schedule(8)` differs somewhere after frame 2,775.
  - `listen_is_fast`: `listen` on `tune.sna` for `LISTEN_FRAMES` takes under 5 s in release.
- [ ] **Step 2: Run, expect a failure.** `cargo test --release -p soundlearn --test listen`
- [ ] **Step 3: Implement** the move of `keys.rs` (music's corpus tests must still pass), then `listen.rs`.
- [ ] **Step 4: Run** `cargo test --release -p soundlearn -p music -p fmzx-pack`. Expect a pass.
- [ ] **Step 5: Commit** `Learned sound: listening (the generic run and the period recorder)`

### Task 7: The code reader

**Files:**
- Create: `host/crates/soundlearn/src/model.rs`
- Create: `host/crates/soundlearn/tests/fixtures/mod.rs`: generated engine routines as `.sna` bytes, built with the layout of `testprogs.rs` (code at 8000h, IM 2 handler), each with a documented listing as `testprogs.rs` does, and each exposing its true formula for the tests
- Test: `host/crates/soundlearn/tests/model.rs`

**Interfaces:**
- Consumes: `z80::decode`.
- Produces:
  ```rust
  pub enum Family { Square, Flips, Slots }
  pub enum Term { Const(i64), Lin { coef_t: i64, x: Input }, Prod { coef_t: i64, x: Input, y: Input }, Recip { num_t: i64, x: Input } }
  pub enum Input { Rel(u16), Rel16(u16), Abs(u16), Abs16(u16), Indexed(u8, i8), Reg(u8) }   // as Task 2's ops
  pub struct ChannelModel { pub sites: Vec<u16>, pub half_period: Vec<Term>, pub silent_if_zero: Option<Input> }  // T-states, sum of terms
  pub struct DurationModel { pub counter: Input, pub per_unit_t: Vec<Term>, pub counts_down_from: Option<Input> }
  pub struct EngineModel { pub anchor: u16, pub code_end: u16, pub family: Family, pub channels: Vec<ChannelModel>,
                           pub pass_t: u16, pub pos: Vec<u16>, pub flips: bool, pub duration: Option<DurationModel>,
                           pub stale: Option<(u16, u16)>, pub table_walk: bool, pub wild: Vec<u16>, pub reloc: Vec<u16> }
  pub enum Refusal { Undecoded(u16), ComputedJump(u16), SelfModUnmodelled(u16), UnknownShape(u16), Ambiguous(u16), NotSpeaker(u16) }
  pub fn read_engine(peek: impl Fn(u16) -> u8, out_pc: u16) -> Result<EngineModel, Refusal>
  pub fn scan_out_sites(peek: impl Fn(u16) -> u8) -> Vec<u16>   // OUT (n),A with even n; OUT (C),r with C set even before it
  ```
  - `half_period` holds T-states between speaker toggles of that channel, as a sum of terms: the
    full period is twice it for a square.
  - `table_walk` means the notes come from memory read through a pointer that the routine
    advances: the "music player" evidence (spec §5.2).
  - `wild` holds operand bytes the routine itself rewrites; `reloc` the absolute words inside
    `[anchor, code_end)`.
- **Approach:**
  - From `out_pc`, build basic blocks forward and backward within ±512 bytes.
  - Find the loops containing the OUT, then evaluate one pass symbolically: per path the
    T-states from `Insn.t` and `t_taken`, the counters (`DJNZ`, `DEC r; JR NZ`, `DEC rr; LD A,h;
    OR l; JR NZ`) and the loads that reload them.
  - A channel's half period is the inner loop's T-states × its counter's reload plus the pass
    overhead.
  - **Channel partition:** sites fed by one shared reload counter are one channel; separate
    counters are separate channels. When the analysis can't tell, refuse with `Ambiguous`.
  - Contention: add the core's average contention for code in 4000h-7FFFh. `zx_sys::contention`
    exists; measure the loop's real pass with `test_step` from a snapshot when in doubt.

- [ ] **Step 1: Write the fixtures** (each listing documented, its true half period as a closure over the fixture's memory):
  - `square_tune` (this is `games/test/tune.sna` itself, read from the repo);
  - `square_one_counter`: a `DJNZ` delay reloaded from a table;
  - `toggler_two_sites`: one counter, two OUT sites toggling the same bit in turn;
  - `xor_unison`: two counters, each XORing 10h into a shared byte and OUTing it, at nearly the
    same reloads;
  - `phase_slots`: an accumulator-add, level-per-channel loop, two channels (a `Recip` term);
  - `sweep`: a routine whose counter is decremented arithmetically every pass (no table);
  - `siren`: alternates two constants;
  - `computed_jump`: a `JP (HL)` inside the loop.
- [ ] **Step 2: Write the failing tests:**
  - `reads_tune`: `read_engine` at 8014h of `tune.sna` gives `Family::Square` and one channel
    whose half period evaluates to the documented `13·B + 16·C + k`. Take `k` from the listing,
    and assert the evaluated period against the measured `TUNE_HZ` within 0.1% for all 8 notes.
    It also gives `table_walk` (IX advanced by 4) and a duration model on HL.
  - `toggler_two_sites_one_channel`: one channel with both sites.
  - `xor_unison_two_channels`: two channels, `flips` true.
  - `ambiguous_partition_refused`: a variant of `toggler_two_sites` where the two sites' counters
    are loaded from the same memory byte but are separate registers gives `Refusal::Ambiguous`.
  - `phase_slots_recip`: two channels with `Recip` terms; evaluated frequencies match the
    fixture's truth within 0.2%.
  - `refuses_computed_jump`: `Refusal::ComputedJump`.
  - `sweep_has_no_table_walk`, `siren_has_no_table_walk`: `table_walk == false`.
  - `scan_finds_sites`: `scan_out_sites` on each fixture lists its OUT addresses, and nothing in
    a RAM full of `D3 FF` (odd port).
- [ ] **Step 3: Run, expect a failure.** `cargo test --release -p soundlearn --test model`
- [ ] **Step 4: Implement `model.rs`.**
- [ ] **Step 5: Run, expect a pass.**
- [ ] **Step 6: Commit** `Learned sound: the code reader (loops, counters, channels, formulas)`

### Task 8: Fit, gates and entry building

**Files:**
- Create: `host/crates/soundlearn/src/fit.rs`, `host/crates/soundlearn/src/gate.rs`
- Test: `host/crates/soundlearn/tests/gate.rs`

**Interfaces:**
- Consumes:
  - `EngineModel` (Task 7);
  - `Heard` and `SiteHeard` (Task 6);
  - `LearnedEntry`, `encode_list` and `SndEngines::with_learned` (Task 2).
- Produces:
  ```rust
  pub struct FitResult { pub inliers: f64, pub distinct_notes: u32, pub span_semitones: f64, pub max_hz: f64 }
  pub fn check_fit(model: &EngineModel, heard: &SiteHeard) -> Result<FitResult, String>   // evaluates model terms on each snapshot vs period sample
  pub fn to_entry(model: &EngineModel, peek: impl Fn(u16) -> u8) -> LearnedEntry          // digest over [anchor, code_end) with wild/reloc, programs from terms, timing from duration
  pub struct ReplayStats { pub routine_frames: u32, pub claimed: u32, pub music_gate: bool }
  pub fn replay(load: &dyn Fn() -> Machine, inputs: &Inputs, frames: u32, entry: &LearnedEntry) -> ReplayStats
  pub enum Verdict { Learned { heard: bool }, Refused(String) }
  pub fn judge(model: &EngineModel, heard: Option<&SiteHeard>, replay: Option<&ReplayStats>, fit: Option<&FitResult>) -> Verdict
  ```
- **Gates (spec §5.2, values from Global Constraints):**
  - **Heard:** `check_fit` must show inliers ≥ 85% (within 0.6% + 160/n T, the spike's
    tolerance), ≥ 3 distinct notes, a span ≥ 0.5 semitone and max ≤ 5 kHz. Then the replay must
    claim ≥ 90% of the frames where the routine's sites sound, and the music gate must pass:
    - ≥ 60% of claimed note time in runs of ≥ 4 frames of one note;
    - pitches recur (some note heard in ≥ 2 separate runs);
    - no ≥ 6 consecutive note changes all in one direction.
  - **Code-only:** `table_walk`, a duration model, and every term exact (no refusal). There is no
    replay.
  - **Built-in wins:** a routine the built-in table matches (`SndEngines::new()` claims its
    frames in the replay, or its digest matches a built-in entry at that anchor) is refused,
    "built-in entry".
  - **Drums:** a heard site in its own routine whose output fails `check_fit` (noise) and sounds
    only between claimed notes of a learned entry becomes that entry's `SND_SITE_DRUM` site, with
    `drum = 200`.
- **Program generation:** each term maps to ops (`Lin`: `x · coef_t · 256`; `Prod`, `Recip`:
  `num_t·256 / x`), summed, times 2 for a full period. `silent_if_zero` is emitted first as
  `Peek…, ZeroSilent`.

- [ ] **Step 1: Write the failing tests:**
  - `learns_tune_end_to_end`: model, fit and replay on `games/test/tune.sna` with no inputs for 600
    frames. Expect `Verdict::Learned { heard: true }`. Then
    `SndEngines::with_learned(encode_list([entry]))` on a fresh run makes confident decisions
    with pitches within 0.5% of `TUNE_HZ`, for ≥ 90% of the tune's sounding frames.
  - `refuses_sweep`, `refuses_siren`: `Refused`, heard or code-only.
  - `builtin_wins`: Castaway, loaded with zx-sys alone (`Machine::new`, `tape_insert` of
    `third_party/games/castaway/castawy.tap`, `tape_autostart`, as `music::pack_tape` does), is
    refused "built-in entry" at its tune's anchor (C864h).
  - `partial_formula_refused`: a model with one term deliberately dropped fails the 90% claim
    gate on `square_one_counter`.
- [ ] **Step 2: Run, expect a failure.** `cargo test --release -p soundlearn --test gate`
- [ ] **Step 3: Implement `fit.rs` and `gate.rs`.**
- [ ] **Step 4: Run, expect a pass.**
- [ ] **Step 5: Commit** `Learned sound: fitting, gates and entry building`

### Task 9: The pipeline in the personal pack

**Files:**
- Create: `host/crates/soundlearn/src/learn.rs`, `host/crates/soundlearn/src/report.rs`
- Modify: `host/crates/fmzx-pack/Cargo.toml` (depend on `soundlearn`), `host/crates/fmzx-pack/src/personal.rs`
- Modify: `host/crates/emu-gate/tests/fw_personal.rs`, `BUILDING.md`, `docs/controls.md`
- Test: `host/crates/fmzx-pack/tests/personal_learn.rs` (new)

**Interfaces:**
- Produces:
  ```rust
  pub const LEARNER_VERSION: u32 = 1;   // bump when learning changes; part of the cache key
  pub struct GameSound { pub entries: Vec<LearnedEntry>, pub line: String, pub details: String }
  pub fn learn(load: &dyn Fn() -> Machine, inputs: &Inputs) -> GameSound   // never panics: a caught panic is "no music learned (learner error: …)"
  ```
- **Order in `learn`:**
  1. listen (`LISTEN_FRAMES`);
  2. collect sites: the heard ones plus `scan_out_sites` on memory at frame 0 and at the end;
  3. `read_engine` each, deduplicating by anchor;
  4. judge (fit and replay for heard routines);
  5. keep at most `SND_LEARNED_MAX` (8) learned entries, heard ones first, then by sounding frames.
- **Report text:**
  - The one-line summary is one of:
    - `"<name>: music learned (<n> routine(s), <heard|code-only|both>, <channels> channel(s))"`;
    - `"<name>: no music learned (<first refusal reason>)"`;
    - `"<name>: no music learned (no speaker routine found)"`.
  - The details, one block per routine: the anchor (hex), the family, the formula text, heard or
    code-only, the gate figures, or the refusal.
- **`personal.rs`:**
  - after preparing each packed game, run `learn`:
    - with `load` re-preparing the game's converted file as a `Machine`;
    - with `inputs` from the game's `.keys` beside the source file if present, else
      `schedule(crc32 of the converted file)`;
  - run the games in parallel with `std::thread::scope`, one thread per core;
  - cache in `<out>/sound-cache/<crc32>-<LEARNER_VERSION>.bin`, holding the encoded list plus
    the line and the details;
  - after `build_pack`, `attach_sound` with the lists;
  - print the lines after the size table;
  - write all details to `<out>/sound-report.txt`;
  - `OUTPUTS` gains `sound-report.txt`, and `sound-cache/` is kept across runs (not cleared).

- [ ] **Step 1: Write the failing tests:**
  - `personal_learns_tune`: a folder with `tune.sna`, `border.sna` and `scroll.sna` (from
    `testprogs`) through `personal::run_with_limit`. Then:
    - the pack's sound list for tune holds one entry, and border and scroll have none;
    - `sound-report.txt` names the tune's routine at 8014h, and the result lines contain
      `tune: music learned` and `border: no music learned`;
    - a second run reads the cache: `sound-cache/` exists, and the run is faster than 1 s for
      these 3 games.
  - `learn_never_fails_the_pack`: a game that is a RAM of `76` (HALT forever, interrupts off) and
    one whose speaker OUT sits next to an `ED FF` in the loop. Both give "no music learned (…)",
    and the pack builds.
  - Extend `fw_personal` (emu-gate): after the existing checks, start `tune`, choose FM: engines
    (the Sound item, as `fw_audio::sound_menu_switches_to_fm` does) and record 3 s of audio. It
    must equal, sample for sample, a host `SndMixer` run in `SND_MODE_FM_B` with
    `set_learned(list)` over the same frames (the `host_audio` pattern of `fw_audio.rs`), and
    contain FM: it must differ from the delayed beeper.
- [ ] **Step 2: Run the host test, expect a failure.** `cargo test --release -p fmzx-pack --test personal_learn`
- [ ] **Step 3: Implement** `learn.rs`, `report.rs` and the `personal.rs` changes. Update the
  docs:
  - BUILDING.md, "Your own games": the pack learns each game's music, the result lines,
    `sound-report.txt`, the cache, and that a `.keys` file beside a game guides the run;
  - `docs/controls.md`, Sound row: FM: engines also plays music the personal pack learned.
- [ ] **Step 4: Run** `cargo test --release -p fmzx-pack -p soundlearn`. Expect a pass. (The emu-gate test runs in Task 11.)
- [ ] **Step 5: Commit** `Learned sound: the personal pack learns each game's music`

### Task 10: The corpus, the built-in agreement and the target games

**Files:**
- Modify: `host/crates/music/src/corpus.rs`, `host/crates/music/src/bin/music-corpus.rs`
- Modify: `host/crates/soundlearn/src/model.rs`, `fit.rs`, `tests/fixtures/mod.rs` (new families, as the targets need)
- Test: `host/crates/music/tests/corpus.rs` (agreement test)

**Interfaces:**
- `Row` gains these fields:
  - `pub learned_fm_pct: f64`: FM: engines with the built-in table plus the game's learned
    entries, through a mixer as `b_fm_pct` is measured;
  - `pub learned_line: String`;
  - `pub learned_truth: Option<Score>`.
  
  `report_txt` and `report_csv` gain the columns. Learning uses `soundlearn::learn` with the same
  loading and inputs as Task 9.

- [ ] **Step 1: Write the failing test** `builtins_agree` (repo games, `music/tests/corpus.rs`). Run the
  learner on Castaway and Colony 8 with the built-in table disabled (a `learn` option
  `ignore_builtins: bool`, test-only):
  - Colony 8 gives entries whose channel periods agree with the built-in reader's within 0.2% on
    every frame both claim (`SndEngines::new().read` vs the learned program on the same machine
    state).
  - Castaway is refused with a reason naming a slide or an unknown shape. A Castaway entry that
    *is* learned must agree with the built-in the same way.
- [ ] **Step 2: Run, expect a failure. Implement, run, expect a pass.**
- [ ] **Step 3: The owner's corpus (local, not committed).**
  - Run `cd host && cargo run --release -p music --bin music-corpus -- ~/Downloads/zx -o <scratch>`.
  - Record each game's learned line, `learned_fm_pct` and truth scores in the task report.
  - **Required:**
    - Cave Runner and Unison, wherever learned, score 0 wrong notes;
    - no learned entry claims frames in the non-music test programs or in Castaway's play;
    - the whole folder finishes in under 5 min, and under 1 min on the cached second run.
- [ ] **Step 4: The targets.**
  - For Bee Line, Night Raid II and Star Pilot in turn, read the refusal in `sound-report.txt`.
  - Where the reason is a loop shape or counter pattern the reader lacks, add a generated fixture
    with the same shape (never the game's bytes: write a new routine of that structure) and its
    test to Tasks 7 and 8's files. Extend `model.rs` until the fixture passes, then re-run the
    corpus.
  - Stop for a game when it is learned with 0 wrong notes against a hand check: compare the
    learned pitch with the edges' measured period in 20 sampled frames, listed in the task
    report. Also stop when the reason is a genuine "unsure" the spec wants left on the beeper,
    and say which.
- [ ] **Step 5: Commit** `Learned sound: corpus column, built-in agreement, target families` (code and fixtures only; no corpus output)

### Task 11: Firmware gate, budgets, wrap-up

**Files:**
- Modify: whatever the runs below show needs fixing; `docs/superpowers/specs/...` is untouched

- [ ] **Step 1: Device builds.** Expect RAMTEXT ≤ 48,706, and `check_reach.py` to pass.
- [ ] **Step 2: Packages and the full emu-gate suite, once.** Expect everything to pass:
  - `fw_personal`, with its learned tune;
  - `fw_audio`, unchanged (built-in table);
  - `fw_budget` within its margins: the worst frame is ≤ 3.6M, and a frame with a learned entry
    playing (the personal tune) is under the Border-scroll figure of 2.50M.
- [ ] **Step 3: `reuse lint` and the host workspace tests.** Expect both to pass.
- [ ] **Step 4: Commit any fixes** with one-line messages.

---

## Self-review notes

**Spec coverage:**

| Spec section | Task(s) |
|---|---|
| §5.2 listen | 6 |
| §5.2 read code | 5, 7 |
| §5.2 fit and gate | 8 |
| §5.2 write | 9 |
| §5.3 | 3 (format), 2 (entry layout) |
| §5.4 | 2 (interpreter, reader), 4 (wiring) |
| §5.5 | 9 (docs, report) |
| §7 | 2, 7, 8, 9, 10, 11 |
| §2 success targets | 10 |

- The octave decision is in 7 and the timing outputs are in 2 and 7, as the spec's §8 calls say.
- Program ops cover the spike's four forms plus `tune.sna`'s two-input sum: `Lin` terms are
  summed, and `Prod` and `Recip` cover the rest.
- Names used across tasks are defined once in an Interfaces block: `LearnedEntry`, `encode_list`,
  `SndEngines::with_learned`, `SndMixer::set_learned`, `attach_sound`, `sound_list`,
  `pack_sound_at`, `EngineModel`, `read_engine`, `scan_out_sites`, `Heard`, `listen`,
  `schedule`, `Inputs`, `judge`, `replay`, `learn`, `GameSound`, `LEARNER_VERSION`.
