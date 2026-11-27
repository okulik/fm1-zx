<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# The engine reader's fixtures

`tests/engines.rs` runs the engine reader (`chipmusic::engine`) on a small program per engine.
The songs are invented for the tests; no game, and no tune from one, is here.

## Tritone: the engine's own code

`tritone-v2-c009.hex` is the Tritone v2 beeper music engine by Shiru, 446 bytes, as its
author's release assembles it at C009h: the bytes C009h-C1C6h of `test.sna` in
<http://shiru.untergrund.net/files/zx/tritone.zip> (archive SHA-256
`30ae88e209e06827e5f7acd1436e6b358c365980e099cad6b73baaac59903b7a`, `test.sna` SHA-256
`0d6d81cbbd731d7e79f366b2f898bd3d74f3297c1215fab0e11c94159555fb1e`), between the release's
start (`LD HL,musicData; CALL play; JP begin`) and its song, which is not taken. The engine's
source, `tritone.asm` in the same archive, says:

> ;Tritone v2 beeper music engine by Shiru (shiru@mail.ru) 03'11
> ;Three channels of tone, per-pattern tempo
> ;One channel of interrupting drums
> ;Feel free to do whatever you want with the code, it is PD

The test puts our own start (the same three instructions) at C000h and a song of ours, written
in the format the archive's `readme.txt` documents, at E000h.

## ntropic and Stocker: programs of ours

The fixtures for ntropic and Stocker are programs written for these tests (hand-assembled in
`tests/engines.rs`), not the engines' code. Each reproduces what the reader relies on, from the
engines' published descriptions and the reader's signatures: ntropic's row read (`LD IYH,10h;
POP HL; LD A,(HL); INC A; RET Z` on 4-byte rows: drum flag, two counters, noise), its sound
loop's `OUT` once a pass (`EX AF,AF'; OUT (FEh),A; DEC D; JP NZ`) and its click drum (`LD
A,(HL); AND F8h; OUT (FEh),A; DEC HL; DEC BC`); Stocker's tick (`LD HL,nn` with the passes in H,
the steps in DE and DE'), its sound loop (`ADD IX,DE; SBC A,A; AND vol1; LD B,A; EXX; ADD
HL,DE; EXX; SBC A,A; AND vol2; OR B; JR Z`) and two of its drums' code (kick and hat). Their
songs and the rest of their code are ours, so they are under the repository's licence.

ntropic (by utz, in <https://github.com/utz82/ZX-Spectrum-1-Bit-Routines>, BSD-style licence)
and Stocker (by Shiru, whose release says "You are completely free to use and modify it in any
way") could be taken as they are, but neither release ships the engine assembled on its own
and the repository has no Z80 assembler: a program of ours keeps the fixture small, readable and
free of any song.
