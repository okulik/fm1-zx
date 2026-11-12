# Plan 2: The FM-1 firmware

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A firmware image and `.fwsc` update package that boots the FM-1 into a game picker, plays the built-in free games at 50 Hz with beeper sound, draws the screen with RGB444 dirty-line updates under a per-frame SPI byte budget, and is proven in fm1-emu with golden screens, sample-exact audio, budgets and a per-divider display frame-rate report.

**Architecture:** Hardware-free firmware logic lives in a new portable C directory `app/` (pixel packing, LCD row scheduler, key mapping, game pack reader, LZ4, text UI, picker, overlay, on-screen keyboard), built for the device and for host tests through `zx-sys`. `platform/fm1/` is the thin device layer on vendored felucca-bsp: startup, linker script, LCD DMA, audio ISR and sample FIFO, input, the main loop. A Rust tool `fmzx-pack` builds the game pack from a TOML manifest and the `.fwsc`; the pack is placed after the app image in the app slot, not linked. emu-gate grows a firmware harness that boots the real image in fm1-emu.

**Tech Stack:** C11 on JieLi pi32v2 clang 4.0.1; felucca-bsp (MPL-2.0); Rust (cc, bindgen, object, lz4_flex, toml, serde, sha2, png); fm1-emu `fm1-core`/`fm1-soc`; CMake + Ninja in the pinned Docker image.

**Spec:** `docs/superpowers/specs/2026-11-11-zx48k-fm1-design.md` (sections 2, 3, 5.1, 5.2, 5.4-5.7, milestone 3). Plan 1 (`docs/superpowers/plans/2026-11-12-plan1-core-and-toolchain.md`) built `core/`, the toolchain and emu-gate's bench runner; its C API (`core/zx/zx.h`) is used as is.

## Global Constraints

- Commit messages: one line, no body, no trailers (no Co-Authored-By).
- Never name any other FM-1 firmware project in files, docs or commits. Felucca and felucca-bsp are named where they are vendored or credited.
- SPDX header on every file we write: `SPDX-FileCopyrightText: 2026 FM-1 ZX contributors`, `SPDX-License-Identifier: GPL-3.0-or-later`; files that can't carry comments get a `REUSE.toml` entry. `reuse lint` passes after every task.
- **Our startup code, linker script, LCD driver and `.fwsc` writer are written fresh.** Do not copy or adapt code from another firmware's `crt0.S`, `app.ld`, `lcd.c` or package writer (those derive from Felucca's GPL-3.0-only files and would make the image GPL-3.0-only). Register facts, command sequences and file formats may be taken from felucca-bsp headers, the ST7789 datasheet and fm1-emu (MIT).
- Exception: `host/crates/fmzx-pack/src/fwsc.rs` and `src/lz4blk.rs` may follow Felucca's `tools/fm1pkg_make.py` and `tools/lz4blk.py`; then they carry the GPL-3.0-only SPDX licence identifier and Felucca's copyright line, and `LICENSES/GPL-3.0-only.txt` is added. The firmware image never links them.
- Vendored code stays byte for byte at pinned commits; we hook around it.
  - felucca-bsp `https://github.com/hugelton/felucca-bsp` at `7e09895c` (resolve and record the full hash) → `platform/fm1/bsp/{chip,boards/fm1}/`, only the files we build, plus its `LICENSE` and `NOTICE`.
  - Felucca's update loader at `a53ecf42` (Felucca 1.6.1; resolve the repository URL under `https://github.com/hugelton/` and the full hash) → `third_party/felucca-loader/`: `firmware/loader/{crt0_ldr.S,ldr_core.c,loader.c,loader.ld}`, `firmware/src/{usb.c,ota.c,libc.c}` and the six headers `firmware/hal/{fm1_cc.h,fm1_flash.h,fm1_sys.h,fm1_time.h,fm1_usb.h,fm1_xip.h}`, laid out as in Felucca's `firmware/` tree. GPL-3.0-only; it ships as a separate program inside the package (`ota.bin`), never linked into our image. `loader.bin` must be 8,216 bytes with SHA-256 `2cde42b7…03f1aa` (record the full hash from the build).
- `core/` and `app/`: C11, integer only, no `malloc`, no libc beyond `memcpy`/`memset`/`memmove`/`memcmp`, no 64-bit division, no misaligned 16/32-bit access. `app/` never includes a felucca-bsp header.
- Memory map (pi32v2): XIP app image at `0x02000120`, app slot `0x8DFBC` bytes; RAMTEXT `0x01C00000` 48 KiB; `0x01C0C000-0x01C0E000` reserved (no code or data); RAM `0x01C0E000` 64 KiB (`.data`, `.bss`); POOL `0x01C1E000` up to the stacks (NOLOAD); user stack `0x01C78000-0x01C7A000`; supervisor stack `0x01C7A100-0x01C7C000`; NOINIT `0x01C7C000` `0x3D50` bytes.
- SRAM code and XIP code never call each other directly (out of branch reach): cross calls go through function pointers. The audio ISR and everything it calls run from RAMTEXT and read no flash (`const` tables it reads go in `.data`).
- Interrupt priorities: ALNK0 (audio) 3, TIMER5 (10 kHz key scan) 4. `lcd_init` and `input_init` run before interrupts are enabled (PA2 is both backlight and key-matrix /OE). Core 1 is held at boot (`fm1_cpu1_stop`), never started.
- Audio: 44,117.647 Hz = 750000/17 (`zx_beeper_init(b, 750000, 17)`), ALNK0 blocks of 128 stereo frames, samples `int32_t` Q23 (`int16 << 8`), DMA buffers 4-byte aligned.
- LCD: ST7789 240×240 over SPI1, SPI clock 60 MHz / (divider + 1); per-frame byte budget `150000 / (divider + 1)` bytes, divider 0 = 150,000 bytes. COLMOD `0x03` (RGB444, default) or `0x55` (RGB565). Pixels high byte first. SPI DMA reads SRAM only.
- Product identity `FM-1_948` (CMake cache `FMZX_PRODUCT`; not `FM-1_015`, `FM-1_900`, `FM-1_964`).
- Rust: edition 2021, workspace `rust-version = "1.88"` except `emu-gate` (`rust-version = "1.99"`); lints as in Plan 1.
- fm1-emu: `fm1-core`, `fm1-soc` at rev `93cb7ee3f7008b27959ba73073b864e37efd7289`.
- **Device rule:** never flash an FM-1, send it SysEx or open its MIDI ports (none is attached during this plan).

## Review Focus

1. A game pack that is truncated, has a bad magic or CRC, or an entry pointing outside it → the picker shows "No games" (or skips the bad entry) and the firmware keeps running; never a read outside the pack (Task 4).
2. A game file that fails to load (`ZX_ERR_*`) → the picker shows "Load failed", stays usable, the machine is not run (Task 9).
3. A frame whose dirty rows exceed the byte budget, sustained for seconds (full-screen scroller at divider 4) → steady ~21 display fps, rows never starved: every pending row is sent within ⌈rows × row_bytes / budget⌉ + 1 frames (Task 2).
4. Audio FIFO running dry (emulation stalled, paused, menu open) → silence or held level, no click storm, underruns counted only while a game runs; FIFO never overflows (Task 9).
5. Holding many FM-1 keys at once, or a key held across HOME/menu/picker transitions → no stuck Spectrum keys after the transition: all game keys and Kempston bits are released when a game is paused, left or reset (Task 3, Task 10).

