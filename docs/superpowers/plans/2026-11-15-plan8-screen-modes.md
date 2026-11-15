# Screen Modes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Screen item in the HOME menu that cycles Crop, Fit, Fit smooth and Border, so the Spectrum picture can be shown cropped (as today), fitted to the 240-pixel width, or scaled with a thin border.

**Architecture:**
- `app/pixels.c` gains one entry point that composes an LCD row straight to panel bytes in a given mode.
  - Crop goes through today's palette-index path, so it is byte-identical.
  - Fit drops one column in 16.
  - Fit smooth and Border blend in 8-bit RGB and reduce to the panel format.
- The firmware keeps the choice in `fmzx_screen_mode`, as it does for Sound.
- The firmware maps dirty Spectrum lines and border changes to LCD rows per mode.

**Tech Stack:** C (app/, integer only), the firmware in platform/fm1, Rust host tests (zx-sys, zx-tests, emu-gate on fm1-emu).

**Spec:** `docs/superpowers/specs/2026-11-15-screen-modes-design.md`

## Global Constraints

- Commits: a single line with no trailers (no Co-Authored-By). Never name other projects in files or commits.
- Device rule: never flash a device, send SysEx or open MIDI ports.
- C rules: integer only, no allocation, `-Wall -Wextra -Werror`. Host and device produce identical bytes, and the host tests use the same `app/pixels.c`.
- **Crop is byte-identical to today**: the same panel bytes as `pix_compose_zx` + `pix_pack`. Every existing golden and test passes unchanged, including `host/crates/emu-gate/tests/golden/*.png` and the zx-tests hashes.
- **Menu.** The Screen item cycles Crop → Fit → Fit smooth → Border → Crop.
  - The labels are exactly `Screen: Crop`, `Screen: Fit`, `Screen: Fit smooth` and `Screen: Border`.
  - The default is Crop.
  - The choice holds through Reset game, Back to games and starting another game, and is not saved.
  - The item sits directly after Sound.
