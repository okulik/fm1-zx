/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The saved game: a 64-byte header and a version 1 .z80 (layout: docs/save-format.md). */
#ifndef FMZX_SAVE_H
#define FMZX_SAVE_H

#include <stdint.h>
#include "zx.h"

#define SAVE_MAGIC "FMZXSAVE"
#define SAVE_VERSION 1u
#define SAVE_HEADER_SIZE 64u
#define SAVE_Z80_HEADER 30u
#define SAVE_Z80_LEN (SAVE_Z80_HEADER + ZX_RAM_SIZE)
#define SAVE_NONE 0xFFFFFFFFu   /* "no game" in a game index */

/* The save area: SAVE_SECTORS flash sectors, SAVE_SECTOR_DATA bytes of the save in each; the rest
 * of a sector stays erased (the installers look for update records there). */
#define SAVE_SECTOR 0x1000u
#define SAVE_SECTOR_DATA 0xF00u
#define SAVE_SECTORS 13u
#define SAVE_AREA_LEN (SAVE_SECTORS * SAVE_SECTOR)
#define SAVE_LEN (SAVE_HEADER_SIZE + SAVE_Z80_LEN)
_Static_assert(SAVE_SECTORS * SAVE_SECTOR_DATA >= SAVE_LEN, "the save fits the save area");

typedef struct {
    uint32_t game;       /* pack index */
    uint32_t game_crc;   /* the pack entry's crc32 */
    uint32_t tape_pos;   /* offset of the next tape block; 0 for snapshot games */
    uint32_t z80_crc;    /* CRC-32 of the .z80 bytes */
} save_info;

/* The 30-byte version 1 .z80 header for registers *r and border colour (0-7): flags bit 0 is
 * bit 7 of R, bits 1-3 the border, bit 5 (compressed) clear. r->pc must be non-zero: a PC of 0
 * would make the file a version 2 header. */
void save_z80_header(const zx_regs *r, uint8_t border, uint8_t out[SAVE_Z80_HEADER]);
/* Fills the 64-byte header for *s (magic, version, fields, z80_len, reserved zeros, CRC). */
void save_header_build(const save_info *s, uint8_t out[SAVE_HEADER_SIZE]);
/* ZX_OK with *out if h is a valid save for the pack at pack_base (already opened with
 * pack_open): magic, version 1, header CRC, z80_len == SAVE_Z80_LEN, game in the pack and that
 * entry's crc32 == game_crc. Else ZX_ERR_FORMAT. */
int save_header_check(const uint8_t h[SAVE_HEADER_SIZE], const uint8_t *pack_base, save_info *out);
/* The offset in the save area of save position pos (0 to SAVE_LEN - 1): the sector tails are skipped. */
uint32_t save_area_offset(uint32_t pos);
/* 1 if pos is the start of a block of the .tap tap[0..len) or equals len, else 0. */
int save_tape_pos_ok(const uint8_t *tap, uint32_t len, uint32_t pos);

#endif
