<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# FM-1 ZX

A ZX Spectrum 48K emulator firmware for the M-VAVE FM-1 synth, written in C for its pi32v2 CPU,
with a host-side Rust test harness. The Z80 core is the redcode/Z80 library, vendored under
`third_party/`. See the [design spec](docs/superpowers/specs/2026-11-11-zx48k-fm1-design.md).

## Status

**Not yet run on a real FM-1.** The firmware is complete for v1 and proven in the host tests and
in the FM-1 emulator [fm1-emu](https://github.com/joakimeriksson/fm1-emu) only. Do not install it
on a unit before the [first device test](docs/hardware-test.md) has passed.

What works, in fm1-emu:

- a game picker, a HOME menu (resume, reset, on-screen Spectrum keyboard, the key map, LCD speed,
  sound, back to games) and pause;
- built-in games: freely licensed Spectrum games (Castaway, ZX Sopwith, Escape from Colony 8, ZX Dino, A World of One-Way) and test programs,
  each with its own mapping of the FM-1's notes, buttons and encoders to Spectrum keys or a
  Kempston joystick ([controls](docs/controls.md));
- FM music from the beeper: a Sound item in the HOME menu re-synthesises beeper music as FM voices where it can be sure of the notes (method A, a 60 ms delay); FM: engines (method B) recognises known music routines (Castaway, Colony 8, BASIC `BEEP`, Cave Runner, Unison), reads their notes from memory, checks them against the beeper and plays them as FM, the same 60 ms delay; Beeper stays the default;
- the Spectrum at its own speed (50 frames a second) with beeper sound, paced by the audio, and
  the screen sent to the LCD by changed rows: 50 display frames a second at LCD divider 1;
- an update package (`.fwsc`) with Felucca's update loader, and a preview package to try in
  fm1-emu's web page ([install and recovery](docs/install.md));
- your own 48K games: `tools/make-personal-pack` builds the packages from a folder of them
  ([your own games](BUILDING.md#your-own-games));
- the UBOOT gesture: OCT- held at power-on calls the boot loader's update-mode entry
  (`fmzx_enter_uboot`); what the FM-1 does then is untested.

Not in v1: USB (so no in-app updates yet, and no adding games without a new package).

## Build and test

See [BUILDING.md](BUILDING.md). In short, from the repository root:

    tools/fetch-test-data                                         # once
    tools/fm1-docker cmake --preset fm1 && tools/fm1-docker cmake --build --preset fm1
    cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release --workspace

    tools/make-packages                                           # build/fm1/fmzx.fwsc, fmzx-preview.fwsc
    tools/web-demo                                                # the release package in fm1-emu's web page

## Documents

- [Building](BUILDING.md): toolchain, host and firmware builds, the test suite, measured budgets
- [Controls](docs/controls.md): the FM-1's controls in the picker, the menu and the games; key maps
- [Game pack format](docs/pack-format.md)
- [Install and recovery](docs/install.md): the packages, trying them in the browser, installing,
  the update mode, back to the stock firmware
- [First device test](docs/hardware-test.md)
- [Licensing](LICENSING.md): every component, its licence and how it is combined
