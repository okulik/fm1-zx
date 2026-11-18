<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# FM-1 ZX

A ZX Spectrum 48K emulator firmware for the M-VAVE FM-1 synth, written in C for its pi32v2 CPU,
with a host-side Rust test harness. The Z80 core is the redcode/Z80 library, vendored under
`third_party/`.

## Status

**First run on a real FM-1: 2026-11-17.** The firmware boots to the game picker on the device; the
rest of the [first device test](docs/hardware-test.md) is still to do, and everything else is
proven in the host tests and in the FM-1 emulator [fm1-emu](https://github.com/joakimeriksson/fm1-emu)
only. Do not install it on a unit of your own before that test has passed.

What works, in fm1-emu:

- a game picker, a HOME menu (resume, reset, on-screen Spectrum keyboard, the key map, LCD speed,
  sound, screen, back to games) and pause;
- four screen modes in the HOME menu's Screen item: Crop (240 of the 256 columns),
  Fit and Fit smooth (all 256 columns, sharp or blended; Fit smooth is the default) and Border (the whole picture with a thin
  border, scaled down);
- built-in games: freely licensed Spectrum games (Castaway, ZX Sopwith, Escape from Colony 8, ZX Dino, A World of One-Way) and test programs,
  each with its own mapping of the FM-1's notes, buttons and encoders to Spectrum keys or a
  Kempston joystick ([controls](docs/controls.md));
- FM music from the beeper, chosen in the HOME menu's Sound item (both with a 60 ms delay):
  - FM: engines (the default) recognises a game's music routine by its code, reads the notes from
    the game's memory and registers, checks them against the beeper's edges and plays them as FM.
    It knows some routines built in (Castaway, Colony 8, BASIC `BEEP`, Cave Runner, Unison); for
    your own games the pack builder learns each game's routine from its code
    ([the music](BUILDING.md#your-own-games));
  - FM: sound guesses the notes from the beeper's edges alone, where it can be sure of them;
  - each voice plays an instrument by its part in the tune: bass, lead or pad (stored at pack
    build, or worked out live from the pitches; `[game.instruments]` overrides it);
  - where FM cannot vouch for the notes (a routine it does not know) the original beeper plays (with
    Effects: Beeper; with Effects: FM the follower below plays it);
  - in FM: engines the sound effects play as FM too (a tone follows the beeper's pitch, with ticks
    and brief noise); the Effects row under Sound (FM or Beeper) turns that off. Frames the follower
    cannot follow (two voices mixed, very busy or sustained irregular sound (3 frames or more; a frame or two plays
    FM noise), or a brief 1-2 frame rejection inside a known tune) stay on the beeper;
- the Spectrum at its own speed (50 frames a second) with beeper sound, paced by the audio, and
  the screen sent to the LCD by changed rows: 50 display frames a second at LCD divider 1;
- an update package (`.fwsc`) with Felucca's update loader, and a preview package to try in
  fm1-emu's web page ([install and recovery](docs/install.md)); the page (`tools/web-demo`) adds a
  guide, labels on the FM-1 drawing and computer keys that follow the running game's key map
  ([computer keyboard](docs/controls.md#computer-keyboard-browser-page));
- your own 48K games: `tools/make-personal-pack` builds the packages from a folder of them,
  turbo and custom-loader tapes included (loaded through the tape signal on the host), and learns
  each game's music ([your own games](BUILDING.md#your-own-games));
- the UBOOT gesture: OCT- held at power-on calls the boot loader's update-mode entry
  (`fmzx_enter_uboot`); what the FM-1 does then is untested.

Not in v1: USB (so no in-app updates yet, and no adding games without a new package), tapes
played on the device itself.

## Build and test

See [BUILDING.md](BUILDING.md). In short, from the repository root:

    tools/fetch-test-data                                         # once
    tools/fm1-docker cmake --preset fm1 && tools/fm1-docker cmake --build --preset fm1
    cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release --workspace

    tools/make-packages                                           # build/fm1/fmzx.fwsc, fmzx-preview.fwsc
    tools/web-demo                                                # the release package in fm1-emu's web page

## Documents

- [Building](BUILDING.md): toolchain, host and firmware builds, the test suite, the browser demo,
  your own games and their music, measured budgets
- [Controls](docs/controls.md): the FM-1's controls in the picker, the menu and the games; key maps
- [Game pack format](docs/pack-format.md)
- [Install and recovery](docs/install.md): the packages, trying them in the browser, installing,
  the update mode, back to the stock firmware
- [First device test](docs/hardware-test.md)
- [Licensing](LICENSING.md): every component, its licence and how it is combined
