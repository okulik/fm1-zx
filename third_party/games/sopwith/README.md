<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# ZX Sopwith

A port of Sopwith (David L. Clark, 1984) to the ZX Spectrum 48K by Neil Rackett (2026).
The file is unmodified.

| | |
|---|---|
| File | `sopwith.sna` (49,179 bytes), the `sopwith.sna` asset of the release tagged `latest` |
| SHA-256 | `5a0ad0a9ad386218c7e675543ec1073e859dbf3320704cdf2c389f1891dab7b9` |
| Release | https://github.com/neilrackett/zxspectrum-sopwith/releases/download/latest/sopwith.sna (published 2026-08-17; the `latest` tag is rolling) |
| Source | https://github.com/neilrackett/zxspectrum-sopwith (the commit `latest` points at: `c1f39d265550c7092e49e7d8b3428e85c69e78e8`, 2026-08-17) |
| Licence | GPL-3.0-or-later (SPDX headers in the source; the original game is GPL-2.0-or-later) |

## Licence statements

The repository's `README.md` (copied here as `README.upstream.md`, with the repository's
`LICENSE`, the GPL 3 text) says:

> Released under the [GNU General Public License v3.0](LICENSE).
>
> The original game is distributed under the GNU GPL version 2 or later, which
> permits this.

Every source file of the port carries `Copyright (C) 2026 Neil Rackett` and an
SPDX licence line naming GPL-3.0-or-later (checked in `src/*.c`, `*.h` and `*.asm` at the
commit above). The README's credits:

> - Original game by David L. Clark, (c) 1984, 1985, 1987 BMB Compuscience
> - SDL port by [Simon Howard](https://github.com/fragglet/sdl-sopwith)
> - Atari ST and ZX Spectrum ports by [Neil Rackett](https://neilrackett.com)

The original-source repository confirms the "2 or later" claim. Its README
(https://github.com/fragglet/sdl-sopwith/blob/044d9e60c58ea11728ce47454f3135d4b1c25c2a/README.md)
says "The original author David L. Clark has kindly released the source code under the GNU GPL",
and `src/swsymbol.c` (the file the port's sprite art comes from, per `src/sprites.txt`) says:

> Copyright(C) 1984-2000 David L. Clark
> Copyright(C) 2001-2005 Simon Howard
>
> You can redistribute and/or modify this program under the terms of the
> GNU General Public License version 2 as published by the Free Software
> Foundation, or any later version.

Third-party parts of the file: the sprite art and the ground profile come from that GPL-2.0-or-later
original (`src/sprites.txt`, `src/ground.txt`); the text is drawn with the Spectrum ROM's font,
which is not part of the file; there is no loading screen, no music and no font data. The
file is built with z88dk, so it contains z88dk's start-up code and a few runtime routines
(for example the ZX0 decompressor for the ground). z88dk is distributed under the Clarified
Artistic License (https://github.com/z88dk/z88dk/blob/master/LICENSE), which permits
distributing programs built with it; the author's repository does not list which routines the file holds.

Attribution: ZX Sopwith by Neil Rackett, https://github.com/neilrackett/zxspectrum-sopwith,
after Sopwith by David L. Clark and the SDL port by Simon Howard.

## Corresponding Source

`zxspectrum-sopwith-c1f39d2.tar.gz` (64,760 bytes, SHA-256
`c517f32162db4205d3871966673ba98c1996cd0b42688496e7f65dcd6c7abf52`) is the Corresponding Source of
`sopwith.sna` (GPL 3 section 6): `git archive --prefix=zxspectrum-sopwith-c1f39d2/
c1f39d265550c7092e49e7d8b3428e85c69e78e8 | gzip -n` of a clone of the repository. Provenance: the
release's `latest` tag ref points at that commit (committed 2026-08-17 11:43 UTC; the release assets were
uploaded 2026-08-17 18:55 UTC), and the next commit, `1aacf55` ("Fixed intermittent key input
issue on menu", 2026-08-18), is later than the release and is not in this file.

## ZX0 notice

The ground profile is compressed with ZX0 and the file holds its decompressor, by Einar Saukas
(BSD-3-Clause, https://github.com/einar-saukas/ZX0/blob/main/LICENSE):

> BSD 3-Clause License
>
> Copyright (c) 2021, Einar Saukas
> All rights reserved.
>
> Redistribution and use in source and binary forms, with or without
> modification, are permitted provided that the following conditions are met:
>
> 1. Redistributions of source code must retain the above copyright notice, this
>    list of conditions and the following disclaimer.
>
> 2. Redistributions in binary form must reproduce the above copyright notice,
>    this list of conditions and the following disclaimer in the documentation
>    and/or other materials provided with the distribution.
>
> 3. Neither the name of the copyright holder nor the names of its
>    contributors may be used to endorse or promote products derived from
>    this software without specific prior written permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
> AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
> IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
> DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
> FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
> DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
> SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
> CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
> OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
> OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

z88dk's Clarified Artistic License does not extend to the compiled output of its library.

## The release and its other files

The release also carries `sopwith.tap` (35,056 bytes) and `sopwith.z80` (28,753 bytes). The pack
uses the `.sna`, a plain 48K snapshot (a 27-byte header and 49,152 bytes of RAM), so no tape
loader is involved.

## Loader check

`Machine::load_sna` accepts the file, and after 300 frames the title screen is up: "ZX SOPWITH",
the key list, "N NOVICE PLAYER", "E EXPERT PLAYER", "C VERSUS THE COMPUTER", "HI-SCORE 0" and the
GPL notice at the bottom.

## Controls

At the title N starts a novice game, E an expert game and C a game against the computer's planes
(a Kempston fire button starts a novice game too). In the air Q and A pull the plane up and down,
O and P throttle down and up, SPACE fires the machine gun, F flips the plane over, B drops a
bomb, H flies home, M switches the sound and 1 abandons the flight. A Kempston joystick flies and
fires (up and down pitch, left and right throttle, fire shoots). Land on your own runway to
refuel and rearm.

The FM-1 keeps its default notes (F3 left, F#3 and G#3 up, G3 down, A3 right, C5-G5 fire, F4 = N
for the novice start) and adds:

| FM-1 input | Spectrum key | Does |
|---|---|---|
| A#3 | F | flip |
| B3 | B | bomb |
| C4 | H | fly home |
| C#4 | C | start against the computer |
| D#4 | E | start an expert game |
| LFO | M | sound on and off |
| OCT- | 1 | abandon the flight (the default) |

The pack's manifest is `games/games.toml`.
