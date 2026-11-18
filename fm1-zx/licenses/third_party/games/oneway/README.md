<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# A World of One-Way

A sliding puzzle game for the ZX Spectrum 48K by Serdjuk (Vitaly, GitHub user Serdjuk), for the ASM 2021
competition, version 0.41 (June 2021). The file is unmodified.

| | |
|---|---|
| File | `Wow.tap` (8,402 bytes), from `Wow.tap` in the release archive; the same bytes are in the repository |
| SHA-256 | `a871d03678d69f1a16bf67c8eeee2ba001f6d5be92b603cec86699e7c3581348` |
| Release archive | https://github.com/Serdjuk/A-World-of-One-way/releases/download/0.41/A-World-of-One-way_0.41.zip (7,543 bytes, SHA-256 `7b9c1b47e9ff358050be91cf7bb4775e1ed45f555bad1393473283447b7adb48`) |
| Source | https://github.com/Serdjuk/A-World-of-One-way at commit `aecdf334f8c97eac2cd54052b73b0da06a38411e` (`Wow.tap` last changed in `9c5e3142`, "fix password input", 2021-06-15, the release's commit); the release's `info.txt` names the same repository under its former account, github.com/vito-Z80 |
| Licence | Apache-2.0 (`LICENSE` in the repository, copied here as `LICENSE.release`, SHA-256 `50e6751797c50dedd75ef1b8a0d9e42f5f8472e9fbce91f34718e9f97b0c780a`), with the beeper engine under MIT (`COPYING.beeper`, SHA-256 `90085acc982c755b2408aae85fa65fdd5ee1b27ac11c6ac6cabca201c99c1651`): SPDX `Apache-2.0 AND MIT` |

The release archive's `info.txt` is copied as `README.release.txt` (SHA-256
`e686d913b5d97331b7df3e4e553cb5b5c2113099161ed9efd28eee6d2ff912a3`) and the repository's `README.md` as
`README.release.md` (SHA-256 `f0a33537a89bc7f19d4985f1156e75e39e1a5c68c542ab65668e5fc61a15d22e`).

## Licence statements

The repository's `LICENSE` is the Apache License, Version 2.0 (GitHub's licence detection reports
`Apache-2.0`); the author did not fill in the appendix's copyright line. All 62 commits are by the one
author. `LICENSE` says:

> Apache License
> Version 2.0, January 2004
> [...]
> 2. Grant of Copyright License. Subject to the terms and conditions of this License, each Contributor
> hereby grants to You a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable
> copyright license to reproduce, prepare Derivative Works of, publicly display, publicly perform,
> sublicense, and distribute the Work and such Derivative Works in Source or Object form.

Redistribution in Object form needs the licence text to go with it (section 4(a)): it is
`LICENSE.release` here, and `LICENSING.md` carries
the attribution.

Beeper engine: the sound player in the game (`code/audio/soundPlayer.asm` in the repository, with
`code/audio/sfxed-1.0.0-win/COPYING` copied here as `COPYING.beeper`) is Juan J. Martínez's beeper engine
from his sfxed tool. The notice, which has to stay with the file, is:

> Beeper engine
> Copyright (C) 2021 by Juan J. Martinez <jjm@usebox.net>
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
> THE SOFTWARE.

Third-party parts checked: the tape is built from `Wow.asm` with `MACHINE = 9` (the 9 KB version with a
BASIC loader). That build reads the Spectrum ROM's character set through its address (`ld bc,#3D00 - 256`
in `charAddr`) and does not include `font/font.SpecCHR`, which only the 16 KB cartridge build
(`code/rom.asm`) embeds, so no font is copied into the file. The title screen is drawn by the game's own
code (`code/title2.asm`, a path animation with the text "Concurso juegos ASM 2021"), there is no loading
screen, and the menu and shop texts are in the code. The sprites are the author's own art
(`sprites/gif`, `sprites/bmp`, `sprites/psd` in the repository, the game's own `sprites/storage.asm`); the
levels are the author's own Tiled maps. The repository's `utils/` folder holds ZX0 and aPLib decoders and
packer programs by other authors, but `includes.asm` includes only `utils/utils.asm` (the author's own
helper routines), and no compressed data or decoder is in the tape. The only third-party code in the
tape is the beeper engine above. `info.txt` credits a YouTube channel only for the description text,
which is not in the game.

Attribution: A World of One-Way by Serdjuk, https://github.com/Serdjuk/A-World-of-One-way, with the
beeper engine by Juan J. Martínez, https://www.usebox.net/jjm/.

## Tape blocks

The standard ROM loader reads it (checked with `Machine::tape_insert`: all three blocks, flags and
checksums valid), and in the host core the title screen appears within 300 frames of `tape_autostart`.
The third block is a data block with no payload (flag and checksum only, 2 bytes); the core accepts it:

| # | Flag | Length | Content |
|---|---|---|---|
| 0 | 00 (header) | 19 | Program `Wow`, 8,373 bytes, autostart line 0 |
| 1 | FF (data) | 8,375 | BASIC program with the machine code in a REM line (`RANDOMIZE USR 23774`) |
| 2 | FF (data) | 2 | empty data block (flag and checksum) |

## Controls

Q/A/O/P (up, down, left, right), or Sinclair joystick 1 or 2 (keys 1-4 and 6-9); each move slides every
character until it meets a wall. CAPS SHIFT + ENTER restarts the level (losing a life) or returns to
the menu. Between levels the shop takes 1 (an extra life) and 2 (a password for the next level). In the
menu 1 starts and 2 continues with a password. The game reads the keyboard only. The pack maps them
onto O F3, P A3, Q F#3/G#3, A G3, OCT- (1) and OCT+ (2), and SEQ + C4 (CAPS SHIFT + ENTER); see
`games/games.toml`.
