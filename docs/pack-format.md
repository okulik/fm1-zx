<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Game pack format

The game pack holds the built-in games, and optionally, for each game, the sound entries learned from its music (version 2). `fmzx-pack games <manifest.toml> -o <pack.bin>` writes it; the firmware reads it in place from flash with `app/pack.c` (it never reads outside the bytes it is given).

All fields are little-endian, offsets count from the start of the pack, and every multi-byte field is naturally aligned.

## Header (32 bytes)

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 8 | `magic` | `FMZXPACK` |
| 8 | 2 | `version` | 2 |
| 10 | 2 | `count` | number of games |
| 12 | 4 | `total_len` | bytes in the pack, a multiple of 4 |
| 16 | 4 | `crc32` | CRC-32 over bytes `[32, total_len)` |
| 20 | 4 | `sound_offset` | of the sound section, 4-byte aligned; 0 with `sound_len` 0: no section |
| 24 | 4 | `sound_len` | bytes of the sound section |
| 28 | 4 | `reserved` | 0 |

## Entries (`count` of 160 bytes, from offset 32)

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 29 | `name` | NUL-terminated printable ASCII, at most 28 characters |
| 29 | 1 | `kind` | 1 `.sna`, 2 `.z80`, 3 `.tap` |
| 30 | 1 | `codec` | 0 stored, 1 LZ4 block |
| 31 | 1 | `crop_x` | 0-16: the first of the 256 screen columns shown on the 240-wide LCD |
| 32 | 4 | `offset` | of the game's data, 4-byte aligned, at or after the end of the entry table |
| 36 | 4 | `stored_len` | bytes at `offset`; for codec 0 equal to `raw_len` |
| 40 | 4 | `raw_len` | bytes of the original file, at most 131,072 |
| 44 | 4 | `crc32` | CRC-32 of the original file |
| 48 | 110 | `keys` | 55 inputs x 2 action bytes (see `app/keymap.h`) |
| 158 | 2 | `reserved` | 0 |

## Data

Each game's data follows the entry table, padded so the next starts on a 4-byte boundary. Codec 1 is one raw LZ4 block (the `lz4_flex::block::compress` output: no frame, no size prefix). The packer stores a game uncompressed (codec 0) when LZ4 does not make it smaller.

## Sound section (version 2)

The section follows the last game's data, starts on a 4-byte boundary, lies inside `total_len` and is covered by the CRC. `fmzx-pack` writes none (`sound_offset` and `sound_len` 0) unless the personal pack builder attaches one (`attach_sound`). A pack with no section is a valid version 2 pack.

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | `version` | 1 |
| 1 | 1 | | 0 |
| 2 | 2 | `count` | the pack's game count |
| 4 | 4 x `count` | `list_offset[]` | per game, relative to the section start; 0: no learned entries for that game |
| | | lists | each starts on a 4-byte boundary |

A list's length is the distance from its offset to the nearest greater list offset, or to the section's end. `sound_list` and `pack_sound_at` return that span, so a list written with a length that is not a multiple of 4 comes back padded with zeros.

### Entry list

A list holds up to 8 entries for one game (`SND_LEARNED_MAX`), of two kinds. A learned entry (kind 0) describes one music routine the game's own code contains: a signature to find it by, the `OUT` sites where its notes start, small programs that compute each note's period from the machine state, and each channel's instrument. A kind-1 record gives stored instruments to a built-in entry the game uses. All fields are little-endian.

```
u8 count (<= 8), u8 0, u16 0, then `count` entries, each a learned entry or a kind-1 record (below); a learned entry:
  u16 size (a multiple of 4: the whole entry, from its first byte to the next entry), u8 kind (0: music), u8 channels (1-3)
  u64 digest
  u16 len, u8 n_wild, u8 n_reloc
  u8 n_sites, u8 flags (0; 1: flips; 2: observed), u8 drum, u8 timing (1: a timing program follows)
  u16 pass, u16 stale_from, u16 stale_len   (stale: an anchor-relative pc range; stale_len 0: none)
  u8 level[3], u8 instrument[3]
  u8 prog_len[3], u8 timing_len
  u16 wild[n_wild], u16 reloc[n_reloc], {u16 off, u8 role, u8 chan} sites[n_sites], u16 pos[channels]
  channel programs (prog_len[c] bytes each, for c < channels), the timing program, zero padding up to `size`
```

