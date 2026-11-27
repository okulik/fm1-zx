<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Micro INC

A match-3 puzzle game for the ZX Spectrum 48K by Juan J. Martínez (2014, version 1.0.1). The file
is unmodified.

| | |
|---|---|
| File | `microInc.tap` (22,573 bytes), from `microInc/microInc.tap` in the release archive |
| SHA-256 | `6342042ac6a36121dc51aa3465414dc3c810ece713ad4aac0e04ec7693caf3c8` |
| Release archive | https://www.usebox.net/jjm/micro-inc/microInc.zip (SHA-256 `7f61ec496911797f20b2f5a478433c82023864f15d02957a0cec9ac28a3062af`) |
| Source | none published |
| Licence | MIT (release `README.txt`, copied here as `README.release.txt`, SHA-256 `849e6f65e49c09fa612c986c79d10f81c61cc486146bbcf238f1d9f8ff028f4d`): SPDX `MIT` |

## Licence statements

The release archive's `README.txt` says:

> License
> =======
>
> Micro INC, version 1.0.1
> Copyright (C) 2014 by Juan J. Martinez - www.usebox.net
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, [...]

The game's web page (https://www.usebox.net/jjm/micro-inc/) has the site-wide footer "© Juan J.
Martínez – CC BY NC SA 4.0, unless stated otherwise"; the release states otherwise, as quoted
above, so its MIT licence applies (the same reading as for Castaway and Escape from Colony 8).

The release names no other author and credits no library, music, font or artwork; the loading
screen and the title screen read "a game by @reidrac" and "© 2014 usebox.net".

Third-party parts: the tape was built with z88dk (the code block holds the compiler's "Small C+
ZX" start-up text), so it holds z88dk start-up code and runtime routines; z88dk's Clarified
Artistic License (https://github.com/z88dk/z88dk/blob/master/LICENSE) permits distributing programs
built with it and does not extend to compiled library output. No source is published, so which
routines the file holds is not stated; no other credit or third-party notice appears in the file
(checked with `strings`).

Attribution: Micro INC by Juan J. Martínez, https://www.usebox.net/jjm/micro-inc/.

## Tape blocks

The standard ROM loader reads it (six blocks, flags and checksums valid):

| # | Flag | Length | Content |
|---|---|---|---|
| 0 | 00 (header) | 19 | Program `microInc`, 93 bytes, autostart line 0 |
| 1 | FF (data) | 95 | BASIC: `BORDER 0: PAPER 0: INK 0: CLS: CLEAR 24099: POKE 23739,111: LOAD "" SCREEN$: LOAD "" CODE: RANDOMIZE USR 24100` |
| 2 | 00 (header) | 19 | Bytes `screen`, 6,912 bytes (the loading screen) |
| 3 | FF (data) | 6,914 | the screen |
| 4 | 00 (header) | 19 | Bytes `game`, 15,493 bytes, load address 24,100 |
| 5 | FF (data) | 15,495 | the game code |

## Music

The title tune plays on the title screen until a key is pressed; the game itself has sound effects
and no music. Its routine is the three-channel engine of Escape from Colony 8's music, which the
firmware knows built in, so FM: engines plays the tune live from the game's own registers.

## Controls

Any key leaves the title, then 1 (OCT-) chooses the keyboard (2 is Kempston, 3 Sinclair, 4
redefines the keys). O/P move the block round the board, SPACE pushes it in, H pauses, CAPS SHIFT
+ 0 leaves the game. The pack maps them onto F3 (O), A3 (P), C5-G5 (SPACE) and B3 (H) (see
`games/games.toml`).
