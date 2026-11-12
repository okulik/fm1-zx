<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Game pack format

The game pack holds the built-in games. `fmzx-pack games <manifest.toml> -o <pack.bin>` writes it; the firmware reads it in place from flash with `app/pack.c` (it never reads outside the bytes it is given).

All fields are little-endian, offsets count from the start of the pack, and every multi-byte field is naturally aligned.

## Header (32 bytes)

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 8 | `magic` | `FMZXPACK` |
| 8 | 2 | `version` | 1 |
| 10 | 2 | `count` | number of games |
| 12 | 4 | `total_len` | bytes in the pack, a multiple of 4 |
| 16 | 4 | `crc32` | CRC-32 over bytes `[32, total_len)` |
| 20 | 12 | `reserved` | 0 |

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

## Checks

The firmware rejects a pack unless: the magic and version match; `count` is at least 1; `total_len` is a multiple of 4 and fits the bytes available; the entry table fits in `total_len`; and the CRC-32 matches. An entry is rejected unless: the name ends in a NUL within 29 bytes and is printable ASCII; `kind` is 1-3, `codec` 0-1, `crop_x` at most 16; `offset` is 4-aligned and after the entry table; `offset + stored_len` fits in `total_len` (without overflow); `raw_len` is at most 131,072; and a stored game has `stored_len == raw_len`. Extracting checks the output size and the game's CRC-32. CRC-32 is IEEE: reflected, polynomial `0xEDB88320`, initial value and final XOR `0xFFFFFFFF`.

`pack_game_at` re-reads `count` and `total_len` from the header, so call `pack_open` on the same bytes first.