- **Fit**: output column x shows source column `x + x/15` (x = 0…239), so source columns 15, 31, …, 255 are dropped. Rows and border are as in Crop. Only Spectrum colours appear.
- **Fit smooth**: output column x covers the source span [16x/15, 16(x+1)/15). Each output pixel is the area-weighted average of the 8-bit RGB of the source columns it overlaps (at most two). Weights are in sixteenths of the output pixel's span, which makes them exact integers, and the weights of a pixel sum to 16. Rows and border are as in Crop.
- **Border**:
  - The source region is frame columns paper −8 … paper +263, which is 272 columns with 8 border pixels each side, by frame lines 40…279 (240 lines, the lines today's LCD rows show).
  - It is scaled uniformly by 240/272 to 240 × 212 with an area filter, in 8-bit RGB, and placed at LCD rows 14…225.
  - LCD rows 0…13 take the border colour of frame line 40; rows 226…239 take that of frame line 279.
  - A border pixel's colour is its frame line's border colour (`zx_border_line`).
- **Colour reduction** of blended values: RGB444 by `>> 4` of the rounded 8-bit value (the release panel), and RGB565 by `>> 3` / `>> 2` (the preview panel), packed as `pix_pack` packs.
- **Palette.** The 8-bit RGB of index i is `pixels.c`'s `R8/G8/B8`: 0xD7 lit, 0xFF bright.
- **Budgets.**
  - Every game's worst frame stays within `fw_budget`'s cap (3,600,000).
  - RAMTEXT ≤ 49,152; it is 48,156 today.
  - If the new modes do not fit in RAMTEXT, Crop stays there and the other modes' filters run from flash. RAMTEXT/XIP calls go through the existing `ram_calls` table or function pointers, per `platform/fm1/app.ld`'s reach rules.
- **Commands:**
  - host: `cd host && cargo test --workspace --exclude emu-gate`
  - device builds: `tools/fm1-docker cmake --build --preset fm1 && tools/fm1-docker cmake --build --preset fm1-preview`, then `FMZX_SKIP_DEVICE_BUILD=1 tools/make-packages --offline`
  - firmware: `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate`
  - `reuse lint`
  - `tools/web-demo --check --skip-firmware`

## Ruling recorded in the plan

The spec asks for every Screen mode × Sound mode in `fw_budget`. Screen work (row composition) and sound work (the mixer) are independent and add up, so the budget runs as follows:
- every game in each Screen mode under FM: engines, the heaviest Sound mode;
- every game in Crop under all three Sound modes, as today.

That is 6 runs per game instead of 12. Cost if wrong: a cross-effect between screen and sound work goes unmeasured, though nothing shares state between them.

## Interfaces (fixed)

```c
/* app/pixels.h */
enum { PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH, PIX_BORDER, PIX_MODE_COUNT };
/* LCD row `row` (0-239) of the game picture in `mode`, as panel bytes (pix_row_bytes(fmt) of them).
 * crop_x is used by Crop only. Crop's bytes equal pix_compose_zx + pix_pack. */
void pix_compose_row(const zx_machine *m, unsigned row, unsigned mode, unsigned crop_x, int fmt, uint8_t *out);
/* Marks in dirty[LCD_ROWS/8] (bit r&7 of byte r>>3) every LCD row whose bytes in `mode` depend on
 * paper line y (0-191). */
void pix_mark_paper_line(unsigned mode, unsigned y, uint8_t dirty[LCD_ROWS / 8u]);
/* Marks every LCD row whose bytes in `mode` depend on the border colour of frame line `line`. */
void pix_mark_border_line(unsigned mode, unsigned line, uint8_t dirty[LCD_ROWS / 8u]);
```

Rust (`zx-sys`):
- `Machine::compose_row_mode(&self, row: usize, mode: u32, crop_x: usize, fmt: i32) -> Vec<u8>`
- `pix_mark_paper_line(mode, y) -> [u8; 30]`
- `pix_mark_border_line(mode, line) -> [u8; 30]`
- the constants `PIX_CROP`, `PIX_FIT`, `PIX_FIT_SMOOTH`, `PIX_BORDER`

Firmware:
- `uint32_t fmzx_screen_mode`, read by emu-gate through its symbol, as `fmzx_sound_mode` is.
- The menu's Screen item index is the Sound item's index + 1. Items after it move down one; the emu-gate constants (`SOUND_ITEM` and others in `host/crates/emu-gate/src/budgets.rs`) are updated to match.

## Review Focus

1. **A border colour change mid-game in Border mode**, e.g. a loader-stripe effect or a flashing border. The LCD must show the new border in every row it feeds, including the blended side columns. Owned by Task 2 (`screen_border_change_redraws`).
2. **Switching modes while a game is paused or running.** The whole panel must redraw in the new mode at once, with no stale rows from the old mode. Owned by Task 2 (`screen_switch_redraws_all`).
3. **The on-screen keyboard and the HOME menu over the game in Border or Fit smooth.** The overlays must look exactly as today, and the game rows around them must compose in the chosen mode. Owned by Task 2 (`screen_overlays_unchanged`).
4. **Per-game `crop_x` with Fit, Fit smooth or Border.** crop_x is ignored in those modes and still applies to Crop. Owned by Task 1 (`modes_ignore_crop_x`).
5. **Colours at the RGB444 boundary.** A flat area in any blended mode must give exactly the palette colour's panel bytes, with no off-by-one from rounding. Owned by Task 1 (`blend_flat_is_exact`).

---

### Task 1: Row composition in every mode, with host tests

**Files:**
- Modify: `app/pixels.c`, `app/pixels.h`, `host/crates/zx-sys/src/lib.rs`
- Test: `host/crates/zx-tests/tests/app_pixels.rs`, with golden hashes appended to `host/crates/zx-tests/tests/golden/hashes.txt` in its existing format

**Interfaces:** Produces the C functions and the zx-sys wrappers in the Interfaces block.

- [ ] **Step 1: Write the failing tests** in `app_pixels.rs`:
  - `crop_mode_is_today`: for `scroll_sna()` and `border_sna()` after 50 frames, for every row and both formats, and for crop_x 0, 8 and 16, `compose_row_mode(row, PIX_CROP, crop_x, fmt)` equals `pack(fmt, compose_row(row, crop_x))`.
  - `fit_drops_every_16th`: with a paper line whose column c has palette index `c % 16` (poked directly), Fit's output column x has index `(x + x/15) % 16`, and columns 15, 31, … 255 never appear.
  - `fit_smooth_weights_sum`: for each x, the weights of output pixel x (in sixteenths of its span, at most two source columns) sum to 16 (test through a helper, or by composing a line with one lit column at a time and checking the reconstructed weights).
  - `blend_flat_is_exact`: a paper filled with a single colour (each of the 16 palette indices in turn) gives, in Fit smooth and Border and in both formats, rows whose every pixel equals that colour's `pix_pack` bytes. Border's border-only rows are exact too.
  - `modes_ignore_crop_x`: Fit, Fit smooth and Border give the same bytes for crop_x 0 and 16.
  - `border_geometry`: in Border, rows 0-13 equal the border colour of frame line 40, and rows 226-239 equal that of line 279. Change the border colour so that only frame lines 40-63 differ from 256-279 (the frame's top border lines; use `border_sna` or OUT timing as `compose_border_rows` does), and check that rows 14-35 show the change and row 225 does not.
  - `mark_rows`:
    - Crop, Fit and Fit smooth: `pix_mark_paper_line(m, y)` marks exactly row 24 + y. `pix_mark_border_line(m, line)` marks exactly row line − 40 for lines 40-63 and 256-279, and nothing for paper lines (64-255) or lines outside 40-279.
    - Border: every row whose composed bytes change when paper line y changes is marked, and no other row. Verify this by composing all rows before and after changing one paper line, for each y. Do the same for border lines 40-279.
  - `golden_modes`: the hash of every composed row of `scroll_sna()` and `border_sna()` at frame 50, in each mode and format, recorded in `hashes.txt`.
- [ ] **Step 2: Run** `cd host && cargo test -p zx-tests --test app_pixels`. Expected: FAIL (the functions are undefined).
- [ ] **Step 3: Implement** `pix_compose_row`, `pix_mark_paper_line` and `pix_mark_border_line` in `app/pixels.c`, plus the zx-sys wrappers.
  - Crop calls the existing `pix_compose_zx` + `pix_pack`.
  - Precompute the Fit smooth and Border weight tables as `static const` arrays (generated by hand or by a small comment-documented formula), with no runtime division in the per-pixel loop.
  - Keep `pix_compose_zx` and `pix_pack` unchanged and exported.
- [ ] **Step 4: Run** the host command. Expected: all pass, with the existing hashes unchanged.
- [ ] **Step 5: Commit** `git commit -m "Pixels: LCD rows composed in Crop, Fit, Fit smooth and Border, with dirty-row maps"`

### Task 2: The Screen item in the firmware, tests, budgets and docs

**Files:**
- Modify:
  - `app/menu.c`, `app/menu.h`: the Screen item and its `CMD_SCREEN`.
  - `platform/fm1/main.c`: `fmzx_screen_mode`; `lcd_compose_band` uses `pix_compose_row`; `game_step` marks rows with `pix_mark_paper_line` and `pix_mark_border_line`; a full redraw (`lcd_mark_all`) on a mode switch; RAMTEXT placement as needed.
  - `platform/fm1/CMakeLists.txt` and `ramcalls.h`, if the placement needs it.
  - `host/crates/emu-gate/src/budgets.rs`: menu item indices and the screen-mode helpers.
  - `docs/controls.md`: the Screen row in the HOME-menu table.
  - `docs/hardware-test.md`: a manual check of each mode.
  - `BUILDING.md`: the budget table, with screen modes.
  - `README.md`: one line.
- Test:
  - `host/crates/emu-gate/tests/fw_controls.rs` (menu cycle and hold);
  - `host/crates/emu-gate/tests/fw_screen.rs`, new (the LCD per mode);
  - `host/crates/emu-gate/tests/fw_budget.rs` (screen modes in the loop);
  - existing tests that count menu items, updated mechanically only.

**Interfaces:** Consumes Task 1's functions. Produces `fmzx_screen_mode` and the menu item.

- [ ] **Step 1: Write the failing tests:**
  - `screen_menu_cycles_four` (fw_controls): from HOME, pressing PLAY on Screen four times shows the labels `Screen: Fit`, `Screen: Fit smooth`, `Screen: Border` and `Screen: Crop` in turn, checked on the LCD as the Sound item tests do. `fmzx_screen_mode` follows (1, 2, 3, 0).
  - `screen_choice_holds` (fw_controls): the choice holds through Reset game, Back to games and starting another game, as `sound_choice_holds` does.
  - `screen_modes_match_host` (fw_screen): for Castaway's title at 6 s and for `border.sna`, in each mode, the fm1-emu LCD equals the host composition `compose_row_mode` of the same machine state, for every row, in the preview format the emulator shows.
  - `screen_border_change_redraws` (fw_screen): in Border with `border.sna` (its border changes every frame), every LCD row equals the host composition after each of 10 consecutive frames. There are no stale rows.
  - `screen_switch_redraws_all` (fw_screen): switching from Crop to Border and on to Fit smooth during Castaway's title gives the new mode's full composition on the next LCD frame.
  - `screen_overlays_unchanged` (fw_screen): the HOME menu and the on-screen keyboard over a game in Border look exactly as in Crop. The overlay rows compare equal, and the game rows outside them match the mode's host composition.
  - `fw_budget`: the runs listed in the plan's ruling (each Screen mode under FM: engines for every game, including noise.sna and the fixture pack, plus Crop under all Sound modes). Every worst frame is within the cap, and RAMTEXT ≤ 49,152.
- [ ] **Step 2: Run** the device builds, repack, then the firmware command. Expected: FAIL on the new tests.
- [ ] **Step 3: Implement** the menu item, the main.c wiring, the redraw on switch and the placement.
  - Measure RAMTEXT. If the new modes do not fit, move the Fit, Fit smooth and Border filters to flash and call them through the reach-safe path.
  - Record the placement in the report.
- [ ] **Step 4: Run:**
  - the firmware command (all pass);
  - the host command;
  - `reuse lint`;
  - `tools/web-demo --check --skip-firmware`.

  Record in the report: the worst frame per Screen mode, RAMTEXT, and the audio ISR figure.
- [ ] **Step 5: Commit** `git commit -m "Firmware: the Screen item (Crop, Fit, Fit smooth, Border), tests and budgets"`
