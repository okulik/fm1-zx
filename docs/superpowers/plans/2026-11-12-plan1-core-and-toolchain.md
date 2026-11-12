# Plan 1: Toolchain, Z80 speed gate and the 48K core

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A repository whose C core emulates a ZX Spectrum 48K on the host (tested from Rust) and builds for the FM-1's pi32v2 CPU, with a measured go/no-go on redcode/Z80's speed in fm1-emu.

**Architecture:** `core/` is portable C11 (device, host, later wasm) wrapping the vendored redcode/Z80 and Z80InsnClock. `host/` is a cargo workspace: `zx-sys` binds the core (cc + bindgen), `zx-tests` holds behaviour tests, `emu-gate` runs a bare-metal pi32v2 benchmark image in fm1-emu. The pi32v2 build is CMake + Ninja inside a pinned Docker image with JieLi's toolchain.

**Tech Stack:** C11, JieLi pi32v2 clang 4.0.1, CMake ≥ 3.25, Ninja, Docker (`linux/amd64`), Rust ≥ 1.88 (cc, bindgen, object), fm1-emu (`fm1-core`, `fm1-soc`).

**Spec:** `docs/superpowers/specs/2026-11-11-zx48k-fm1-design.md` (sections 3, 5.1-5.3, 5.7, 5.8). Later plans: 2 firmware + emu-gate views, 3 virtual FM-1 page, 4 plain browser page.

## Global Constraints

- Commit messages: one line, no body, no trailers (no Co-Authored-By).
- Never name any other FM-1 firmware project in files, docs or commits; write files fresh rather than copying headers from elsewhere.
- SPDX header on every file we write: `SPDX-FileCopyrightText: 2026 FM-1 ZX contributors` and `SPDX-License-Identifier: GPL-3.0-or-later`.
- Vendored code stays byte for byte, at the pinned commits; we hook around it.
  - redcode/Z80 `fc5d86cc0d631c4521955a45118d15964c09e80d` (LGPL-3.0) → `third_party/Z80/`
  - agaxia/Z80InsnClock `a265388b2e72c31f60abe5ad2f65427feca873ea` (0BSD) → `third_party/Z80InsnClock/`
  - Keep each upstream's `API/`, `sources/<lib>.c`, licence files and README; drop CI, docs build, tests.
- Zeta is **not** vendored: both libraries are built with `Z80_EXTERNAL_HEADER` / the matching InsnClock macro pointing at our `core/z80/z_types.h`, which defines only the `Z*` types and macros they use (find the set by compiling).
- Z80 build options (compile definitions): `Z80_STATIC`, `Z80_WITH_LOCAL_HEADER`, `Z80_WITH_Q`, `Z80_WITH_ZILOG_NMOS_LD_A_IR_BUG`, `Z80_WITH_PRECOMPUTED_DAA`. Not: `FULL_IM0`, `SPECIAL_RESET`, `UNOFFICIAL_RETI`, `PARITY_COMPUTATION`, `EXECUTE`. Z80InsnClock in Extra mode.
- `core/`: C11, integer arithmetic only, no `malloc`, no libc beyond `memcpy`/`memset`/`memmove`/`memcmp`, **no 64-bit division** (no compiler-rt on the device; 64-bit multiply and shifts are fine), no misaligned 16/32-bit accesses.
- Machine constants: 69,888 T-states a frame, 224 a line, 312 lines, INT held 32 T-states from T=0, first contended T-state 14,335, paper line 0 is frame line 64.
- Rust: edition 2021, `rust-version = "1.88"`, workspace lints `unsafe_op_in_unsafe_fn = "deny"`, clippy `all = warn`; `[profile.test] opt-level = 3`.
- fm1-emu: `fm1-core` and `fm1-soc` from `https://github.com/joakimeriksson/fm1-emu` at rev `93cb7ee3f7008b27959ba73073b864e37efd7289`.
- JieLi toolchain: `https://pkgman.jieliapp.com/s/linux-toolchain`, SHA-256 `a1545cddbc451a06bac43eb5cd96d831470c42a97c5c0b1f98879a20fc7647bd`. If the hash no longer matches, stop and ask the owner; never just replace the hash.
- Never flash an FM-1 or open its MIDI ports (none is attached during this plan).

## Review Focus

1. Truncated or corrupt `.sna`/`.z80`/`.tap` data → an error code, never an out-of-bounds read (Task 7, Task 8).
2. A 128K `.z80` snapshot → `ZX_ERR_UNSUPPORTED`, machine untouched (Task 7).
3. A beeper routine toggling faster than `ZX_MAX_EDGES` a frame → later edges dropped but the frame's final level still correct (Task 4).
4. Tape with a bad checksum, a flag mismatch, or at its end → the ROM sees carry clear and emulation keeps running (Task 8).
5. Instruction overshoot at frame ends → no drift: 1,000 frames take exactly 1,000 × 69,888 T-states plus the last overshoot (Task 2).

## File structure

