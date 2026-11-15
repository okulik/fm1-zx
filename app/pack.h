/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The game pack: a read-only image of built-in games (layout: docs/pack-format.md). */
#ifndef FMZX_PACK_H
#define FMZX_PACK_H

#include <stdint.h>
#include "keymap.h"
#include "lz4.h"

#define PACK_MAGIC "FMZXPACK"
#define PACK_VERSION 2u
#define PACK_NAME_LEN 28u
#define PACK_MAX_RAW 131072u
#define PACK_HEADER_SIZE 32u
#define PACK_ENTRY_SIZE 160u
enum { PACK_SNA = 1, PACK_Z80 = 2, PACK_TAP = 3 };
enum { PACK_STORED = 0, PACK_LZ4 = 1 };

typedef struct {
    char name[PACK_NAME_LEN + 1];
    uint8_t kind, codec, crop_x;
    keymap keys;
    const uint8_t *data;   /* points into the pack */
    uint32_t stored_len, raw_len, crc32;
} pack_game;

/* Validates the header of the pack at base[0..avail): magic, version, total_len, entry table
 * size and the CRC32 over [32, total_len). ZX_OK with *count, or ZX_ERR_FORMAT. */
int pack_open(const uint8_t *base, uint32_t avail, uint32_t *count);
/* Validates entry `index` and fills *g. pack_open(base, ...) must have succeeded first (this
 * re-reads count and total_len from the header). ZX_ERR_FORMAT if the entry is bad. */
int pack_game_at(const uint8_t *base, uint32_t index, pack_game *g);
/* The learned sound entries of game `index` (docs/pack-format.md, the sound section): ZX_OK with
 * *list pointing into the pack and *len its bytes, or *len 0 (and *list NULL) if the game has
 * none; ZX_ERR_FORMAT if the section (or the game's list in it) is bad. Never reads outside the
 * pack; pack_open(base, ...) must have succeeded first. A bad section never affects the games. */
int pack_sound_at(const uint8_t *base, uint32_t index, const uint8_t **list, uint32_t *len);
/* Decompresses (or copies) the game into out[0..cap); checks length and CRC. */
int pack_extract(const pack_game *g, uint8_t *out, uint32_t cap, uint32_t *len);
/* IEEE CRC-32, reflected, init and xorout 0xFFFFFFFF. */
uint32_t pack_crc32(const uint8_t *p, uint32_t n);

#endif
