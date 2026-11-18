<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Castaway

A platform game for the ZX Spectrum 48K by Juan J. Martínez (2016, ZX Dev 2015 competition).
The file is unmodified.

| | |
|---|---|
| File | `castawy.tap` (29,470 bytes), from `castaway/castawy.tap` in the release archive |
| SHA-256 | `2cbd71373f138e685bbb69d72f42c61eca5c78e65642b2f2a7702c610060e8a6` |
| Release archive | https://www.usebox.net/jjm/castaway/castaway.zip (SHA-256 `5e8973d057e7111a85cc2dac955b07c9f58efb139078d73cf07e6739dd0b98bc`) |
| Source | https://github.com/reidrac/castaway-zx (commit `d4746cf4f70d4ffe511885eca5b195a4b0264c95`) |
| Licence | MIT (release), or GPL-3.0 code and CC-BY-SA 2.0 assets (repository): SPDX `MIT OR (GPL-3.0-only AND CC-BY-SA-2.0)` |

## Licence statements

The release archive's `README.txt` (copied here as `README.release.txt`) says:

> LICENCE
> -------
>
> Castaway
> Copyright (C) 2016 by Juan J. Martinez - www.usebox.net
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, [...]

The source repository's `README.md` (copied here as `README.upstream.md`, with the
repository's `gpl-3.0.txt`) says:

> The source code of the game is licensed GPL 3.0, the assets are
> [CC-BY-SA](https://creativecommons.org/licenses/by-sa/2.0/).
>
> The tools/libraries included that I don't own have their own copyright notices
> and license (some are public domain, others are open source).

The game's web page (https://www.usebox.net/jjm/castaway/) has the site-wide footer
"© Juan J. Martínez – CC BY NC SA 4.0, unless stated otherwise"; the release archive and the
repository both state otherwise, as quoted above.

Attribution (CC-BY-SA 2.0): Castaway by Juan J. Martínez, https://www.usebox.net/jjm/castaway/.

## The release and the repository

The repository is a 2021 import of the source, not the build of the 2016 release. The
release tape matches it in its BASIC loader (`loader.bas`: `CLEAR 24099: POKE 23739,111:
LOAD ""CODE: RANDOMIZE USR 64856`), load addresses (code at 24100, loader at 64856, loading
screen at 60000) and texts, but its loader reads headered blocks and its main block (26,926
bytes) is not UCL-compressed, while the repository's `Makefile` writes headerless,
compressed blocks. The release archive carries its own licence, so the tape is covered
either way.

## Controls

Press 1 (OCT-) in the menu for the keyboard (2 is Kempston, 3 Sinclair). O/P move, Q jumps,
SPACE fires, H pauses, CAPS SHIFT + 0 ends the game. The pack maps them onto F3 (O), A3 (P),
F#3/G#3 (Q), C5-G5 (SPACE) and B3 (H) (see `games/games.toml`).
