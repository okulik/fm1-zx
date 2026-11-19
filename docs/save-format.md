# The save format

One saved game lives in a fixed area of flash: a 64-byte header followed by a version 1
`.z80`. All fields are little-endian. The code is `app/save.c`.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | `magic`: `FMZXSAVE` |
| 8 | 2 | `version`: 1 |
| 10 | 2 | `game`: the game's index in the pack |
| 12 | 4 | `game_crc`: the pack entry's `crc32` field |
| 16 | 4 | `tape_pos`: offset of the next tape block; 0 for snapshot games |
| 20 | 4 | `z80_len`: 49,182 |
| 24 | 4 | `z80_crc`: CRC-32 of the `.z80` bytes (`pack_crc32`) |
| 28 | 32 | reserved, 0 |
| 60 | 4 | `header_crc`: CRC-32 of bytes 0-59 |
| 64 | `z80_len` | the `.z80` |

## Where it lives

The save area is flash 0xEA000-0xF7000 (13 sectors of 4 KiB), above the app area; updates of
FM-1 ZX leave it alone. Each sector holds 3,840 bytes of the save at its start, and bytes
0xF00-0xFFF of every sector stay erased, because the installers look for update records there.
Save byte `p` is at area offset `(p / 3840) * 4096 + p % 3840` (`save_area_offset`). The 49,246
bytes of a save fill 13 sectors, the last one partly.

## The `.z80`

A 30-byte version 1 header and the 48 KB of RAM, uncompressed (49,182 bytes). In the header,
flags (byte 12) bit 0 is bit 7 of R, bits 1-3 are the border colour and bit 5 (compressed) is
clear. The PC at offset 6 must be non-zero: a zero PC marks a version 2 header, so a machine
stopped at PC 0 cannot be saved this way.

## A valid save

The header is valid when the magic, the version (1) and the header CRC check out, `z80_len` is
49,182, `game` is below the pack's game count and the pack's entry `game` has `crc32` equal to
`game_crc`: a different pack never resumes a wrong game. The `.z80` CRC is checked when the game
is resumed. For a tape game, `tape_pos` must be a block boundary of the game's tape, or the
tape's length.

## Writing

Erase the area's sectors, program the `.z80` from save byte 64, then program the 64-byte header
last (each at its area offset, the sector tails untouched). Erased flash reads 0xFF, so until the
header lands the area holds no save: a power cut during a write loses the old save and leaves
none, never a broken one.