## File structure

```
app/CMakeLists.txt
app/pixels.{h,c}      palette, RGB444/RGB565 row packing, LCD row composition (border, crop)
app/lcdsched.{h,c}    pending rows, oldest-first bands under a byte budget, display-fps stats
app/keymap.{h,c}      FM-1 inputs → Spectrum matrix / Kempston; default map; encoder pulses
app/lz4.{h,c}         LZ4 block decoder (bounds-checked)
app/pack.{h,c}        game pack reader
app/ui.{h,c}          30×30 text grid with the ROM font → palette-index rows
app/menu.{h,c}        picker, HOME menu, key-map page, on-screen keyboard state machines
platform/fm1/CMakeLists.txt  crt0.S  app.ld  main.c  lcd.{h,c}  audio.{h,c}  input.{h,c}
platform/fm1/libc_min.c  platform/fm1/VENDORED.md  platform/fm1/bsp/...
third_party/felucca-loader/...   third_party/games/<id>/{<file>,README.md,LICENSE*}
games/games.toml      the manifest; games/test/{scroll.sna,border.sna} our test programs
host/crates/zx-sys    also compiles app/*.c and binds app headers
host/crates/zx-tests/tests/app_*.rs, games.rs
host/crates/fmzx-pack/{Cargo.toml,inputs.toml,src/{main.rs,lib.rs,manifest.rs,pack.rs,testprogs.rs,fwsc.rs,lz4blk.rs},tests/*.rs}
host/crates/emu-gate/src/fw.rs, tests/fw_*.rs, tests/golden/*.png
docs/controls.md  docs/hardware-test.md  docs/install.md  LICENSING.md
```

## Shared names (fixed here; each task's Interfaces block repeats what it needs)

```c
/* app/pixels.h */
#define LCD_W 240u
#define LCD_ROWS 240u
#define LCD_TOP_BORDER 24u          /* LCD rows 0-23 = frame lines 40-63; 216-239 = frame lines 256-279 */
#define LCD_MAX_ROW_BYTES 480u
enum { PIX_RGB444 = 0, PIX_RGB565 = 1 };
unsigned pix_row_bytes(int fmt);                                   /* 360 or 480 */
void pix_pack(int fmt, const uint8_t idx[LCD_W], uint8_t *out);    /* palette indices 0-15 → panel bytes */
void pix_compose_zx(const zx_machine *m, unsigned row, unsigned crop_x, uint8_t idx[LCD_W]);

/* app/lcdsched.h */
typedef struct { uint32_t frames, row_sends, pending_frames_sum, bytes, bands; } lcd_stats;
typedef struct lcd_sched lcd_sched;   /* full definition in the header, statically allocated */
void lcd_sched_init(lcd_sched *s, unsigned row_bytes);
void lcd_sched_mark(lcd_sched *s, unsigned row, uint32_t frame);   /* keeps the older pending_since */
void lcd_sched_mark_all(lcd_sched *s, uint32_t frame);
void lcd_sched_begin_frame(lcd_sched *s, uint32_t frame, uint32_t budget_bytes);
int  lcd_sched_next_band(lcd_sched *s, unsigned *row0, unsigned *rows);   /* 1 = band reserved */
unsigned lcd_sched_pending(const lcd_sched *s);
uint32_t lcd_budget_bytes(unsigned divider);                       /* 150000 / (divider + 1) */
#define LCD_BAND_ROWS 8u
#define LCD_WINDOW_COST 11u                                        /* CASET+RASET+RAMWR bytes per band */

/* app/keymap.h */
#define KM_BUTTONS 14u   /* OCT-,OCT+,FX,SCL,ENV,LFO,EDIT,GLO,HOME,SAVE,ARP,SEQ,PLAY,REC = 0..13 */
#define KM_NOTES 27u     /* note 0 = F3 ... 26 = G5 */
#define KM_ENCODERS 7u
#define KM_INPUTS (KM_BUTTONS + KM_NOTES + 2u * KM_ENCODERS)   /* 55: buttons, notes, enc e ccw = 41+2e, cw = 42+2e */
#define KM_ACTIONS 2u
#define KM_NONE 0xFFu
/* action byte: 0-39 Spectrum key (row*5 + bit; row r = A(8+r)), 0x40|b Kempston bit b (0 right,1 left,2 down,3 up,4 fire) */
typedef struct { uint8_t act[KM_INPUTS][KM_ACTIONS]; } keymap;
void km_default(keymap *k);
typedef struct { uint32_t buttons, notes; int8_t enc[KM_ENCODERS]; } km_input;   /* held masks + detents this poll */
typedef struct km_state km_state;     /* encoder pulse timers; full definition in the header */
void km_reset(km_state *s);
void km_apply(const keymap *k, km_state *s, const km_input *in, uint8_t rows[8], uint8_t *kempston);
#define KM_PULSE_FRAMES 3u

/* app/pack.h */
#define PACK_MAGIC "FMZXPACK"
#define PACK_VERSION 1u
#define PACK_NAME_LEN 28u
enum { PACK_SNA = 1, PACK_Z80 = 2, PACK_TAP = 3 };
enum { PACK_STORED = 0, PACK_LZ4 = 1 };
#define PACK_MAX_RAW 131072u
typedef struct { char name[PACK_NAME_LEN + 1]; uint8_t kind, codec, crop_x; keymap keys;
                 const uint8_t *data; uint32_t stored_len, raw_len, crc32; } pack_game;
int pack_open(const uint8_t *base, uint32_t avail, uint32_t *count);      /* ZX_OK or ZX_ERR_FORMAT */
int pack_game_at(const uint8_t *base, uint32_t index, pack_game *g);      /* ZX_ERR_FORMAT if the entry is bad */
int pack_extract(const pack_game *g, uint8_t *out, uint32_t cap, uint32_t *len);   /* decompress + CRC check */
uint32_t pack_crc32(const uint8_t *p, uint32_t n);                         /* IEEE, reflected, init/xorout 0xFFFFFFFF */
int lz4_block_decode(const uint8_t *src, uint32_t n, uint8_t *dst, uint32_t cap, uint32_t *out_len);

/* app/ui.h, app/menu.h */
#define UI_COLS 30u
#define UI_ROWS 30u
void ui_init(const uint8_t *rom);                    /* font = ROM 0x3D00, chars 32-127 */
void ui_clear(uint8_t ink, uint8_t paper);
void ui_text(unsigned col, unsigned row, const char *s, uint8_t ink, uint8_t paper);
void ui_compose_row(unsigned lcd_row, uint8_t idx[LCD_W]);
uint32_t ui_dirty_rows(uint8_t rows[LCD_ROWS / 8u]);   /* bitmap of LCD rows changed since the last call */
enum { EV_NONE, EV_UP, EV_DOWN, EV_LEFT, EV_RIGHT, EV_OK, EV_BACK, EV_HOME, EV_PAUSE, EV_CAPS, EV_SYM };
enum { VIEW_PICKER, VIEW_GAME, VIEW_MENU, VIEW_KEYS, VIEW_OSK, VIEW_PAUSED, VIEW_MESSAGE };
typedef struct menu menu;       /* full definition in the header */
enum { CMD_NONE, CMD_START, CMD_RESUME, CMD_RESET, CMD_LEAVE, CMD_DIVIDER, CMD_OSK_KEY };
void menu_init(menu *mn, uint32_t game_count);
int  menu_event(menu *mn, int ev, uint32_t *arg);    /* returns a CMD_*; arg = game index, divider, or OSK key 0-39 */
int  menu_view(const menu *mn);
void menu_draw(const menu *mn, const uint8_t *pack_base);     /* draws the current view into the ui grid; names and the
                                                                selected game's keys come from the pack (pack_game_at) */
```