The fixed part of an entry is 36 bytes. `digest`, `len`, `wild`, `reloc`, `sites`, `pass`, `pos`, `flags` and `drum` are the matcher's signature fields (`core/sound/sound.h`, `snd_engine`); `role` is 0 tonal, 1 drum, 2 stop; `chan` is the channel of a tonal site, or `0xFF` (any) on a `flips` entry, and always 0 on an `observed` entry. `level[c]` is channel `c`'s loudness; entries are named `learned`.

`flags` is a bit set, at most one bit of it set: bit 0 `flips` (each edge flips one channel's level, the speaker sounds their XOR: `snd_engine.flips`), bit 1 `observed` (`snd_engine.observed`). An observed entry describes a routine that mixes its channels into the one speaker bit, so its edges cannot be split into channels: the personal-pack builder verifies it on the host (by ear and by a replay through method B), and method B takes every frame whose edges all come from the entry's sites with the voices its programs give, without checking them against the edges. Every `OUT` site of such a routine is listed as a tonal site of channel 0.

`instrument[c]` is channel `c`'s stored role in the FM synth: 0 none (the live rule decides by pitch), 1 bass, 2 lead, 3 pad. A value above 3 counts as 0, and so does the byte of a channel past `channels`. While the entry claims a frame, method B gives each of its channels that role; a channel with none, and any frame no entry with stored roles claims, gets the live rule's role (`core/sound/roles.c`).

A kind-1 record, "roles for a built-in entry", is 15 bytes padded to 16 (or more: `size` covers it):

```
u16 size (a multiple of 4, at least 16), u8 kind = 1, u8 channels (1-3)
u64 digest     the built-in entry's (core/sound/engines_builtin.h): which entry it is for
u8 role[3]     as instrument[] above
zero padding up to `size`
```

It produces no engine: once the list is decoded, `snd_learned_roles` gives the built-in entry with that `digest` these roles for this game (roles above 3, and those of channels past the record's `channels` or the entry's, count as 0). A record whose `digest` names no built-in entry (or names a learned entry) changes nothing; of two records for the same entry, the later wins. The firmware assembles its table (the built-in entries, then the learned ones) whenever the list decodes, so a game whose list holds kind-1 records and no learned entry still gets its built-in entries' roles.

The decoder (`snd_learned_decode`, `core/sound/learned.c`) rejects the whole list (`ZX_ERR_FORMAT`, no entries) unless all of this holds, for every entry:

- the list is at least 4 bytes and 4-byte aligned in memory, and `count` is at most 8;
- the entry's `size` and `kind` lie in the list; `kind` is 0 or 1; `size` is at least 36 (a kind-1 record: 16), a multiple of 4 and fits in the list (entries are packed one after the other; bytes after the last entry are ignored);
- a kind-1 record's `channels` is 1-3; it counts towards `count`'s 8, has no sites and adds no work; nothing else in it is checked (its roles are read as above);
- for a learned entry (kind 0):
  - `channels` is 1-3; `flags` is 0, 1 or 2 (no other bit, not `flips` and `observed` together); `timing` is 0 or 1;
  - `len` is 1 to 512 (`SND_LEARNED_LEN_MAX`) and `n_sites` at most 16 (`SND_LEARNED_SITES_MAX`);
  - `prog_len[c]` is 1-64 for each channel and 0 for the unused ones; `timing_len` is 1-64 with a timing program, else 0;
  - the arrays and programs together fit in `size` after the 36 fixed bytes;
  - `wild` offsets strictly ascend and are below `len`; `reloc` offsets ascend at least 2 apart and are below `len`;
  - every site offset is below `len`, `role` is at most 2, and a tonal site's `chan` is below `channels` (or `0xFF` on a `flips` entry; exactly 0 on an `observed` entry);
  - every program is valid (below);
- over the whole list, the sum of the learned entries' `n_sites x len` is at most 4096 (`SND_LEARNED_WORK_MAX`): method B hashes each site's signature for an `OUT` no entry is known at yet, so this bounds that work per frame.

The reserved bytes (the 2 after the list's `count`, the padding) are ignored.

A game whose list is rejected plays as if it had no learned entries. A bad sound section never affects the games; a bad section (or list) is the same as none.

The `instrument` bytes and kind-1 records do not change the pack version: the pack is always installed together with the firmware that reads it (both are in one `.fwsc` package), so no firmware ever meets a pack newer than itself.

### Program

A program is bytecode for a stack machine of `uint32_t` values, at most 8 deep and at most 64 bytes long (`SND_PROG_MAX`). Operands are little-endian. It runs against the machine (memory at the frame's end), the entry's *anchor* (where the signature matched) and a register set `ctx` (the registers at the site, or at the frame's end for the timing program's second run).

| Op | Operand | Pushes or does |
|---|---|---|
| `01` | u8 k | k |
| `02` | u16 k | k |
| `03` | u32 k | k |
| `10` | u16 o | the byte at `anchor + o` |
| `11` | u16 o | the word at `anchor + o` |
| `12` | u16 a | the byte at `a` |
| `13` | u16 a | the word at `a` |
| `14` | u8 r, i8 d | the byte at `reg16(r) + d`; r is 8-12 (BC, DE, HL, IX, IY) |
| `20` | u8 r | register r of `ctx` |
| `30` `31` `32` | | add, sub, mul of the top two (wrapping) |
| `33` | | div, unsigned; a zero divisor ends the program as silent |
| `34` | | and |
| `35` `36` | u8 s | shift the top right, left by s (0-31) |
| `40` | | pop x; x = 0 ends the program as silent |

Registers: 0 A, 1 F, 2 B, 3 C, 4 D, 5 E, 6 H, 7 L, 8 BC, 9 DE, 10 HL, 11 IX, 12 IY, 13 SP, 14 A', 15 BC', 16 DE', 17 HL'. Addresses wrap at 16 bits, and any address is safe to read.

A program ends at its length with exactly one value on the stack: the result. Anything else makes it invalid: an unknown op, an operand past the end, a stack underflow or overflow (a ninth push), an op with a bad register or shift, a length over 64, or another depth at the end. The decoder dry-runs every program (every read gives 0, nothing is silent) so that no invalid program is ever run. A channel program that is silent or gives 0 leaves that channel without a note. A timing program that is invalid is rejected with the list; one that is silent in a frame leaves the timing unknown (`SND_B_SINCE_ALL`) for it.

## Checks

The firmware rejects a pack unless: the magic and version match; `count` is at least 1; `total_len` is a multiple of 4 and fits the bytes available; the entry table fits in `total_len`; and the CRC-32 matches. An entry is rejected unless: the name ends in a NUL within 29 bytes and is printable ASCII; `kind` is 1-3, `codec` 0-1, `crop_x` at most 16; `offset` is 4-aligned and after the entry table; `offset + stored_len` fits in `total_len` (without overflow); `raw_len` is at most 131,072; and a stored game has `stored_len == raw_len`. Extracting checks the output size and the game's CRC-32. CRC-32 is IEEE: reflected, polynomial `0xEDB88320`, initial value and final XOR `0xFFFFFFFF`.

The sound section is not checked by `pack_open`. `pack_sound_at(base, index, &list, &len)` checks it for one game, and returns `ZX_ERR_FORMAT` unless: `sound_offset` and `sound_len` are both 0 (no section: `len` 0), or `sound_offset` is 4-aligned and after the entry table, `sound_offset + sound_len` fits in `total_len` (without overflow), and `sound_len` holds the offset table; the section `version` is 1 and its `count` equals the game's count; and the game's list offset is 0 (none), or 4-aligned, past the offset table and inside the section. The entry list is then checked by `snd_learned_decode` (above), and the pack version is 2: a pack of any other version is rejected by `pack_open`.

`pack_game_at` and `pack_sound_at` re-read `count` and `total_len` from the header, so call `pack_open` on the same bytes first.
