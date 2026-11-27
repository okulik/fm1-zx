<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Black Star

A shoot-'em-up for the ZX Spectrum 48K by Juan J. Martínez (2015, BitBitJam 2015). The file is
the release's tape **without its loading screen** (below); the rest is unmodified.

| | |
|---|---|
| File | `blackstr-noscreen.tap` (19,407 bytes), made from `blackstar/blackstr.tap` in the release archive (26,349 bytes, SHA-256 `c6494c5fa5d045ce17866846cdd654bd61a337d8070d84330dcd18d5d060198d`) |
| SHA-256 | `452516e4e8c92997e304529ea2a77bff815792299a5e43afbdb649003d51b5e6` |
| Release archive | https://www.usebox.net/jjm/blackstar/blackstar.zip (SHA-256 `469ac0f51c6a193b681d990d13aae78b047c09569ce8edf2d723eda828ae89ca`) |
| Source | https://www.usebox.net/jjm/blackstar/blackstar_src_jam.zip (SHA-256 `38f175461ad4c6fe92a833563287c8f3a05af5c30d3ecb3bccfc4f6d21e987f0`), whose `blackstr.tap` is the release's, byte for byte |
| Licence | MIT (release `README.txt`, copied here as `README.release.txt`, SHA-256 `a809a7a9e9c09b4241fb63830eb692c518549417e8357c2915545769d231d7bf`), with third-party code under its own terms (below: the music engine public domain, the decompressor MIT-style): SPDX `MIT` |

## Licence statements

The release archive's `README.txt` says:

> License
> -------
>
> The loading screen is based on Angelo Nero's "rainbow wormhole".
>
> This bundle includes some third party code that has its own license, for
> the rest the following applies.
>
> Copyright (C) 2015 by Juan J. Martinez - www.usebox.net
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, [...]

The source archive's `README.txt` has the same licence text. The game's web page
(https://www.usebox.net/jjm/blackstar/) has the site-wide footer "© Juan J. Martínez – CC BY NC SA
4.0, unless stated otherwise"; the release states otherwise, as quoted above, so its MIT licence
applies (the same reading as for Castaway and Escape from Colony 8).

## The loading screen, left out

The release names its loading screen as based on another artist's work, "Angelo Nero's 'rainbow
wormhole'", and its MIT grant covers only "the rest" besides third-party parts. No licence for that
work, and no source for it, could be found, so the screen is not taken: the tape here has the
release's blocks without the screen's two (MIT permits modifying the rest):

- blocks 2 and 3 (the header `screen`, 6,912 bytes at 32768, and its data) are removed;
- the BASIC loader's one line, `10 BORDER 0: PAPER 0: INK 0: CLS: CLEAR 24099: POKE 23739,111:
  LOAD ""SCREEN$: LOAD ""CODE: RANDOMIZE USR 25124`, loses the statement `LOAD ""SCREEN$:` (the
  five bytes `EF 22 22 AA 3A`); the line's length, the header's program and variables lengths
  (93 to 88 bytes) and both blocks' checksums are set to match;
- the code blocks (the header `main`, 19,269 bytes at 25124, and its data) are the release's,
  unchanged.

The game draws its own screens (the title, the menu and the warning) from its code; none is the
loading screen.

## Third-party parts

The tape is built with z88dk and its SP1 sprite library (the source's `Makefile`), so it holds
z88dk start-up, runtime and SP1 code; z88dk's Clarified Artistic License
(https://github.com/z88dk/z88dk/blob/master/LICENSE) permits distributing programs built with it
and does not extend to compiled library output. Two routines in the source's `src/lib/` are other
people's:

- the music engine, utz's ntropic, whose header in `ntropic.asm` says "beeper routine by utz
  01'14, revised 08'14 (irrlichtproject.de) [...] this code is public domain", adapted to z88dk by
  Juan J. Martínez;
- the UCL decompressor `ucl.asm`, "Decompressor for Z80 (c) 2000 Adam D. Moss", under an MIT-style
  permission notice that asks for the notice to be kept with copies; it is copied here from the
  source as `COPYING.ucl`.

The source credits no one else for the music (`song1.ntropic`, `gameover.ntropic`), the graphics
or the font, and the title screen reads "Code, Graphics & sound / Juan J. Martinez, @reidrac".

Attribution: Black Star by Juan J. Martínez, https://www.usebox.net/jjm/blackstar/.

## Controls

Any key leaves the title (the title tune reads the keys between its patterns, so hold it a moment),
then 1 (OCT-) chooses the keyboard (2 is Kempston, 3 Sinclair, 4 redefines the keys) and any key
skips the intro. O/P move left and right, SPACE fires, CAPS SHIFT + 0 ends the game. The pack maps
them onto F3 (O), A3 (P) and C5-G5 (SPACE) (see `games/games.toml`).
