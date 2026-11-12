<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# First device test

The checklist for the day an FM-1 is available. Until then everything is proven in the host
tests and fm1-emu only.

**The device rule.** Every step that flashes the FM-1, sends it SysEx or opens its MIDI ports
is marked **[owner's yes]** and is done only with the owner's explicit yes for that step, given
at that moment. A yes for one step is not a yes for the next. Nothing here is automated.

Write down the results (date, firmware build, the FM-1's stock firmware version) in
`BUILDING.md`'s measured budgets, and keep the packages that were installed.

## 0. Before anything is written

- [ ] Note the stock firmware version the unit came with (shown at start-up or in its menu).
- [ ] Have M-VAVE's stock package (`FM-1.fwsc`) and M-VAVE's workbench at hand, and a community
      recovery tool (FM-1-transporter or M-flasher) installed and tried on the computer.
- [ ] Practise the hardware update mode with the stock firmware: hold OCT- and OCT+ together at
      power-on for about five seconds and check that the computer sees the boot loader on USB
      (`docs/install.md`). Switch off and on again: the stock firmware starts. This writes
      nothing, but it opens the USB connection: **[owner's yes]**.
- [ ] If the recovery tool can read the whole flash (FM-1-transporter reads the 1 MiB in a few
      seconds), make a backup of the unit's flash, including its USR area (the factory patch
      bank) **[owner's yes]**.
- [ ] Run `tools/make-packages` and `fmzx-pack verify` on both packages; run the whole test
      suite (`BUILDING.md`). Record the package SHA-256s.

## 1. SPI pattern test (milestone 6; firmware not in this plan)

A small test firmware that only draws known patterns (stripes, a grid, colour bars, single-pixel
lines) on the LCD at SPI dividers 4, 2, 1 and 0, a few seconds each, with the divider shown on
screen. It is not part of this plan and is written first when the device arrives.

- [ ] Install the pattern test **[owner's yes]**.
- [ ] For each divider (4, 2, 1, 0: 12, 20, 30, 60 MHz) the owner checks by eye: no torn or
      shifted lines, no wrong colours, no flicker, the whole 240×240 area drawn.
- [ ] Note the fastest clean divider. FM-1 ZX boots at divider 4 (`FMZX_LCD_DIVIDER`); if a
      faster one is clean, it can become the default.
- [ ] Back to stock with the update mode and M-VAVE's workbench, as in `docs/install.md`
      **[owner's yes]**. This is the first test of the way back; do not go on if it fails.

## 2. FM-1 ZX itself

- [ ] Install `build/fm1/fmzx.fwsc` from the stock firmware through a community installer
      (`docs/install.md`) **[owner's yes]**. The FM-1 restarts into the game picker.
- [ ] The picker shows the four built-in games in the right colours (RGB444), all on black: the
      "FM-1 ZX" title in bright yellow, the names and the "1/4" counter in white, the selected
      name in black on a bright-cyan bar; no scrambled pixels.

### LCD speed

- [ ] Start the scroll test ("Scroll test"). Open the HOME menu, pick "LCD speed" and step it
      through 4, 2, 1 and 0 (each press steps to the next; the item shows the current value).
      At each, resume and watch for a clean picture: the scroller should get smoother as the
      divider drops (in fm1-emu about 21, 36 and 50 frames a second at 4, 2 and 1), with no
      tearing, shifted rows or stray colours.
- [ ] Note the fastest divider with a clean picture, and whether it matches step 1.
- [ ] The border test ("Border and tone test"): border stripes stay steady.

### Audio

- [ ] The border test plays a steady tone (about 976 Hz): no crackle, no dropouts, no drift in
      pitch over a minute.
- [ ] MASTER changes the volume smoothly, down to silence.
- [ ] Moon Caves's title music and Castaway's effects sound right, at every LCD speed.

### Controls

- [ ] Picker: SELECT and ALGORITHM turns and OCT-/OCT+ move the selection, PLAY starts a game.
- [ ] In a game: the notes and buttons act as the game's key map says (`docs/controls.md`,
      `games/games.toml`); PLAY pauses and resumes; HOME opens the menu; Reset game, Keyboard,
      Key map and Back to games work.
- [ ] On-screen keyboard: the cursor moves, PLAY types, SAVE and REC give CAPS and SYMBOL SHIFT.
- [ ] Encoders: no skipped or doubled detents when turned slowly and quickly.

### Underruns

`fmzx_stats.underruns` counts the audio FIFO running dry while a game runs. On the device it
cannot be read without a debugger: FM-1 ZX has no USB stack and no screen for it yet. What to
observe instead is its effect: an underrun is a short gap in the sound, heard as a **crackle**
or click.

- [ ] Play each game for a few minutes at the fastest clean LCD speed with the volume up,
      listening for crackles, and again at divider 4. Note any game, screen or speed that
      crackles: that is where the CPU budget runs short on the real hardware (emu-gate's
      budgets assume 240 MHz).

### The way back

- [ ] UBOOT gesture: switch off; hold OCT- while switching on and for one second, with the FM-1
      connected by USB **[owner's yes]** (it opens the USB connection, as in section 0). Expected,
      untested: the screen shows no picture and the computer sees the boot loader on USB.
- [ ] A short OCT- tap at power-on: FM-1 ZX starts normally.
- [ ] From the update mode, back to stock with M-VAVE's workbench (or a community recovery
      tool) **[owner's yes]**. The stock firmware starts and plays its patches.
- [ ] Install FM-1 ZX again from stock **[owner's yes]**, then use the UBOOT gesture and the
      community installer to install it once more over itself **[owner's yes]**: this is the
      update path for later versions.

Only when all of section 2 has passed may `docs/install.md` drop its warning and a package be
offered to anyone else.