---

### Task 1: Plan 1 follow-ups

**Files:**
- Modify: `core/zx/zx.h`, `host/crates/zx-sys/src/lib.rs`, `BUILDING.md`, `host/crates/emu-gate/Cargo.toml`, `bench/fm1/CMakeLists.txt`
- Test: `host/crates/zx-tests/tests/tape.rs`

**Interfaces:**
- Produces: `Machine::tape_insert` drops its buffer once the tape is ejected by a snapshot load (`Machine.tape == None` after a successful `load_sna`/`load_z80`).

- [ ] **Step 1: Write the failing test** `snapshot_load_drops_tape_buffer` in `tape.rs`: insert a tape, load a valid `.sna`, assert `m.has_tape_buffer() == false` (add `pub fn has_tape_buffer(&self) -> bool` to `zx-sys`).
- [ ] **Step 2: Run** `cargo test --release -p zx-tests --test tape` → FAIL.
- [ ] **Step 3: Fix** `zx-sys`: set `self.tape = None` after a successful snapshot load.
- [ ] **Step 4: Fix docs and config:**
  - The `zx.h` comment on snapshot loads says it releases **all** keys, matching the code.
  - BUILDING.md: correct the stale `.text` size from a fresh build, fix the budget-row commit cell, and re-read the wording.
  - `emu-gate` gets `rust-version = "1.99"`, not the workspace value.
  - `bench/fm1/CMakeLists.txt` fails the configure step when `third_party/zx-rom/48.rom` is not exactly 16,384 bytes.
- [ ] **Step 5: Run** `cargo test --release --workspace` and `tools/fm1-docker cmake --build --preset fm1` → green.
- [ ] **Step 6: Commit** `git commit -m "Fix Plan 1 follow-ups: tape buffer, docs, emu-gate rust-version, ROM size check"`

### Task 2: Pixel packing and the LCD row scheduler (`app/`)

**Files:**
- Create: `app/CMakeLists.txt` (static lib `fmzx_app`, links `zx_core`; `-Wall -Wextra -Werror`), `app/pixels.{h,c}`, `app/lcdsched.{h,c}`
- Modify: `CMakeLists.txt` (add `app`), `host/crates/zx-sys/build.rs` (compile `app/*.c`, bind `app/*.h`)
- Test: `host/crates/zx-tests/tests/app_pixels.rs`, `app_sched.rs`

**Interfaces:**
- Consumes: `zx_screen_line`, `zx_border_line`, `zx_line_dirty` (zx.h).
- Produces: the `app/pixels.h` and `app/lcdsched.h` names in "Shared names".

Palette (index → RGB8): 0-7 = black, blue, red, magenta, green, cyan, yellow, white with channel 0xD7; 8-15 the same with 0xFF (8 is black). RGB444 nibble = `v >> 4` (0xD7 → 0xD); RGB565 = `(r>>3)<<11 | (g>>2)<<5 | b>>3`. RGB444 packs two pixels in three bytes: `R0G0`, `B0R1`, `G1B1`, high nibble first. RGB565 sends the high byte first.

- [ ] **Step 1: Write failing tests** (`app_pixels.rs`):
  - `rgb444_pair_packing`: indices `[2, 13, …]` (red, bright cyan) → first three bytes `0xD0, 0x00, 0xFF`.
  - `rgb565_white_bright`: index 15 → `0xFF, 0xFF`; index 7 → `0xD6, 0xBA`.
  - `row_bytes`: 360 and 480.
  - `compose_border_rows`: a machine with border 2 on frame line 40 and border 5 on frame line 279 → LCD row 0 all index 2, LCD row 239 all index 5.
  - `compose_crop`: with `crop_x` 0, 8 and 16, LCD row 24 equals `zx_screen_line(0)[crop_x .. crop_x + 240]`.
- [ ] **Step 2: Write failing tests** (`app_sched.rs`):
  - `all_pending_fit`: all 240 rows marked at frame 1, budget 150,000 at RGB444 → bands of ≤ 8 contiguous rows in ascending order covering 0-239. `bytes` = 240 × 360 + bands × 11.
  - `budget_limits_and_carries`: 192 rows marked at frame 1, budget 30,000 → bands stop once the next band (shrunk to ≥ 1 row) doesn't fit. Rows sent are the lowest-numbered (all the same age). The rest stay pending with `pending_since` 1.
  - `oldest_first`: rows 100-107 marked at frame 1 and not sent (budget 0); rows 0-7 marked at frame 2; frame 2 with budget for 8 rows → rows 100-107 are sent.
  - `remark_keeps_age`: re-marking a pending row at a later frame keeps the earlier `pending_since`.
  - `scroller_steady_fps`: each frame marks rows 24-215 at RGB444 with budget `lcd_budget_bytes(4)`; after 500 frames, `50 * row_sends / pending_frames_sum` is in 20.0-23.0, and no row has been pending for more than 4 frames.
  - `static_screen_fps`: one row marked every 10th frame with budget 30,000 → fps exactly 50, bytes only for that row.
  - `budget_values`: 30,000, 50,000, 75,000, 150,000 for dividers 4, 2, 1, 0.
- [ ] **Step 3: Run** `cargo test --release -p zx-tests --test app_pixels --test app_sched` → FAIL (missing symbols).
- [ ] **Step 4: Implement** `pixels.c` (palette tables in `.data`-friendly `static const` arrays, 256-entry pair table for RGB444) and `lcdsched.c`.
  - Scheduler algorithm: each `next_band` picks the pending row with the smallest `(pending_since, row)`. It extends the band over the following contiguous pending rows (any age) up to `LCD_BAND_ROWS`. It shrinks the band to what the remaining budget covers at `rows × row_bytes + LCD_WINDOW_COST`. It returns 0 when not even one row fits or nothing is pending.
  - A sent row adds `frame − pending_since + 1` to `pending_frames_sum`.
- [ ] **Step 5: Run** the tests → PASS. Also `tools/fm1-docker cmake --build --preset fm1` builds `fmzx_app`.
- [ ] **Step 6: Commit** `git commit -m "Add RGB444/RGB565 row packing and the LCD row scheduler"`

### Task 3: Key mapping (`app/keymap`) and the manifest key syntax

**Files:**
- Create: `app/keymap.{h,c}`, `host/crates/fmzx-pack/{Cargo.toml,src/lib.rs,src/manifest.rs}` (add to the workspace), `docs/controls.md`
- Test: `host/crates/zx-tests/tests/app_keymap.rs`, `host/crates/fmzx-pack/tests/manifest.rs`

**Interfaces:**
- Produces: the `app/keymap.h` names in "Shared names".
- Produces: `fmzx_pack::manifest::{Manifest, Game, parse_manifest(&str) -> Result<Manifest, String>, encode_keymap(&BTreeMap<String,String>) -> Result<[[u8;2];55], String>}`.
  - The keymap starts as `km_default` (same table, mirrored in Rust and checked against the C one through `zx-sys`). Each listed input replaces that input's actions.
  - A value is one or two actions joined by `+`.

