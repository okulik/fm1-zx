<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# ZX Dino

A dinosaur runner in the style of the Chrome offline game, for the ZX Spectrum 48K, written in Z80
assembly by Aleksey Kurepin (the repository's committer; the licence names "Kukovnikov Bonifas", as
does the game's own screen), October 2026. The file is unmodified.

| | |
|---|---|
| File | `dino.tap` (9,504 bytes), from `dino.tap` in the repository |
| SHA-256 | `feda14f25decccea6167f21f8340e83942c75f6dbdf178603d0e99eff9c6a269` |
| Download URL | https://raw.githubusercontent.com/foxweb/zxdino/7dfad2befa8dfca2d696ef8b09d57f07d5dc1397/dino.tap |
| Source | https://github.com/foxweb/zxdino at commit `7dfad2befa8dfca2d696ef8b09d57f07d5dc1397` ("sprites rewrited", 2026-10-23); the repository has no releases, and the file is the one committed there |
| Licence | MIT (`LICENSE` in the repository, copied here as `LICENSE.release`, SHA-256 `1c77d8631910df83f71f838316e5b9f95320f11522ccd8d852dedfc8c02d3825`; the repository's `README.md`, copied as `README.release.md`, SHA-256 `215315bf5023e1ea4bd4541ab6eec91d434f2d4a6b90c96d7490ca3be4af994f`, ends "MIT. See LICENSE."): SPDX `MIT` |

## Licence statements

`LICENSE` says:

> MIT License
>
> Copyright © 2026 Kukovnikov Bonifas
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, [...]

The repository's `README.md` ends: "## License / MIT. See [LICENSE](LICENSE)."

Third-party parts and sprite origin: the game is assembled from `dino.asm` (no libraries, no compiler
runtime) and the 12 sprites in `sprites/*.gif`, 8 to 16 pixels wide and 8 to 22 tall, converted by
the repository's `art.py`. In the initial commit the sprites were typed in `art.py` as `.`/`#` text
tables (a 16x18 dinosaur, cacti, birds, a cloud), and the last commit moved them to GIF files;
all of it is in the one author's commit history, and the README says the sprites are GIF bitmaps
in `sprites/` that `art.py` turns into `sprites.inc`. They are redrawn at the Spectrum's scale (the
Chrome game's dinosaur is about 44 by 47 pixels), not copied from Chrome's sprite sheet, and the
code, README and licence name no other source for them; the game is described only as "in the style
of" the Chrome offline game. The text uses the Spectrum ROM's character set through its address
(`ld bc, $3D00`), which copies nothing from the ROM into the file. The screen's
credit line is "(c) 2026 Kukovnikov Bonifas". No other credit or third-party notice appears.

Attribution: ZX Dino by Aleksey Kurepin / Kukovnikov Bonifas, https://github.com/foxweb/zxdino.

## Tape blocks

The standard ROM loader reads it (checked with `Machine::tape_insert`: all four blocks, flags and
checksums valid), and in the host core the title screen appears within 300 frames of `tape_autostart`
(the game's own demo is playing by frame 1,500):

| # | Flag | Length | Content |
|---|---|---|---|
| 0 | 00 (header) | 19 | Program `dino`, 63 bytes, autostart line 10 |
| 1 | FF (data) | 65 | BASIC loader |
| 2 | 00 (header) | 19 | Bytes `dino-code`, 9,391 bytes, load address 32,768 |
| 3 | FF (data) | 9,393 | the game code |

## Controls

Any key starts a run, SPACE or Q jumps, A or CAPS SHIFT ducks (a low bird passes overhead). After
a crash release the key and press any key again. The game reads the keyboard only. The pack maps
them onto F#3/G#3 (Q), C5-G5 (SPACE) and G3 (A); SEQ is CAPS SHIFT and any button starts a run
(see `games/games.toml`).