```
LICENSE  README.md  BUILDING.md  .gitignore  .editorconfig
CMakeLists.txt  CMakePresets.json  cmake/pi32v2.cmake  cmake/embed.cmake
core/CMakeLists.txt
core/z80/z_types.h          types/macros replacing Zeta for Z80 and Z80InsnClock
core/zx/zx.h                public machine API (one header)
core/zx/machine.c           memory map, CPU callbacks, frame loop, registers
core/zx/timing.c            contention tables, Z80InsnClock glue (Task 2)
core/zx/ula.c               ports: keyboard, Kempston, border, beeper edges, floating bus
core/zx/screen.c            dirty lines, line decode to palette indices
core/zx/beeper.c            edge → sample renderer
core/zx/snapshot.c          .sna and .z80
core/zx/tape.c              .tap, ROM trap, autostart
third_party/Z80/  third_party/Z80InsnClock/  third_party/zx-rom/{48.rom,README.md}
bench/fm1/{CMakeLists.txt,crt0.S,bench.ld,libc_min.c,zx_bench_main.c}
tools/docker/{Dockerfile,toolchain.sha256}  tools/fm1-docker  tools/fetch-test-data  tools/test-data.sha256
host/Cargo.toml
host/crates/zx-sys/{Cargo.toml,build.rs,src/lib.rs}
host/crates/zx-tests/{Cargo.toml,src/lib.rs,tests/*.rs}   src/lib.rs holds the shared fixtures
host/crates/emu-gate/{Cargo.toml,src/lib.rs,tests/z80_speed.rs}
```

## The C API (built up across tasks; names fixed here)

```c
#define ZX_ROM_SIZE 16384u
#define ZX_RAM_SIZE 49152u
#define ZX_FRAME_TSTATES 69888u
#define ZX_LINE_TSTATES 224u
#define ZX_FRAME_LINES 312u
#define ZX_INT_TSTATES 32u
#define ZX_FIRST_CONTENDED 14335u
#define ZX_PAPER_FIRST_LINE 64u
#define ZX_SCREEN_WIDTH 256u
#define ZX_SCREEN_LINES 192u
#define ZX_MAX_EDGES 4096u
enum { ZX_OK = 0, ZX_ERR_FORMAT = -1, ZX_ERR_UNSUPPORTED = -2 };

typedef struct { uint32_t tstate; uint8_t level; } zx_edge;      /* level = (OUT 0xFE value >> 3) & 3: bit0 MIC, bit1 EAR */
typedef struct { uint16_t af, bc, de, hl, af_, bc_, de_, hl_, ix, iy, sp, pc;
                 uint8_t i, r, iff1, iff2, im; } zx_regs;
typedef struct zx_machine zx_machine;   /* full definition in zx.h so callers allocate it statically */

void zx_init(zx_machine *m, const uint8_t *rom);               /* rom: 16 KB, outlives m; power-on state */
void zx_run_frame(zx_machine *m);
void zx_get_regs(const zx_machine *m, zx_regs *r);
void zx_set_regs(zx_machine *m, const zx_regs *r);
uint8_t zx_peek(const zx_machine *m, uint16_t addr);
void zx_poke(zx_machine *m, uint16_t addr, uint8_t value);     /* ignores ROM; marks dirty lines like a CPU write */
uint32_t zx_frame_tstate(const zx_machine *m);                 /* T-states already run into the next frame */
uint32_t zx_frame_count(const zx_machine *m);
void zx_set_keys(zx_machine *m, const uint8_t rows[8]);        /* row r = address line A(8+r); bit set = pressed */
void zx_set_kempston(zx_machine *m, uint8_t bits);             /* bit0 right, 1 left, 2 down, 3 up, 4 fire */
const zx_edge *zx_edges(const zx_machine *m, uint32_t *count);
uint8_t zx_border_line(const zx_machine *m, unsigned frame_line);
int zx_line_dirty(const zx_machine *m, unsigned y);
void zx_screen_line(const zx_machine *m, unsigned y, uint8_t out[256]);   /* palette 0-15, bright = +8 */
uint32_t zx_contention(uint32_t tstate);                       /* delay for a contended access at tstate */
int zx_load_sna(zx_machine *m, const uint8_t *data, uint32_t len);
int zx_load_z80(zx_machine *m, const uint8_t *data, uint32_t len);
int zx_tape_insert(zx_machine *m, const uint8_t *tap, uint32_t len);       /* tap outlives m's use of it */
void zx_tape_autostart(zx_machine *m);                         /* reset, then types LOAD "" + ENTER */

typedef struct zx_beeper zx_beeper;   /* full definition in zx.h */
void zx_beeper_init(zx_beeper *b, uint32_t rate_num, uint32_t rate_den);  /* sample rate = num/den Hz */
uint32_t zx_beeper_frame_samples(const zx_beeper *b);
uint32_t zx_beeper_render(zx_beeper *b, const zx_edge *edges, uint32_t count, int16_t *out);
```

