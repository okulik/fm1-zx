<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Escape from Colony 8

A top-down action game for the ZX Spectrum 48K by Juan J. Martínez (2014, version 1.0). The file
is unmodified.

| | |
|---|---|
| File | `colony8.tap` (33,045 bytes), from `colony8/colony8.tap` in the release archive |
| SHA-256 | `b1d2907a80fe0f10ad3ef1f17f49b77b1da06d8f6224bacfd35092ec5298249a` |
| Release archive | https://usebox.net/jjm/colony8/colony8.zip (15,988 bytes, SHA-256 `552e4fa81b23202b9469137b5e6cb3e60507e9a7b1929b1a3ef3cd4c6b42bc37`) |
| Source | none published (the author's GitHub account has no repository for the game) |
| Licence | MIT (release `README.txt`, copied here as `README.release.txt`, SHA-256 `87161b6f9b750e467514fef4a024b846131718047d949fbf51be72cf7c645be9`): SPDX `MIT` |

## Licence statements

The release archive's `README.txt` says:

> License
> =======
>
> Escape from Colony 8, version 1.0
> Copyright (C) 2014 by Juan J. Martinez - www.usebox.net
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, [...]

The game's web page (https://www.usebox.net/jjm/colony8/) has the site-wide footer
"© Juan J. Martínez – CC BY NC SA 4.0, unless stated otherwise"; the release archive states
otherwise, as quoted above, so its MIT licence applies (the same reading as for Castaway).

The release names no other author and credits no library, music, font or artwork.

Third-party parts: the tape was built with z88dk (the code block begins with the compiler's
"Small C+ ZX" start-up text), so it holds z88dk start-up code and a few runtime routines. z88dk's
Clarified Artistic License (https://github.com/z88dk/z88dk/blob/master/LICENSE) permits
distributing programs built with it and does not extend to compiled library output. The release
and the author's pages do not say which routines the file holds (no source is published), and no
art, music, font or loading screen comes from them: the screen shows "a game by @reidrac" and
"Copyright (C) MMXIV usebox.net", and no other credit or third-party notice appears in the file
(checked with `strings`).

Attribution: Escape from Colony 8 by Juan J. Martínez, https://www.usebox.net/jjm/colony8/.

## Tape blocks

The standard ROM loader reads it (checked with `Machine::tape_insert`: all six blocks, flags and
checksums valid), and in the host core the title screen appears about 2 seconds after `tape_autostart`:

| # | Flag | Length | Content |
|---|---|---|---|
| 0 | 00 (header) | 19 | Program `colony8`, 93 bytes, autostart line 0 |
| 1 | FF (data) | 95 | BASIC: `BORDER 0: PAPER 0: INK 0: CLEAR 24099: POKE 23739,111: LOAD "" SCREEN$: LOAD "" CODE: RANDOMIZE USR 26708` |
| 2 | 00 (header) | 19 | Bytes `screen`, 6,912 bytes (loading screen, loaded by `LOAD "" SCREEN$`) |
| 3 | FF (data) | 6,914 | the screen |
| 4 | 00 (header) | 19 | Bytes `game`, 25,965 bytes, load address 26,708 |
| 5 | FF (data) | 25,967 | the game code |

## Controls

At the menu press 1 for the keyboard (2 is Kempston, 3 Sinclair, 4 redefines the keys). O/P move
left and right, Q/A up and down, SPACE fires (hold it for some seconds to load a missile, which needs
ammunition, and release it to launch), H pauses, CAPS SHIFT + 0 leaves the game. The pack maps them onto
F3 (O), A3 (P), F#3/G#3 (Q), G3 (A), C5-G5 (SPACE) and B3 (H) (see `games/games.toml`).