Manifest key syntax:
- Inputs:
  - `button.<OCT-|OCT+|FX|SCL|ENV|LFO|EDIT|GLO|SAVE|ARP|SEQ|REC>`
  - `note.<F3..G5>`, sharps as `F#3`
  - `enc.<SELECT|ALGORITHM|K1|K2|K3|K4|PRESETS>.<cw|ccw>`
  - HOME and PLAY are reserved: mapping either is an error.
- Actions:
  - `key.<A-Z|0-9|ENTER|SPACE|CAPS|SYM>`
  - `kempston.<left|right|up|down|fire>`
  - `none`

Default map (my call, documented in `docs/controls.md`):
- **Kempston:** left = F3, down = G3, right = A3, up = F#3 and G#3, fire = C5, D5, E5, F5, G5.
- **Keys:** ENTER = C4, SPACE = D4, Y = E4, N = F4.
- **Digits on buttons:** 1-8 on OCT-, OCT+, FX, SCL, ENV, LFO, EDIT, GLO; 9 on SAVE; 0 on ARP.
- **Shifts:** CAPS SHIFT on SEQ, SYMBOL SHIFT on REC.

- [ ] **Step 1: Write failing C tests** (`app_keymap.rs`):
  - `default_kempston`: holding notes 0 and 3 → Kempston `0b1010` (left, up), no matrix keys.
  - `default_enter_digits`: C4 + OCT- → ENTER (row 6, bit 0) and `1` (row 3, bit 0).
  - `two_actions`: an input with `CAPS+0` sets both keys.
  - `encoder_pulse`: one cw detent on an encoder mapped to `key.P` holds P for exactly `KM_PULSE_FRAMES` calls. 3 detents in one poll hold it for 9.
  - `release_all`: after `km_reset` and an empty `km_input`, rows are all 0 and Kempston is 0.
  - `home_play_ignored`: buttons 8 and 12 held → no action, even in a keymap where `act` is set for them.
- [ ] **Step 2: Write failing Rust tests** (`manifest.rs`):
  - `parse_minimal`: one game → defaults filled in.
  - `override_note`: `"note.F3" = "key.O"` changes only that input.
  - `reserved_home_rejected`, `unknown_key_rejected`, `three_actions_rejected`.
  - `default_matches_c`: the encoded default equals `km_default` read through `zx-sys`.
- [ ] **Step 3: Run** → FAIL. **Step 4: Implement** `keymap.c` and `manifest.rs` (serde + toml). **Step 5: Run** → PASS.
- [ ] **Step 6: Commit** `git commit -m "Add FM-1 to Spectrum key mapping and the manifest key syntax"`

### Task 4: Game pack: format, LZ4, reader and packer

**Files:**
- Create: `app/lz4.{h,c}`, `app/pack.{h,c}`, `host/crates/fmzx-pack/src/{pack.rs,main.rs}`, `docs/pack-format.md`
- Test: `host/crates/zx-tests/tests/app_pack.rs`, `host/crates/fmzx-pack/tests/pack.rs`

**Interfaces:**
- Consumes: `keymap`, `encode_keymap`, `parse_manifest` (Task 3); `zx-sys` `Machine` for validation.
- Produces:
  - the `app/pack.h` names;
  - `fmzx_pack::pack::build_pack(manifest_path: &Path) -> Result<Vec<u8>, String>`;
  - the CLI `fmzx-pack games <manifest.toml> -o <pack.bin>`.

Format (little-endian, all offsets from the pack start, every field naturally aligned):
- **Header, 32 bytes:**
  - `magic[8] = "FMZXPACK"`, `version u16 = 1`, `count u16`, `total_len u32`
  - `crc32 u32` over bytes `[32, total_len)`
  - `reserved[12] = 0`
- **Then `count` entries of 160 bytes:**
  - `name[29]` (NUL-terminated ASCII, ≤ 28 chars), `kind u8`, `codec u8`, `crop_x u8` (0-16)
  - `offset u32`, `stored_len u32`, `raw_len u32`, `crc32 u32` (of the raw file)
  - `keys u8[110]` (55 × 2), `reserved[6]`
- **Data:** each game's data 4-byte aligned after the entries.
- `codec 1` = one LZ4 block (lz4_flex `compress` block format, no frame); `raw_len ≤ PACK_MAX_RAW`.

- [ ] **Step 1: Write failing C tests** (`app_pack.rs`):
  - `lz4_roundtrip`: lz4_flex-compressed random and repetitive buffers (0, 1, 15, 4,096, 131,072 bytes) decode identically.
  - `lz4_rejects_bad`: a match offset 0, an offset before the output start, a literal or match running past `cap` or past the input → `ZX_ERR_FORMAT`. Fuzz 10,000 random mutations of a valid block: never a crash. Run once under `ZX_SANITIZE=1` on macOS.
  - `pack_open_ok`, plus the failures:
    - `pack_bad_magic`, `pack_bad_crc`
    - `pack_total_len_exceeds_avail`
    - `entry_out_of_bounds` (offset + stored_len > total_len)
    - `raw_len_too_big`
    - `crop_over_16`
    - `name_unterminated`
    - each → `ZX_ERR_FORMAT` (the header ones from `pack_open`, the entry ones from `pack_game_at`).
  - `extract_crc_mismatch` → `ZX_ERR_FORMAT`.
- [ ] **Step 2: Write failing Rust tests** (`pack.rs`):
  - `build_and_read_back`: a manifest with a `.sna`, a `.z80` and a `.tap` (fixtures from `zx-tests`). Read back through the C reader; names, kinds, crop and keys match; extracted bytes equal the inputs.
  - `validation_loads_each_game`: the packer rejects a manifest whose file fails `load_sna`/`load_z80`/`tape_insert`, naming the file.
  - `rejects_oversize`: raw file > `PACK_MAX_RAW` → error.
  - `stored_when_lz4_doesnt_help`: incompressible data → codec 0.
- [ ] **Step 3: Run** → FAIL. **Step 4: Implement.**
  - The packer validates each game by loading it in the host core and running 50 frames. For a `.tap` it uses `tape_autostart` and runs 500 frames.
  - It writes `docs/pack-format.md` (the layout above).
- [ ] **Step 5: Run** → PASS.
- [ ] **Step 6: Commit** `git commit -m "Add the game pack format, LZ4 decoder, reader and packer"`

### Task 5: Text UI, picker, HOME menu and key-map page (`app/ui`, `app/menu`)

**Files:**
- Create: `app/ui.{h,c}`, `app/menu.{h,c}`
- Test: `host/crates/zx-tests/tests/app_ui.rs`

**Interfaces:**
- Consumes: `pack_game` names and keys (Task 4); the ROM bytes (`third_party/zx-rom/48.rom`).
- Produces: the `app/ui.h` and `app/menu.h` names in "Shared names".

Behaviour (the event mapping from FM-1 controls to `EV_*` is done by the device layer in Task 10):
- **Picker** (`VIEW_PICKER`):
  - Title row "FM-1 ZX" and up to 24 game names. A bright-cyan bar marks the selection; the list scrolls.
  - `EV_UP`/`EV_DOWN` move with wrap-around. `EV_OK` → `CMD_START` with the index.
  - With 0 games it shows "No games" and ignores `EV_OK`.