Rust wrapper (`zx-sys`), used by every test: `Machine::new()` (built-in ROM via `include_bytes!`), and safe methods mirroring the C names: `run_frame`, `regs`/`set_regs`, `peek`/`poke`, `frame_tstate`, `frame_count`, `set_keys([u8; 8])`, `set_kempston(u8)`, `edges() -> &[zx_edge]`, `border_line(usize) -> u8`, `line_dirty(usize) -> bool`, `screen_line(usize) -> [u8; 256]`, `load_sna(&[u8]) -> Result<(), i32>`, `load_z80`, `tape_insert(&'static [u8])` (or an owned buffer the `Machine` keeps alive), `tape_autostart`; `Beeper::new(num, den)`, `frame_samples()`, `render(&[zx_edge]) -> Vec<i16>`; free fn `contention(u32) -> u32`. `Machine` holds `Box<zx_machine>` and is `!Sync`.

---

### Task 1: Skeleton, vendored Z80, machine runs code on the host

**Files:**
- Create: `LICENSE` (GPL-3.0 text), `README.md` (one paragraph: what it is, link to the spec), `.gitignore` (`build/`, `host/target/`), `.editorconfig`
- Create: `third_party/Z80/`, `third_party/Z80InsnClock/` (pinned, see Global Constraints), `third_party/zx-rom/48.rom`, `third_party/zx-rom/README.md`
- Create: `core/z80/z_types.h`, `core/zx/zx.h`, `core/zx/machine.c`
- Create: `host/Cargo.toml`, `host/crates/zx-sys/*`, `host/crates/zx-tests/{Cargo.toml,src/lib.rs,tests/cpu.rs}`

**Interfaces:**
- Produces: `zx_init`, `zx_run_frame` (no interrupt or contention yet: runs until `cycles >= ZX_FRAME_TSTATES`, carries the overshoot), `zx_get_regs`, `zx_set_regs`, `zx_peek`, `zx_poke`, `zx_frame_tstate`, `zx_frame_count`; the `Machine` wrapper with those methods.

- [ ] **Step 1: Vendor and record.** Copy the two libraries at their pinned commits. Get `48.rom` from the Fuse emulator's source release (`roms/48.rom`); check CRC32 `0xDDEE531F`; write `third_party/zx-rom/README.md` with its source, CRC32 and SHA-256, and the note that Amstrad permits distributing the unmodified ROMs with emulators.

- [ ] **Step 2: Write the failing tests** in `zx-tests/tests/cpu.rs`:

```rust
#[test] fn runs_a_program_from_ram() {
    let mut m = Machine::new();
    load(&mut m, 0x8000, &[0x3E,0x05, 0xC6,0x03, 0x32,0x00,0x90, 0xF3, 0x76]); // LD A,5; ADD A,3; LD (9000h),A; DI; HALT
    let mut r = m.regs(); r.pc = 0x8000; r.sp = 0xFF00; m.set_regs(&r);
    m.run_frame();
    assert_eq!(m.peek(0x9000), 8);
    assert_eq!(m.regs().af >> 8, 8);
}
#[test] fn rom_is_read_only() { let mut m = Machine::new(); let b = m.peek(0x0000); m.poke(0x0000, b ^ 0xFF); assert_eq!(m.peek(0x0000), b); }
#[test] fn power_on_starts_the_rom_at_zero() { assert_eq!(Machine::new().regs().pc, 0); assert_eq!(Machine::new().peek(0), 0xF3); }
#[test] fn frame_overshoot_is_carried() {
    let mut m = Machine::new();
    for _ in 0..3 { m.run_frame(); assert!(m.frame_tstate() < 23); } // longest Z80 instruction is 23 T
    assert_eq!(m.frame_count(), 3);
}
```

`load` is a fixture in `zx-tests/src/lib.rs` (pokes bytes from an address).

- [ ] **Step 3: Run** `cd host && cargo test -p zx-tests --test cpu`. Expected: compile failure (no `zx-sys`).

- [ ] **Step 4: Implement.** `zx-sys/build.rs` compiles `core/zx/*.c`, `third_party/Z80/sources/Z80.c`, `third_party/Z80InsnClock/sources/Z80InsnClock.c` with `cc` (include dirs `core/z80`, `core/zx`, the two `API/` dirs; defines from Global Constraints; `-std=c11 -Wall -Wextra -Werror` for `core/` only) and runs `bindgen` on `core/zx/zx.h` (allowlist `zx_.*`, `ZX_.*`). `machine.c`: the `Z80` callbacks read ROM (0x0000-0x3FFF) and RAM (0x4000-0xFFFF, `ram[addr - 0x4000]`), ignore ROM writes; `zx_machine` keeps `ram` 4-byte aligned.

- [ ] **Step 5: Run** the tests. Expected: 4 passed.

- [ ] **Step 6: Commit** `git add -A && git commit -m "Add skeleton, vendored Z80 and a host-tested 48K memory map"`

