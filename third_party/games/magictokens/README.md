<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Magic Tokens

A match-3 puzzle game for the ZX Spectrum 48K by Shiru (2010, for the "Your
Game-4" competition). The file is unmodified.

| | |
|---|---|
| File | `magic_tokens.tap` (21,004 bytes), from `magic_tokens/magic_tokens.tap` in the release archive |
| SHA-256 | `d51d1ea9025ee71e0ecd3de957d6ac279ab68aa38404cc0426be0df88b3dba5c` |
| Release archive | http://shiru.untergrund.net/files/zx/magic_tokens.zip (SHA-256 `697e7deafd17d3d841f5bba67665d025c7d23adb72108770da8a9558af95e462`) |
| Source | none published |
| Licence | public domain (the release's `readme_eng.txt` and `readme_rus.txt`, copied here as `README.release.eng.txt`, SHA-256 `e9718a3a1a52ec0556807c3b8f8b5dc2ff0baf4f99de21c45e8cd705bff97e9a`, and `README.release.rus.txt`, SHA-256 `3944e02fd8221adf4be2b4f81997e838cd69d44c390fb9df07af89850b6b1f7c`, in Windows-1251): SPDX `LicenseRef-Public-Domain` |

## Licence statements

The release archive's `readme_eng.txt` says:

> Recently there were a lot of discussions around the copyright in modern ZX Spectrum projects.
> For this reason the game is released into the public domain. You are completely free to copy,
> modify, distribute, sell, buy, play, and do whatever you want with it.

and its `readme_rus.txt` the same in Russian ("игра передаётся в общественное достояние. Её можно
свободно копировать, модифицировать, распространять, продавать, покупать и делать всё, что угодно").

## The music

The title tune and the jingles play through Stocker, the beeper engine the author wrote for this
game ("I decided to develop new beeper music engine 'Stocker' [...] The song was then remade for
the engine, and few jingles were made as well"); Stocker's own release
(http://shiru.untergrund.net/files/zx/stocker.zip) says of it "You are completely free to use and
modify it in any way". The tune is the author's own: it is the track "Magic Tokens" (1:05) of his
album "Shiru's 1-Bit Music 2010 Compilation" (https://shiru8bit.bandcamp.com/album/shirus-1-bit-music-2010-compilation),
described as "A compilation of ZX Spectrum 48K beeper music that I composed during 2010" and
credited "Composed by Shiru". The readme names other match-3 games as the inspiration for the
game's idea, not for its music or graphics.

## Third-party parts

The readme lists the tools the game was made with. Two of them leave code in the file: the SDCC
compiler's runtime routines (SDCC's library carries a linking exception that permits distributing
programs built with it), and the depacking routines of the two compressors it names, BitBuster
("some game resources compressed with it, and unpacked when needed"; BitBuster's current release,
1.2, https://github.com/abekermsx/BitBuster-1.2, is MIT) and Hrust ("to pack final binary of the
game"). The readme gives neither depacker's licence.

Attribution: Magic Tokens by Shiru, http://shiru.untergrund.net/software.shtml.

## Tape blocks

The standard ROM loader reads it (six blocks, flags and checksums valid). The BASIC loader (block
1) runs `CLEAR VAL "25087"`, loads block 3 (1,498 bytes of code) at 32768 and runs it, then loads block 5 (19,341 bytes, the game, packed) at 25088 and runs it.

## Controls

Fire leaves the title; the cursor moves with Q/A/O/P, and fire held with a direction swaps two
tokens; ENTER opens the game's menu (restart the level, sound off; it also pauses). The game
reads the keyboard (QAOPM, M fire), Kempston, Sinclair and cursor joysticks at once. The pack maps
F3 (O), A3 (P), F#3/G#3 (Q), G3 (A) and C5-G5 (M), and C4 is ENTER by default (see
`games/games.toml`).