- **Game** (`VIEW_GAME`):
  - `EV_HOME` opens `VIEW_MENU` and the game pauses.
  - `EV_PAUSE` toggles `VIEW_PAUSED`, which shows "PAUSED" over the top border rows and returns `CMD_NONE`.
- **Menu** (`VIEW_MENU`):
  - Items: Resume, Reset game, Keyboard, Key map, LCD speed: <d>, Back to games.
  - `EV_OK` on them → `CMD_RESUME`, `CMD_RESET`, `VIEW_OSK`, `VIEW_KEYS`, `CMD_DIVIDER` (cycles 4 → 2 → 1 → 0 → 4, arg = new divider), `CMD_LEAVE` → `VIEW_PICKER`.
  - `EV_HOME` or `EV_BACK` → `CMD_RESUME`.
- **Key map** (`VIEW_KEYS`):
  - Lists every mapped input and its actions, one per row (e.g. `F3   KEMPSTON LEFT`), paged with `EV_UP`/`EV_DOWN`.
  - `EV_BACK`/`EV_HOME` → menu.
- **On-screen keyboard** (`VIEW_OSK`):
  - The bottom 5 text rows (LCD rows 200-239): 4 rows of 10 keys in Spectrum layout plus a status line. The game keeps running.
  - `EV_LEFT`/`EV_RIGHT` move within a row, `EV_UP`/`EV_DOWN` between rows. `EV_OK` → `CMD_OSK_KEY`, arg = key 0-39, pressed while OK is held.
  - `EV_CAPS`/`EV_SYM` toggle sticky shifts, shown in the status line and applied with the next key.
  - `EV_HOME` closes it.