### Task 2: Frame timing, interrupts and contention

**Files:**
- Create: `core/zx/timing.c`; Modify: `core/zx/machine.c`, `core/zx/zx.h`
- Test: `host/crates/zx-tests/tests/timing.rs`

**Interfaces:**
- Consumes: Task 1's machine.
- Produces: `zx_contention(uint32_t)`; `zx_run_frame` now asserts INT for the first 32 T-states and applies memory and I/O contention using Z80InsnClock (Extra mode) to know each M-cycle's T-state.
- Produces (host tests only, declared in `zx.h` under `#ifdef ZX_TESTING`, which `zx-sys/build.rs` defines): `void zx_test_set_tstate(zx_machine *m, uint32_t t)` and `uint32_t zx_test_step(zx_machine *m)` (runs one instruction, returns its T-states including contention); wrapped as `Machine::test_set_tstate` / `test_step`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn contention_pattern() {
    let expect = [6,5,4,3,2,1,0,0];
    for i in 0..8 { assert_eq!(contention(14335 + i), expect[i as usize]); }
    assert_eq!(contention(14335 + 128), 0);                 // past the line's 128 contended T
    assert_eq!(contention(14335 + 224), 6);                 // next line
    assert_eq!(contention(14334), 0);
    assert_eq!(contention(14335 + 192 * 224), 0);           // after the paper
}
#[test] fn fetch_from_contended_ram_is_delayed() {
    // NOP at 0x4000 fetched at T=14335 takes 6 + 4; the same NOP at 0x8000 takes 4.
    assert_eq!(cycles_of_nop_at(0x4000, 14335), 10);
    assert_eq!(cycles_of_nop_at(0x8000, 14335), 4);
}
#[test] fn one_interrupt_per_frame() {           // IM 2 vector table in RAM; handler increments a RAM byte
    let n = count_interrupts(/* frames */ 10, /* di */ false);
    assert_eq!(n, 10);
    assert_eq!(count_interrupts(10, true), 0);
}
#[test] fn no_drift_over_1000_frames() {
    let mut m = Machine::new();
    let start = total_tstates(&m);
    for _ in 0..1000 { m.run_frame(); }
    assert_eq!(total_tstates(&m) - start, 1000 * 69888 + m.frame_tstate() as u64);
}
#[test] fn io_contention_cases() {
    // IN A,(n) at 0x8000 (uncontended fetches, 7 T) started at T=14328 so its I/O cycle starts at 14335.
    assert_eq!(cycles_of_in_at(0x40, 0xFE), 17); // contended high byte, ULA port: C:1, C:3
    assert_eq!(cycles_of_in_at(0x40, 0xFF), 23); // contended high byte, odd port: C:1 x4
    assert_eq!(cycles_of_in_at(0x80, 0xFE), 16); // uncontended high byte, ULA port: N:1, C:3
    assert_eq!(cycles_of_in_at(0x80, 0xFF), 11); // uncontended, odd port: N:4
}
```

Fixtures to add to `zx-tests/src/lib.rs`: `cycles_of_nop_at(addr, t)` and `cycles_of_in_at(a_high, n)` (set PC and A, `test_set_tstate`, `test_step`, return the T-states); `count_interrupts` installs IM 2 with a vector table in RAM pointing at a handler that increments a RAM byte; `total_tstates` = `frame_count * 69888 + frame_tstate`. `zx-sys/build.rs` defines `ZX_TESTING` on the host.

- [ ] **Step 2: Run** `cargo test -p zx-tests --test timing`. Expected: FAIL.
- [ ] **Step 3: Implement** `zx_contention` as a 69,888-entry-free computation (line = (t - 14335) / 224 and phase = (t - 14335) % 224 via incremental bookkeeping or a 224-entry table plus a line check; no division in the per-access hot path if avoidable), and the callbacks applying delays at each M-cycle's T-state. I/O contention follows the four-case table in the test.
- [ ] **Step 4: Run** the tests. Expected: all of `cpu` and `timing` pass.
- [ ] **Step 5: Commit** `git commit -am "Add frame interrupts and 48K memory and I/O contention"`

### Task 3: pi32v2 build in Docker

**Files:**
- Create: `tools/docker/Dockerfile`, `tools/docker/toolchain.sha256`, `tools/fm1-docker`, `CMakeLists.txt`, `CMakePresets.json`, `cmake/pi32v2.cmake`, `cmake/embed.cmake`, `core/CMakeLists.txt`, `bench/fm1/{CMakeLists.txt,crt0.S,bench.ld,libc_min.c,zx_bench_main.c}`, `BUILDING.md`

**Interfaces:**
- Produces: a `zx_core` static library target (core + both vendored libs, same defines as host); `fmzx_embed(<input> <symbol> <out.c>)` CMake function generating `const uint8_t <symbol>[]` and `<symbol>_size`; the bench image `build/fm1/bench/fm1/zx_bench.{elf,bin,dis}`.

- [ ] **Step 1: Dockerfile.** `ubuntu:24.04`; apt: `build-essential cmake ninja-build clang libclang-dev pkg-config git jq curl xz-utils unzip ca-certificates python3`; rustup `1.99.0` minimal with target `wasm32-unknown-unknown` into `/usr/local/{rustup,cargo}` (world-writable); download the JieLi archive, check `TOOLCHAIN_SHA256` build arg with `sha256sum -c`, extract to `/opt/jieli`, symlink `/opt/jieli/toolchain`; `ENV JIELI_TOOLCHAIN=/opt/jieli/toolchain FMZX_BUILD_IMAGE=1`; `git config --system --add safe.directory '*'`.
- [ ] **Step 2: `tools/fm1-docker`** (POSIX sh): if `FMZX_BUILD_IMAGE` is set, `exec "$@"`; else `docker build --quiet --platform linux/amd64 -t fmzx-build:local --build-arg TOOLCHAIN_SHA256="$(cat tools/docker/toolchain.sha256)" tools/docker`, then `docker run --rm --platform linux/amd64 -u "$(id -u):$(id -g)" -v "$root:$root" -w "$root" fmzx-build:local "$@"`.
- [ ] **Step 3: `cmake/pi32v2.cmake`**: `CMAKE_SYSTEM_NAME Generic`; C and ASM compiler `$JIELI_TOOLCHAIN/pi32v2/bin/clang`, target `pi32v2`; linker `$JIELI_TOOLCHAIN/pi32v2/bin/ld` with `CMAKE_C_LINK_EXECUTABLE "<CMAKE_LINKER> <LINK_FLAGS> <OBJECTS> -o <TARGET> <LINK_LIBRARIES>"`; objcopy/objdump from `$JIELI_TOOLCHAIN/common/bin`; include dir `$JIELI_TOOLCHAIN/pi32v2/include`; C flags `-mcpu=r3 -mfprev1 -ffreestanding -fno-builtin`, Release `-O2 -DNDEBUG`; `CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY`; fail with a message pointing at `tools/fm1-docker` if clang is missing. Root `CMakeLists.txt`: C11, `-ffunction-sections -fdata-sections` when cross-compiling, `add_subdirectory(core)` and, when cross-compiling, `bench/fm1`. Preset `fm1`: Ninja, `build/fm1`, Release, that toolchain file.
- [ ] **Step 4: Bench image.** `crt0.S` (section `.text.crt0`): `_start: goto 1f; rts; 1: cli; sp = 0x01C7A000; ssp = 0x01C7C000; call main; goto .`. `bench.ld`: `XIP (rx) 0x02000000 len 0x80000`, `RAM (rwx) 0x01C08000 len 0x6C000`; `.text` (crt0 first, text, rodata) in XIP; `.data` in RAM loaded right after `.text` (assert it); `.bss` NOLOAD; discard `.comment .note*`; link with `--orphan-handling=error -e _start`. Image = objcopy `-j .text` then `-j .data` (JieLi's objcopy takes one `-j` per run) concatenated, plus `.dis`. `libc_min.c`: `memcpy`, `memset`, `memmove`, `memcmp`. `zx_bench_main.c`: copy `.data`, zero `.bss`, copy the embedded ROM to a RAM array, `zx_init`, `bench_mark()`, run `FRAMES` (150) frames, `bench_mark()`, then write at `0x01C80000`: word1 frames, word2 FNV-1a-32 of `zx_peek` over 0x4000-0x5AFF, word3 final PC, and last word0 = magic `0x5A58424Eu`; then spin. `bench_mark` is `__attribute__((noinline))` with an empty `asm volatile("" ::: "memory")`.
- [ ] **Step 5: Verify:** `tools/fm1-docker cmake --preset fm1 && tools/fm1-docker cmake --build --preset fm1`. Expected: builds with no warnings from `core/`; `zx_bench.bin` exists; `llvm-nm`/`objdump` shows no undefined symbols (in particular no `__udivdi3`/`__divdi3`). Record the `.text` size of `zx_core` in `BUILDING.md`.
- [ ] **Step 6: `BUILDING.md`**: host build/test commands, firmware build commands, the toolchain-hash procedure (download, check it came from JieLi over HTTPS and its top directory is `jieli-linux-toolchains-<version>`, then ask the owner), and that Docker must be running.
- [ ] **Step 7: Commit** `git add -A && git commit -m "Add the pi32v2 Docker build and the Z80 benchmark image"`

### Task 4: Speed gate in fm1-emu

**Files:**
- Create: `host/crates/emu-gate/{Cargo.toml,src/lib.rs,tests/z80_speed.rs}`

**Interfaces:**
- Consumes: `zx_bench.{elf,bin}` (Task 3); `Machine` (host).
- Produces: `emu_gate::run_bench(build: &Path, name: &str, marks: usize) -> Run { phases: Vec<u64>, words: [u32; 8] }` and `emu_gate::step(cpu, soc)` stepping through a bus that panics on any misaligned 16/32-bit access to RAM (`fm1_soc::RAM_BASE`, 1 MiB) — Plan 2 reuses both.

- [ ] **Step 1: Write the test** (`#[ignore = "needs the fm1 build: set FMZX_FM1_BUILD and run with --ignored"]`):

