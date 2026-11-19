/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include <string.h>

#include "pack.h"
#include "save.h"

#define H_VERSION 8u
#define H_GAME 10u
#define H_GAME_CRC 12u
#define H_TAPE_POS 16u
#define H_Z80_LEN 20u
#define H_Z80_CRC 24u
#define H_CRC 60u

static void put16(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
}

static void put32(uint8_t *p, uint32_t v)
{
    put16(p, v);
    put16(p + 2, v >> 16);
}

static uint32_t get16(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8);
}

static uint32_t get32(const uint8_t *p)
{
    return get16(p) | (get16(p + 2) << 16);
}

void save_z80_header(const zx_regs *r, uint8_t border, uint8_t out[SAVE_Z80_HEADER])
{
    out[0] = (uint8_t)(r->af >> 8);
    out[1] = (uint8_t)r->af;
    put16(out + 2, r->bc);
    put16(out + 4, r->hl);
    put16(out + 6, r->pc);
    put16(out + 8, r->sp);
    out[10] = r->i;
    out[11] = r->r & 0x7Fu;
    out[12] = (uint8_t)((r->r >> 7) | ((border & 7u) << 1));
    put16(out + 13, r->de);
    put16(out + 15, r->bc_);
    put16(out + 17, r->de_);
    put16(out + 19, r->hl_);
    out[21] = (uint8_t)(r->af_ >> 8);
    out[22] = (uint8_t)r->af_;
    put16(out + 23, r->iy);
    put16(out + 25, r->ix);
    out[27] = r->iff1;
    out[28] = r->iff2;
    out[29] = r->im;
}

void save_header_build(const save_info *s, uint8_t out[SAVE_HEADER_SIZE])
{
    memset(out, 0, SAVE_HEADER_SIZE);
    memcpy(out, SAVE_MAGIC, 8);
    put16(out + H_VERSION, SAVE_VERSION);
    put16(out + H_GAME, s->game);
    put32(out + H_GAME_CRC, s->game_crc);
    put32(out + H_TAPE_POS, s->tape_pos);
    put32(out + H_Z80_LEN, SAVE_Z80_LEN);
    put32(out + H_Z80_CRC, s->z80_crc);
    put32(out + H_CRC, pack_crc32(out, H_CRC));
}

int save_header_check(const uint8_t h[SAVE_HEADER_SIZE], const uint8_t *pack_base, save_info *out)
{
    pack_game g;

    if (memcmp(h, SAVE_MAGIC, 8) != 0 || get16(h + H_VERSION) != SAVE_VERSION
        || get32(h + H_CRC) != pack_crc32(h, H_CRC) || get32(h + H_Z80_LEN) != SAVE_Z80_LEN
        || pack_base == NULL) {
        return ZX_ERR_FORMAT;
    }
    if (pack_game_at(pack_base, get16(h + H_GAME), &g) != ZX_OK || g.crc32 != get32(h + H_GAME_CRC)) {
        return ZX_ERR_FORMAT;
    }
    out->game = get16(h + H_GAME);
    out->game_crc = get32(h + H_GAME_CRC);
    out->tape_pos = get32(h + H_TAPE_POS);
    out->z80_crc = get32(h + H_Z80_CRC);
    return ZX_OK;
}

int save_tape_pos_ok(const uint8_t *tap, uint32_t len, uint32_t pos)
{
    uint32_t p = 0;

    while (p < pos && p + 2u <= len) {
        p += 2u + get16(tap + p);
    }
    return p == pos && pos <= len;
}

uint32_t save_area_offset(uint32_t pos)
{
    return (pos / SAVE_SECTOR_DATA) * SAVE_SECTOR + pos % SAVE_SECTOR_DATA;
}