- [ ] **Step 1: Write failing tests:**
  - `font_from_rom`: `ui_text` "A" at (0,0) → LCD rows 0-7, columns 0-7 match the ROM glyph at `0x3D00 + (65-32)*8`.
  - `dirty_rows`: writing one cell marks exactly its 8 LCD rows.
  - `picker_wrap_and_start`: 3 games; UP from 0 → 2; OK → `CMD_START`, arg 2.
  - `picker_empty`.
  - `menu_items_and_divider_cycle`: OK on LCD speed four times → args 2, 1, 0, 4.
  - `menu_leave` → `CMD_LEAVE` and view picker.
  - `osk_navigation_and_shift`: SYM, then OK on key P → `CMD_OSK_KEY` with P's index and SYM latched for that key, cleared after it.
  - `keys_page_lists_default`: the first lines include `F3` and `KEMPSTON LEFT`.
  - `golden_picker_hash`: an FNV-1a over the 240 composed rows of the picker with 3 names. Update with `FMZX_GOLDEN_UPDATE=1`, stored in `tests/golden/hashes.txt`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "Add the text UI, game picker, HOME menu, key-map page and on-screen keyboard"`

### Task 6: The built-in games and our two test programs

**Files:**
- Create: `third_party/games/<id>/…` for each accepted game, `games/games.toml`, `host/crates/fmzx-pack/src/testprogs.rs`, `games/test/{scroll.sna,border.sna}`, `LICENSES/*` as needed, `REUSE.toml` entries
- Test: `host/crates/fmzx-pack/tests/testprogs.rs`, `host/crates/zx-tests/tests/games.rs`

**Interfaces:**
- Consumes: `build_pack` (Task 4).
- Produces: `games/games.toml`, which every later task's pack is built from. The scroller has id `scroll` and is listed first. The border program has id `border`.

**Licence gate:** commit a game only if its licence permits redistribution in a firmware image and on a web page. The evidence must be an upstream licence file or README in the author's own repository or release. Copy that file into `third_party/games/<id>/` and quote the sentence in its `README.md`, with the source URL, download URL and SHA-256 of the file. A game that fails the gate is left out and the reason recorded in the ledger. Candidates:
1. **Moon Caves** (Eugeny Novikov, 2019). Code MIT, art CC-BY-SA 3.0 (`github.com/EugenyN/Moon Caves` README). File `mooncaves48.tap` from release v1.0.
2. **Castaway** (Juan J. Martínez, 2016). Code GPL-3.0, assets CC-BY-SA (`github.com/reidrac/castaway-zx` README). The `.tap` comes from `usebox.net/jjm/castaway/castaway.zip`; check it matches the repo's build.
3. **Star Field** (Juan J. Martínez, 2015). MIT per the README in `starfield_src_jam.zip`. Accepted only if every third-party component the README mentions, and the loading screen, has a licence that permits redistribution.
4. **Mars Base** (salvaKantero, 2015). GPL-2.0 (`github.com/salvakantero/MarsBase_ZX`). Accepted only if `MARSBASE_EN.TAP` contains none of the cover illustrations the README excludes from the GPL.

Test programs (ours, GPL-3.0-or-later), generated by `fmzx-pack testprogs <dir>` from documented machine code in `testprogs.rs`, committed, and checked equal to a fresh generation:
- `scroll.sna`: IM 1. After each HALT it shifts all 6,144 bitmap bytes left by one byte (column 0 wraps to 31), with an initial pattern so every line differs. The border stays 1.
- `border.sna`: after each HALT, an `OUT (0xFE)` loop of fixed timing. Every 224 T-states (one line) it changes the border colour (cycling 0-7) and toggles the beeper bit 4 every 8 lines, producing a tone of 3,500,000 / (2 × 8 × 224) ≈ 976.6 Hz.

- [ ] **Step 1: Write failing tests:**
  - `testprogs_reproducible`.
  - `scroll_dirties_every_line`: frames 10-60 each report all 192 lines dirty.
  - `border_stripes_and_tone`: in frame 20, frame lines 40-279 hold ≥ 8 distinct colour changes, and edge count is 39 ± 1 per frame.
  - `each_game_runs` (M5): every manifest entry loads, and after 300 frames (tapes with autostart: 1,500) the screen hash changes between frames 250 and 300 (or the game sits at a non-blank title screen). Record each game's frame-300 screen hash in `tests/golden/hashes.txt`.
  - `pack_fits_budget`: the built pack is ≤ 393,216 bytes.
- [ ] **Step 2: Run** → FAIL. **Step 3: Gather the games** through the licence gate, write the manifest (per-game key maps and crop from the games' own instructions; Moon Caves: player 1 W/A/S/D/F on the default Kempston keys' positions), and generate the test programs. **Step 4: Run** → PASS, and `reuse lint` passes.
- [ ] **Step 5: Commit** `git commit -m "Add the built-in free games and the scroller and border test programs"`

### Task 7: Firmware skeleton: felucca-bsp, startup, LCD, boot to the picker; emu-gate firmware harness

**Files:**
- Create: `platform/fm1/{CMakeLists.txt,crt0.S,app.ld,main.c,lcd.h,lcd.c,libc_min.c,VENDORED.md}`, `platform/fm1/bsp/…`
- Modify: `CMakeLists.txt` (cross builds add `app` and `platform/fm1`), `host/crates/emu-gate/{Cargo.toml,src/lib.rs}`
- Create: `host/crates/emu-gate/src/fw.rs`, `tests/fw_boot.rs`, `tests/golden/picker.png`

**Interfaces:**
- Consumes: `fmzx_app`, `zx_core`; `ui_*`, `menu_*`, `pix_*`, `lcd_sched_*`, `pack_open`.
- Produces (device):
  - target `fmzx_app_image` → `build/fm1/platform/fm1/fmzx_app.{elf,bin,dis}`;
  - `lcd_init(int fmt)`, `lcd_set_divider(unsigned d)`, `lcd_pump(void)` (starts the next band when the DMA is idle and the scheduler has one; returns at once), `lcd_busy(void)`;
  - the symbols `fmzx_stats` (struct: `frames`, `underruns`, `overflows`, `view`, `game`, `divider`, `budget`, `pending`, plus the embedded `lcd_stats`) and `fmzx_bootopt`.
- Produces (host): `emu_gate::fw::{Firmware::load(build: &Path) -> Firmware, Firmware::with_pack(&[u8]) -> Firmware, Machine::boot(&Firmware, BootOpt) -> Machine, Machine::run_ms(u64), Machine::press(key_id: u8, ms: u64), Machine::hold(key_id: u8, down: bool), Machine::turn(enc: u8, clicks: i32), Machine::set_adc(ch: u8, v: u16), Machine::lcd_rgb565() -> &[u16], Machine::stats() -> FwStats, Machine::audio() -> &[i16], Machine::cost_of(symbol: &str) -> Vec<u64>, BootOpt { rgb565: bool, divider: u8 }}`.

Device facts:
- **Startup (`crt0.S`, ours):**
  - Disable interrupts, set `sp`/`ssp` to the stack tops, then call `fmzx_cstart`.
  - `fmzx_cstart`, in order:
    - `fm1_cpu1_stop`, `fm1_time_init`, `fm1_wdt_arm(0x0D)`;
    - the **UBOOT gesture**: OCT- held through 1 s after power-on → `fm1_wdt_stop(); fm1_enter_uboot()`, using a raw matrix read before RAM init;
    - `fm1_irq_init`, zero `.bss` and POOL, copy `.data` and `.ram_text`, run `.init_array`, `fm1_mailbox_clear`, then `fmzx_main`.
- **`app.ld`:**
  - The memory map in Global Constraints. The image is `.entry`, `.ram_text`, `.text`/`.rodata`, `.data`, back to back from `0x02000120`.
  - `.ram_text` holds `*libzx_core.a:*(.text*)`, the audio ISR path and the LCD pump.
  - `.data` additionally holds `*libzx_core.a:*(.rodata*)` (the contention and flag tables, copied to RAM).
  - Exports `_app_image_end`, the XIP address one past the image. The pack lives at `ALIGN(_app_image_end, 4096)`.
  - `--orphan-handling=error`, `--gc-sections`, `--emit-relocs`. `ASSERT`s: RAMTEXT ≤ 48 KiB, nothing in `0x01C0C000-0x01C0E000`, image ≤ the app slot.
- **Boot options:** `fmzx_bootopt` is a `uint32_t[2]` at the fixed address `0x01C7FF00` (NOINIT, not cleared). If word 0 is `0x5A584F50` ("ZXOP"), word 1 bit 0 = RGB565 and bits 8-11 = divider. Otherwise the defaults apply: RGB444 and `FMZX_LCD_DIVIDER` (CMake cache, default 4). Clear word 0 after reading it.
- **LCD:**
  - Bring-up: `fm1_lcd_hw_init`, 80 ms, SWRESET `0x01`, 150 ms, SLPOUT `0x11`, 120 ms, COLMOD `0x03`/`0x55`, MADCTL `0x00`, INVON `0x21`, NORON `0x13`, `fm1_lcd_baud(divider)`. Feed the watchdog in the waits.
  - DISPON `0x29` after the first full picture.
  - Band buffers: two `uint8_t[LCD_BAND_ROWS * LCD_MAX_ROW_BYTES]` in POOL, 4-aligned. Compose into one while the other is sent, one DMA per band. Before any command, wait for the transfer (`fm1_lcd_wait`, which counts timeouts).
  - Divider 0 programs baud 0.
- **Audio PLL in fm1-emu:** `Machine::boot` writes the stock audio PLL values to `0x119A0`/`0x119A4` before the first instruction, as the stock boot leaves them. The values are in felucca-bsp's `FM1_AUDIO_PLL_STOCK0/1`. Without them `fm1_audio_init` refuses to start.
- **fm1-emu's LCD ignores COLMOD:** it always parses RGB565. Every emu-gate screen test boots with `BootOpt { rgb565: true, .. }`. RGB444 correctness rests on the host tests of Task 2, plus Task 11's byte counts.

- [ ] **Step 1: Vendor felucca-bsp** (the files the firmware includes: `chip/{fm1_cc.h,fm1_cpu1_stop.h,fm1_gpio.h,fm1_irq.h,fm1_isr.S,fm1_vec.S,fm1_sys.h,fm1_time.h,fm1_timer.h,fm1_xip.h}`, `boards/fm1/{fm1_adc.h,fm1_audio.h,fm1_input.h,fm1_lcd_hw.h,fm1_led_anim.h}` and anything they include) with `LICENSE`, `NOTICE`, and `platform/fm1/VENDORED.md` giving the URL, full hash, file list and "no local changes".
- [ ] **Step 2: Write the failing emu-gate test** `fw_boot.rs`:
  - `boots_to_picker`: a pack of 3 games from `games/games.toml`. After 1,500 ms the display is on, `stats().view == VIEW_PICKER`, and the LCD equals `tests/golden/picker.png` pixel for pixel (`FMZX_GOLDEN_UPDATE=1` rewrites it; on a mismatch write `<build>/picker.actual.png`).
  - `no_pack_shows_no_games`: the image without a pack → "No games" golden.
  - `corrupt_pack_shows_no_games`: a pack with one flipped byte → the same.
  - `no_misaligned_access`: the whole run goes through the alignment-checked bus (report the pc).
  - `uboot_gesture`: OCT- held from power-on → execution reaches `fm1_enter_uboot` (ELF symbol hit) within 1,200 ms.
- [ ] **Step 3: Run** `FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate --test fw_boot` → FAIL. Without the env var, tests fail with a message saying how to build, rather than skipping.
- [ ] **Step 4: Implement** the startup, linker script, `main.c` (init, the picker loop drawing through the scheduler at budget `lcd_budget_bytes(divider)` per 20 ms), `lcd.c`, and the emu-gate harness.
- [ ] **Step 5: Run** the device build and `fw_boot` → PASS.
- [ ] **Step 6: Commit** `git commit -m "Add the firmware skeleton on felucca-bsp: startup, LCD, picker, emu-gate harness"`

### Task 8: Input

**Files:**
- Create: `platform/fm1/input.{h,c}`
- Modify: `platform/fm1/main.c`
- Test: `host/crates/emu-gate/tests/fw_input.rs`

**Interfaces:**
- Consumes: felucca-bsp `fm1_input.h`, `fm1_adc.h`, `fm1_timer.h`.
- Produces:
  - `input_init(void)`;
  - `input_poll(km_input *held, uint32_t *pressed_buttons, uint32_t *pressed_notes)`: an interrupts-off snapshot plus press edges, and encoder detents via `fm1_enc_take`;
  - `input_master(void) -> uint16_t` (0-1023, the average of 4 `fm1_adc_read(FM1_ADC_MASTER)` reads);
  - TIMER5 at priority 4 calling `fm1_input_tick`.
  - fm1-emu key ids: buttons 0-13, notes 14-40 (`14 + note`). Verify against felucca-bsp `FM1_KEYMAP`, and record the result in `docs/controls.md`.

- [ ] **Step 1: Write the failing tests:**
  - `encoder_scrolls_picker`: SELECT +2 clicks → selection 2.
  - `oct_buttons_scroll`: OCT+ → selection +1.
  - `play_starts_game`: PLAY → `stats().view == VIEW_GAME`, `stats().game == selection`.
  - `master_pot_read`: ADC ch 4 at 512 → `fmzx_master` (symbol) within ±8 of 512.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** the picker's control map (SELECT/ALGORITHM encoders and OCT-/OCT+ → `EV_UP`/`EV_DOWN`, PLAY → `EV_OK`). **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "Add FM-1 input scanning and picker controls"`

### Task 9: Running a game: frame pacing, audio, LCD updates

**Files:**
- Create: `platform/fm1/audio.{h,c}`
- Modify: `platform/fm1/main.c`, `platform/fm1/app.ld`
- Test: `host/crates/emu-gate/tests/fw_game.rs`, `fw_audio.rs`

**Interfaces:**
- Consumes: `zx_*` (zx.h), `pack_*`, `pix_*`, `lcd_sched_*`, `lcd_pump`, felucca-bsp `fm1_audio.h`.
- Produces:
  - `audio_init(void)`, `audio_start(void)`;
  - `audio_fifo_free(void) -> uint32_t`, `audio_fifo_level(void) -> uint32_t`;
  - `audio_push(const int16_t *s, uint32_t n)`;
  - `audio_set_gain(uint16_t q10)`, `audio_mute(int on)`;
  - `fm1_alnk0_irq` (RAMTEXT);
  - `game_frame(void)`: one Spectrum frame plus beeper render, push and row marking;
  - `lcd_compose_band(unsigned row0, unsigned rows)`: composes and packs one band (called by `lcd_pump`);
  - the `zx_machine` is the global `fmzx_machine` in POOL.

Behaviour:
- **Game start** (`CMD_START`):
  - `pack_extract` into a 128 KiB POOL buffer, `zx_init(m, rom_copy)` (ROM copied from flash to POOL at boot), then load by kind. A `.tap` gets `zx_tape_insert` plus `zx_tape_autostart`, and the buffer stays alive for multi-load.
  - Then `zx_beeper_init(b, 750000, 17)`, `lcd_sched_mark_all`, and the view becomes `VIEW_GAME`.
  - On any load error, show "Load failed" (`VIEW_MESSAGE`; OK returns to the picker) and don't run.
- **Pacing:**
  - The main loop runs a frame whenever `audio_fifo_level() < 1,024` samples. A frame is: apply input, `zx_run_frame`, `zx_beeper_set_level(b, zx_edges_start_level(m))`, render `zx_beeper_frame_samples` samples into a frame buffer, `audio_push`, mark the dirty LCD rows (paper rows from `zx_line_dirty`; border rows whose colour differs from the last sent), and `lcd_sched_begin_frame` with the current budget.
  - Between frames: `lcd_pump`, input poll, watchdog feed.
  - The FIFO holds 4,096 `int16` samples in POOL. `audio_push` never writes past capacity: the excess is dropped and counted in `fmzx_stats.overflows`.
- **ISR:** takes 128 samples per half block and writes the stereo Q23 words `(s * gain_q10 >> 10) << 8` to both channels.
  - When muted, or when the FIFO is empty, it writes the last sample value decayed toward 0 (`v -= v >> 4` per sample).
  - An empty FIFO counts `fmzx_stats.underruns`, only while `VIEW_GAME` or `VIEW_OSK` is running.
  - Gain: `q10 = master >= 1016 ? 1024 : master * master >> 10`, read every 10 frames.
- **Leaving or pausing:**
  - While paused, in the menu or on the key page: no frames run, audio is muted, and all Spectrum keys and Kempston bits are released (`zx_set_keys` all 0, `km_reset`).
  - `CMD_RESET` re-runs the game start from the pack. `CMD_LEAVE` returns to the picker. Every view change marks all rows.

- [ ] **Step 1: Write the failing tests:**
  - `fw_game.rs::runs_at_50hz`: start `scroll`; over 10 s emulated, `stats().frames` = 500 ± 2, `underruns == 0` after the first 200 ms, `overflows == 0`.
  - `fw_game.rs::screen_matches_host`: boot with divider 0 (budget 150,000) and RGB565; start `border`; run 3 s; at the next point where the LCD scheduler has no pending rows, the LCD equals the host composition (`pix_compose_zx` + RGB565 through `zx-sys`) of a host machine run for `stats().frames` frames from the same pack entry.
  - `fw_game.rs::load_failed_message`: a pack whose game passes the CRC but fails `zx_load_sna` (hand-built) → `VIEW_MESSAGE` and no frames.
  - `fw_audio.rs::beeper_sample_exact`: ADC 1023; start `border`; capture `audio()` for 2 s. After the first sample with `|x| > 64`, 44,100 samples equal the host render (`Beeper::render` of the host machine's per-frame edges, same start alignment).
  - `fw_audio.rs::tone_frequency`: zero crossings over 1 s → 976.6 Hz ± 1%.
  - `fw_audio.rs::silent_when_paused`: PLAY to pause → after 50 ms every sample `|x| < 16`, `underruns` unchanged.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** the device build and the emu-gate tests → PASS.
- [ ] **Step 5: Commit** `git commit -m "Run games: audio-paced frames, beeper FIFO, dirty-row LCD updates"`

### Task 10: In-game controls: key mapping, HOME menu, pause, on-screen keyboard

**Files:**
- Modify: `platform/fm1/main.c`, `docs/controls.md`
- Test: `host/crates/emu-gate/tests/fw_controls.rs`

**Interfaces:**
- Consumes: `km_apply`, `km_reset` (Task 3); `menu_event` and `menu_draw` (Task 5); `input_poll` (Task 8).

Control map in game:
- Each frame, held inputs go through the game's keymap into `zx_set_keys`/`zx_set_kempston`.
- HOME → `EV_HOME`; PLAY → `EV_PAUSE`.
- In the menu and on the key page: SELECT/ALGORITHM encoders and OCT-/OCT+ → `EV_UP`/`EV_DOWN`; PLAY → `EV_OK`; HOME → `EV_BACK`.
- In the on-screen keyboard:
  - SELECT encoder → `EV_LEFT`/`EV_RIGHT`; ALGORITHM encoder → `EV_UP`/`EV_DOWN`.
  - PLAY → `EV_OK` (the key is held while PLAY is held); SAVE → `EV_CAPS`; REC → `EV_SYM`; HOME → close.
  - Mapped note keys keep working, except those whose input is used by the keyboard.
- `CMD_DIVIDER` → `lcd_set_divider`.

- [ ] **Step 1: Write the failing tests:**
  - `note_key_reaches_kempston`: in `scroll`, holding F3 → Spectrum port 0x1F reads bit 1 (read through the `zx_machine` in RAM via the ELF symbol `fmzx_machine`).
  - `home_menu_pauses_and_releases`: hold F3, press HOME → `VIEW_MENU`, frames stop, Kempston reads 0. Resume → frames continue. Release F3 → still 0.
  - `menu_golden`: HOME menu screen equals `tests/golden/menu.png`.
  - `osk_types_key`: open the keyboard, navigate to `P`, press PLAY → keyboard row 5 bit 0 set while PLAY is held, clear after.
  - `divider_menu`: LCD speed item pressed → `fmzx_stats` shows divider 2, and the per-frame budget reported is 50,000.
  - `leave_and_restart`: menu → Back to games → picker; start the same game again → `frames` restarts from 0.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "Add in-game controls: key mapping, HOME menu, pause, on-screen keyboard"`

### Task 11: emu-gate budgets and the per-divider display frame-rate report

**Files:**
- Create: `host/crates/emu-gate/tests/fw_budget.rs`, `fw_report.rs`, `host/crates/emu-gate/src/budgets.rs`
- Modify: `BUILDING.md` (how to run, the latest report table)

**Interfaces:**
- Consumes: `Machine::cost_of`, `Machine::stats` (Task 7).

Budgets (240 MHz, 20 ms a frame = 4,800,000 instructions):
- **CPU per frame** (`game_frame` + `lcd_compose_band` × bands that frame): ≤ 3,600,000 for every built-in game and test program, measured over 10 s of play after loading.
- **Audio ISR:** `fm1_alnk0_irq` ≤ 8,000 per block.
- **RAMTEXT:** ≤ 49,152 bytes, from the ELF section `.ram_text`.
- **Image size:** image + pack ≤ 0x8DFBC. The ROM copy, the band buffers, the FIFO, the machine and the 128 KiB game buffer all fit POOL (ELF symbols, asserted).

The report is `fw_report::display_fps`. For each game in `games/games.toml`, at dividers 4, 2 and 1, with RGB444:
- boot without the RGB565 flag (the LCD content isn't checked; bytes are counted by the scheduler stats in `fmzx_stats`);
- start the game, apply its scripted input from the manifest's optional `demo = "<key>@<ms>,…"` field, and run 20 s;
- compute `fps = 50 * row_sends / pending_frames_sum`, average bytes per frame and the worst pending age.

It writes `build/fm1/fmzx-report.json` and prints a table. It asserts:
- `scroll` at divider 4 is 20-23 fps and at divider 1 ≥ 49 fps;
- every game at divider 1 is ≥ 45 fps.

- [ ] **Step 1: Write the tests.** **Step 2: Run** → they fail until the `cost_of` and stats plumbing is complete, then pass. Record any budget a game misses in the ledger rather than raising the limit.
- [ ] **Step 3: Update** BUILDING.md with the commands and the table.
- [ ] **Step 4: Commit** `git commit -m "Add firmware budgets and the per-divider display frame-rate report"`

### Task 12: The update package, install and recovery docs

**Files:**
- Create: `third_party/felucca-loader/…` (+ `VENDORED.md`), `host/crates/fmzx-pack/{inputs.toml,src/fwsc.rs,src/lz4blk.rs}`, `host/crates/fmzx-pack/tests/fwsc.rs`, `host/crates/emu-gate/tests/fw_package.rs`, `docs/install.md`, `docs/hardware-test.md`, `LICENSING.md`, `LICENSES/Apache-2.0.txt`, `LICENSES/GPL-3.0-only.txt`, `LICENSES/MPL-2.0.txt`
- Modify: `platform/fm1/CMakeLists.txt` (loader target), `tools/docker/Dockerfile` (base image by digest), `README.md`

**Interfaces:**
- Consumes: `fmzx_app.bin`, `build_pack`.
- Produces:
  - the CLI `fmzx-pack fwsc --app <app.bin> --pack <pack.bin> --loader <loader.bin> -o <out.fwsc> [--product FM-1_948] [--cache DIR] [--offline]`, plus `fmzx-pack verify <pkg.fwsc>`;
  - the CMake target `felucca_loader` → `build/fm1/third_party/felucca-loader/loader.bin`.

Package facts:
- **Container:** a ufw container (chip `AC791N`) with `flash.bin` (type 0, 0x93000 bytes) and `ota.bin` (type 100, the LZ4-wrapped loader, 4,096-byte blocks). The product identity goes in the marker slots.
- **`flash.bin`:** the 0x4000-byte head from `uboot.boot` (chip key 0x980F), then the app area. App at 0x120, then the pack at the next 4,096-byte boundary after the app, all padded with 0xFF to the slot 0x8DFBC; then `cfg_tool.bin` and `cfg/eq_cfg_hw.bin`.
- **Inputs from JieLi's AC79 SDK** (Apache-2.0), fetched to `build/ac79/` and checked by SHA-256. Base URL `https://gitee.com/Jieli-Tech/fw-AC79_AIoT_SDK/raw/AC79NN_SDK_V1.2.1_2023-12-13/cpu/wl82/tools/`:
  - `uboot.boot` `4e3b4c220dc96641cb5a723f41e68ce41d5261ae9434bb33fbd7f2c59976ded4`;
  - `cfg_tool.bin` `276579954f076886a6a7694f65dc71c034a63a2c204b76749065c0ac7b010d1b`;
  - `cfg/eq_cfg_hw.bin` `41167491bffed4651750719c973d2758adeb9021a5670d02d6a53c85ed80ea7d`.
- **The loader:** built from the vendored Felucca sources with exactly the commands of Felucca's `tools/build.py` `build_loader()`, without our toolchain flags. `fmzx-pack verify` checks its marker `FELUCCA-LOADER-1`.
- **What v1 leaves out:** our image has no USB stack. Updating away from FM-1 ZX uses the UBOOT gesture (OCT- held at power-on) and the community recovery and installer tools. The in-app SysEx update step comes with the USB plan.

- [ ] **Step 1: Vendor** the Felucca loader sources and add the `felucca_loader` target. Its `loader.bin` must be 8,216 bytes with the SHA-256 recorded in `VENDORED.md`.
- [ ] **Step 2: Write failing tests** (`fwsc.rs`):
  - `roundtrip_with_fm1_soc`: `fm1_soc::fwsc::unpack` of our package returns an app area whose bytes at 0x120 equal `app.bin`, and the pack at the 4 KiB boundary equals `pack.bin`.
  - `identity_in_markers`: the identity is `FM-1_948`.
  - `refuses_oversize`: app + pack over the slot → error.
  - `refuses_wrong_sha`: a cached input with a wrong hash → error.
  - `verify_flags_problems`.
  - `golden_package`: a SHA-256 of the package from a fixed synthetic app, pack and loader.
- [ ] **Step 3: Write the failing emu-gate test** `fw_package.rs::package_boots`: boot the `.fwsc` through `fm1_soc::stock_from_package` + `stock_boot` (Board `Fm1`) → picker golden as in Task 7.
- [ ] **Step 4: Implement**, then run all tests → PASS.
- [ ] **Step 5: Write the docs:**
  - `docs/install.md`: build, install through the community installers, the UBOOT gesture, the way back to stock through M-VAVE's workbench.
  - `docs/hardware-test.md`: the first-device checklist, each flash only with the owner's yes. The SPI pattern test at dividers 4, 2, 1, 0 (milestone 6, firmware not in this plan), then the real firmware with LCD speed cycled from the HOME menu, audio and controls checks, and reading `underruns`.
  - `LICENSING.md`: every component, its licence and how it is combined.
  - Pin the Docker base image by digest.
- [ ] **Step 6: Run** `reuse lint`, `cargo test --release --workspace` and the full emu-gate suite → green.
- [ ] **Step 7: Commit** `git commit -m "Add the .fwsc update package, Felucca's loader, install and recovery docs"`