```rust
#[test] #[ignore = "..."]
fn redcode_z80_fits_the_frame_budget() {
    let r = run_bench(&build_dir(), "zx_bench", 2);
    assert_eq!(r.words[1], 150);
    let mut m = Machine::new(); for _ in 0..150 { m.run_frame(); }
    assert_eq!(r.words[2], fnv1a((0x4000..0x5B00).map(|a| m.peek(a))), "device and host screens differ");
    assert_eq!(r.words[3], u32::from(m.regs().pc));
    let per_t = r.phases[0] as f64 / (150.0 * 69888.0);
    println!("pi32v2 instructions per Z80 T-state: {per_t:.2}");
    assert!(per_t < 30.0, "over the budget: {per_t:.2} instructions per T-state");
}
```

`fnv1a` is FNV-1a-32 (offset 0x811C9DC5, prime 0x01000193), the same as the bench's. `run_bench` loads the `.bin` into `Soc::new(1 << 20, bin)`, starts `Cpu::new(fm1_soc::XIP_BASE)`, records the step count each time `pc` equals `bench_mark`'s address (from the ELF via `object`), stops at the magic, panics after 2e9 steps. Implement `fm1_core::Bus` for the checking wrapper by forwarding every trait method to `Soc` (read fm1-core's `Bus` trait at the pinned rev for the full list).

- [ ] **Step 2: Run** `FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate -- --ignored --nocapture`. Expected: PASS and the printed figure. 30 instructions per T-state is 2.1 M instructions a frame, 44 % of a 240 MHz frame; the rest is the screen, audio and input.
- [ ] **Step 3: If it fails the budget or the hashes differ, stop.** Report the figure and the per-function profile (instructions by symbol from the ELF) to the owner; the spec's next step is superzazu/z80, which the owner decides. Do not continue to Task 5.
- [ ] **Step 4: Record** the figure, the date and the commit in `BUILDING.md` under "Measured budgets".
- [ ] **Step 5: Commit** `git add -A && git commit -m "Add the fm1-emu speed gate for the Z80 core"`

### Task 5: ULA ports

**Files:**
- Create: `core/zx/ula.c`; Modify: `core/zx/machine.c`, `core/zx/zx.h`
- Test: `host/crates/zx-tests/tests/ula.rs`

**Interfaces:**
- Produces: `zx_set_keys`, `zx_set_kempston`, `zx_edges`, `zx_border_line`; edges and border lines reset at the start of each frame. Test-only (`ZX_TESTING`): `uint8_t zx_test_level(const zx_machine *m)`, the current beeper level, wrapped as `Machine::test_level`.

- [ ] **Step 1: Write the failing tests:**
  - `keyboard_half_rows`: press `rows[1] = 0b00001` (A); `IN A,(0xFE)` with B=0xFD reads bit0 = 0, bits 5 and 7 = 1; with B=0xFE (row 0) bit0 = 1; with B=0x00 (all rows) bit0 = 0.
  - `ear_bit_follows_last_out` (issue 3): after `OUT (0xFE),0x10` bit 6 of an FE read is 1; after `OUT (0xFE),0x00` it is 0.
  - `kempston`: `set_kempston(0b10001)`; `IN A,(0x1F)` reads `0x11`; with no bits, `0x00`.
  - `border_per_line`: border 7 from the previous frame; `OUT (0xFE),2` whose I/O cycle lands at T = 1000 (via `test_set_tstate`/`test_step`) → `border_line(4) == 7`, `border_line(5) == 2`, `border_line(311) == 2`; a write landing at T = 5 × 224 exactly takes effect from line 5.
  - `beeper_edges_have_tstates`: two OUTs toggling EAR at known T-states → `edges()` = `[(t1, 2), (t2, 0)]` (within ±1 T of the I/O cycle per Z80InsnClock); repeating the same level adds no edge.
  - `edge_overflow_keeps_final_level` (Review Focus 3): 5,000 toggles in one frame → `edges().len() == 4096` and `test_level()` after the frame matches the last OUT.
  - `floating_bus`: `IN A,(0xFF)` executed so its I/O read lands on a paper T-state reads the ULA's current bitmap/attribute byte (8-T pattern: bitmap, attr, bitmap+1, attr+1, then four idle = 0xFF); outside the paper it reads 0xFF.
- [ ] **Step 2: Run** `cargo test -p zx-tests --test ula`. Expected: FAIL.
- [ ] **Step 3: Implement** in `ula.c`: even port = ULA (keyboard read; OUT sets border bits 0-2 and records the edge level); `(port & 0xFF) == 0x1F` = Kempston; other odd ports = floating bus.
- [ ] **Step 4: Run** all host tests. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -am "Add keyboard, Kempston, border, beeper edges and floating bus"`

### Task 6: Screen: dirty lines and decode; ROM boot proof

**Files:**
- Create: `core/zx/screen.c`; Modify: `core/zx/machine.c`, `core/zx/zx.h`
- Test: `host/crates/zx-tests/tests/screen.rs`; fixture `screen_text(&Machine, row) -> String` in `zx-tests/src/lib.rs`

**Interfaces:**
- Produces: `zx_line_dirty`, `zx_screen_line`. Dirty bits clear at the start of each `zx_run_frame`; a write that changes a bitmap byte marks its line, an attribute byte its 8 lines; the FLASH phase (`(frame_count / 16) & 1`) toggling marks the lines of cells with FLASH set; after `zx_init` and any snapshot load all 192 lines are dirty.

- [ ] **Step 1: Write the failing tests:**
  - `boots_to_the_copyright_line`: 200 frames from power-on → `screen_text(&m, 23) == "© 1982 Sinclair Research Ltd"` (trailing spaces trimmed). `screen_text` matches each 8×8 cell against the ROM font at 0x3D00 (chars 0x20-0x7F; 0x7F is `©`), `?` when nothing matches.
  - `line_decode`: poke bitmap byte at y=9, x=8..15 = `0b1010_0000` and its attribute = ink 2, paper 5, bright → `screen_line(9)[8..12] == [10, 13, 10, 13]`.
  - `flash_swaps_after_16_frames`: FLASH cell decodes ink/paper normally in frames 0-15 and swapped in 16-31.
  - `dirty_marks`: after a frame with one bitmap write only that line is dirty; same-value writes mark nothing; an attribute write marks 8 lines; the frame after, none.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.** Bitmap address → y: `((a >> 8) & 7) | ((a >> 2) & 0x38) | ((a >> 5) & 0xC0)` for `a = addr - 0x4000`.
- [ ] **Step 4: Run** all host tests. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -am "Add screen decode with dirty lines and the ROM boot test"`

### Task 7: Snapshots

**Files:**
- Create: `core/zx/snapshot.c`; Test: `host/crates/zx-tests/tests/snapshot.rs`; fixtures `make_sna(regs, ram, border) -> Vec<u8>`, `make_z80_v1(regs, ram, compressed) -> Vec<u8>`, `make_z80_v3(regs, ram, hw_mode) -> Vec<u8>`

**Interfaces:**
- Produces: `zx_load_sna`, `zx_load_z80` (return `ZX_OK`, `ZX_ERR_FORMAT` or `ZX_ERR_UNSUPPORTED`; on error the machine is unchanged).

- [ ] **Step 1: Write the failing tests:**
  - `sna_round_trip`: registers (all fields), IM 2, IFF, border 3, RAM pattern → after load `regs()` match (PC popped from the stack, SP + 2), RAM matches, `border_line(0) == 3` after one frame with no OUT.
  - `z80_v1_compressed_and_plain`: same state through v1 compressed (`ED ED nn bb` runs, end marker `00 ED ED 00`) and uncompressed → identical machine state.
  - `z80_v3_48k_pages`: pages 4, 5, 8 → 0x8000, 0xC000, 0x4000; hardware modes 0 and 1 (v2) and 0, 1, 3 (v3) load.
  - `rejects_128k` (Review Focus 2): v3 with hardware mode 4 → `Err(ZX_ERR_UNSUPPORTED)`, and a probe byte poked before is unchanged.
  - `rejects_truncated` (Review Focus 1): every prefix of a valid `.sna`, v1 and v3 file, and a v1 stream with a run past 48 KB → `Err(ZX_ERR_FORMAT)`, never a crash (run under the host build's `-fsanitize=address,undefined` when `ZX_SANITIZE=1`; add that switch to `build.rs`).
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement** (decode into a scratch check pass first, then commit to the machine, so errors leave it unchanged; no extra 48 KB buffer: validate fully, then decode).
- [ ] **Step 4: Run** all host tests, then `ZX_SANITIZE=1 cargo test -p zx-tests --test snapshot`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -am "Add .sna and .z80 snapshot loading"`

### Task 8: Tape loading and the Z80 accuracy suite

**Files:**
- Create: `core/zx/tape.c`, `tools/fetch-test-data`, `tools/test-data.sha256`; Test: `host/crates/zx-tests/tests/tape.rs`; fixtures `basic_line(n, tokens) -> Vec<u8>`, `tap_block(flag, data) -> Vec<u8>`, `tap_header(kind, name, len, p1, p2) -> Vec<u8>`

**Interfaces:**
- Produces: `zx_tape_insert`, `zx_tape_autostart`. The trap: when an opcode is fetched at 0x0556 (LD-BYTES) and a tape is inserted, `fetch_opcode` returns `Z80_HOOK` and the `hook` callback performs the load, then returns `0xC9` (RET); with no tape it returns the ROM's own opcode. Semantics on entry A = flag, IX = destination, DE = length, carry = LOAD (set) or VERIFY (clear): consume the next block; success (flag matches, length ≥ DE, XOR checksum good) → carry set, IX += DE, DE = 0; otherwise carry clear (flag mismatch and bad checksum consume the block; end of tape consumes nothing). VERIFY compares instead of writing.

- [ ] **Step 1: Write the failing tests:**
  - `autostart_loads_basic_and_code`: tape = BASIC `10 LOAD ""CODE : RANDOMIZE USR 32768` (tokens LOAD 0xEF, CODE 0xAF, RANDOMIZE 0xF9, USR 0xC0, number 32768 as `"32768"` + `0x0E 00 00 00 80 00`), header autostart line 10, then a CODE block at 32768: `LD A,5Ah; LD (4000h),A; JR $`. `tape_autostart()`, run ≤ 600 frames → `peek(0x4000) == 0x5A` and PC within the `JR $`.
  - `bad_checksum_and_end_of_tape` (Review Focus 4): direct `CALL 0556h` harness with a corrupted block → carry clear, next call loads the following block; at tape end → carry clear and further frames run.
  - `flag_mismatch_skips_block`, `verify_mode_does_not_write`.
  - `rejects_malformed_tap` (Review Focus 1): a block length past the end → `Err(ZX_ERR_FORMAT)`.
  - `z80test_full_passes` (release only, `#[cfg_attr(debug_assertions, ignore)]`): load `build/test-data/z80full.tap` from Patrik Rak's z80test 1.2a, autostart, run until the screen stops changing for 50 frames or 60,000 frames pass → the screen text reports all tests passed (check the exact wording in z80test's source and assert it). If the file is missing, fail with "run tools/fetch-test-data".
- [ ] **Step 2: `tools/fetch-test-data`**: downloads z80test 1.2a's release archive from its GitHub releases into `build/test-data/`, checks it against `tools/test-data.sha256`, extracts `z80full.tap`. Nothing fetched is committed.
- [ ] **Step 3: Run.** Expected: FAIL.
- [ ] **Step 4: Implement.** Autostart: reset, then press J, SYMBOL SHIFT+P twice, ENTER, each held 3 frames and released 3 frames, starting once the ROM's editor is ready (choose the readiness signal; the test proves it).
- [ ] **Step 5: Run** `tools/fetch-test-data && cd host && cargo test --release -p zx-tests`. Expected: all pass.
- [ ] **Step 6: Commit** `git add -A && git commit -m "Add .tap loading through the ROM trap and the z80test suite"`

### Task 9: Beeper renderer

**Files:**
- Create: `core/zx/beeper.c`; Modify: `core/zx/zx.h`; Test: `host/crates/zx-tests/tests/beeper.rs`

**Interfaces:**
- Produces: `zx_beeper_init`, `zx_beeper_frame_samples`, `zx_beeper_render`; test-only `void zx_beeper_test_dc_block(zx_beeper *b, int on)`. Samples a frame = rate × 69,888 / 3,500,000, kept exact as a reduced fraction with a carried remainder (FM-1: `24000000/544` Hz → 104,832 samples per 119 frames; 48 kHz → 119,808 per 125 frames). Each sample is the time-weighted average of the level over its span (box filter), mapped level → amplitude `{0, 1000, 12000, 13000}` (none, MIC, EAR, both), then a one-pole DC blocker (pole 0.995 in Q15). No 64-bit division.

- [ ] **Step 1: Write the failing tests:**
  - `exact_sample_counts`: FM-1 rate: sum of `frame_samples` over 119 frames (calling `render` with no edges each frame) == 104,832; 48 kHz: 125 frames == 119,808.
  - `silence_is_zero`: no edges, level 0 → all samples 0.
  - `dc_decays`: level held at EAR for 1 s → the last 100 samples have `|x| < 200`.
  - `square_wave_pitch`: EAR toggled every 1,750 T-states for 50 frames → zero crossings per second within 2 % of 2,000 (a 1 kHz tone).
  - `box_filter_half_level`: an edge in the exact middle of a sample's span gives that sample half the step (± 1 LSB, with `zx_beeper_test_dc_block(b, 0)`).
  - `matches_machine_edges`: a `CALL 03B5h` (ROM BEEPER) harness with HL = 1642, DE = 26 (middle C for 0.1 s) → the rendered pitch is 261.6 Hz within 2 %.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** all host tests and rebuild the fm1 preset (the core must still compile for pi32v2 without undefined symbols). Expected: PASS, build clean.
- [ ] **Step 5: Commit** `git commit -am "Add the band-limited beeper renderer"`

---

## Done when

- `cd host && cargo test --release --workspace` passes (with `tools/fetch-test-data` run once).
- `tools/fm1-docker cmake --build --preset fm1` builds clean, and the ignored emu-gate test passes with the figure recorded in `BUILDING.md`.
- The owner has seen the instructions-per-T-state figure before Plan 2 starts.
