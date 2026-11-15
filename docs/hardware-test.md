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
- [ ] The picker shows the nine entries (the five built-in games and the four test programs) in the right colours (RGB444), all on black: the
      "FM-1 ZX" title in bright yellow, the names and the "1/9" counter in white, the selected
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
- [ ] Castaway's effects sound right, at every LCD speed.
- [ ] ZX Sopwith's effects (engine, shots, bombs, crashes) sound right, with no crackle, at every LCD speed.
- [ ] Escape from Colony 8's effects and title tune sound right, with no crackle, at every LCD speed.
- [ ] ZX Dino's jump and crash sounds are clean and in time with the screen; nothing crackles while the dinosaur runs.
- [ ] A World of One-Way's music plays without crackle, and its sound effects keep in time with the moves.
- [ ] FM music: in the HOME menu set Sound to "FM: sound" and resume. Castaway's title and Escape from Colony 8's title music play as FM voices, in tune and in time, with no wrong notes; everything is about 60 ms later than on the beeper.
- [ ] Switching Sound between the three choices several times while music plays: a brief fade and gap when turning FM on, no clicks or pops in either direction.
- [ ] In FM: sound, the effects in ZX Dino and ZX Sopwith stay on the beeper (their usual sound, only delayed), with no crackle.
- [ ] FM: engines: set Sound to "FM: engines" and start Castaway: its title tune plays as FM with two voices, in tune.
- [ ] The Sound choice survives Reset game and starting another game from the picker (it is back to Beeper after power-off).
- [ ] Screen modes: in Castaway, set Screen to each choice in turn and resume. Crop looks as before. Fit shows the whole width, sharp, with no colour other than the Spectrum's. Fit smooth shows the whole width with soft column edges. Border shows the whole picture smaller with a thin border at the sides, the border colour above and below, and no stretched or squashed shapes. In every choice the picture is complete at once after leaving the menu, with no leftover rows from the previous choice.
- [ ] Border with a changing border: set Screen to Border and start the Tape multi-load test; when its border turns from blue to green, the green fills the side columns and the rows above and below the picture at once, with no leftover blue row.
- [ ] The HOME menu, the on-screen keyboard and PAUSED look the same in Border and Fit smooth as in Crop, and the game rows around the keyboard keep the chosen mode.
- [ ] The Screen choice survives Reset game and starting another game from the picker (it is back to Crop after power-off).
- [ ] Beeper tune test ("Beeper tune test"): a clean rising C major scale from C5 to C6, repeated, every note the same length and none cut short or clicking at the changes between notes; the screen shows eight static bars, which do not change with the notes.

ROM BEEP (BASIC `BEEP`) has no device check: the pack has no BASIC entry, so the host and emulator tests cover it with a generated BASIC BEEP tape (`beep_tap`).

### Controls

- [ ] Picker: SELECT and ALGORITHM turns and OCT-/OCT+ move the selection, PLAY starts a game.
- [ ] In a game: the notes and buttons act as the game's key map says (`docs/controls.md`,
      `games/games.toml`); PLAY pauses and resumes; HOME opens the menu; Reset game, Keyboard,
      Key map and Back to games work.
- [ ] On-screen keyboard: the cursor moves, PLAY types, SAVE and REC give CAPS and SYMBOL SHIFT.
- [ ] Encoders: no skipped or doubled detents when turned slowly and quickly.
- [ ] ZX Sopwith: F4 (N) starts a game; F#3/G#3 and G3 pull the plane up and down, A3 and F3 raise and lower the throttle, C5 fires; the status bar and the plane move without tearing.
- [ ] Escape from Colony 8: OCT- chooses keyboard control at the title; F3/A3 walk left and right, F#3 and G3 go up and down, C5 shoots, and a held C5 charges a missile that leaves when released; the room scrolls without shifted rows.
- [ ] ZX Dino: any button starts a run; F#3 or C5 jumps the cacti, G3 ducks under a bird; after a crash any key starts again.
- [ ] A World of One-Way: OCT- starts from the menu, F3/A3/F#3/G3 slide the heroes left, right, up and down to the next wall, and the picture stays clean while they move.
- [ ] Tape multi-load test ("Tape multi-load test"): about 4 to 5 seconds after you start it (the ROM autostarts the tape, then the program waits two seconds) the border turns green and a screen image appears from the third block; Reset game in the HOME menu loads the tape again from its first block and reaches green again.

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
