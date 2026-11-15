/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "pack.h"
#include "zx.h"
#include <string.h>

/* Entry layout (160 bytes): name[29] kind codec crop_x | offset stored_len raw_len crc32 | keys[110] | reserved[2] */
#define E_NAME 0u
#define E_KIND 29u
#define E_CODEC 30u
#define E_CROP 31u
#define E_OFFSET 32u
#define E_STORED 36u
#define E_RAW 40u
#define E_CRC 44u
#define E_KEYS 48u

static uint32_t rd16(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8);
}

static uint32_t rd32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

uint32_t pack_crc32(const uint8_t *p, uint32_t n)
{
    static const uint32_t nib[16] = {
        0x00000000u, 0x1DB71064u, 0x3B6E20C8u, 0x26D930ACu, 0x76DC4190u, 0x6B6B51F4u, 0x4DB26158u, 0x5005713Cu,
        0xEDB88320u, 0xF00F9344u, 0xD6D6A3E8u, 0xCB61B38Cu, 0x9B64C2B0u, 0x86D3D2D4u, 0xA00AE278u, 0xBDBDF21Cu,
    };
    uint32_t c = 0xFFFFFFFFu, i;

    for (i = 0; i < n; i++) {
        c ^= p[i];
        c = (c >> 4) ^ nib[c & 15u];
        c = (c >> 4) ^ nib[c & 15u];
    }
    return ~c;
}

int pack_open(const uint8_t *base, uint32_t avail, uint32_t *count)
{
    uint32_t n, total;

    if (avail < PACK_HEADER_SIZE || memcmp(base, PACK_MAGIC, 8) != 0 || rd16(base + 8) != PACK_VERSION) {
        return ZX_ERR_FORMAT;
    }
    n = rd16(base + 10);
    total = rd32(base + 12);
    if (total > avail || total < PACK_HEADER_SIZE || (total & 3u) != 0u || n == 0u || n * PACK_ENTRY_SIZE > total - PACK_HEADER_SIZE) {
        return ZX_ERR_FORMAT;
    }
    if (pack_crc32(base + PACK_HEADER_SIZE, total - PACK_HEADER_SIZE) != rd32(base + 16)) {
        return ZX_ERR_FORMAT;
    }
    *count = n;
    return ZX_OK;
}

int pack_game_at(const uint8_t *base, uint32_t index, pack_game *g)
{
    uint32_t n = rd16(base + 10), total = rd32(base + 12);
    const uint8_t *e;
    uint32_t i, offset, stored, raw;

    if (index >= n) {
        return ZX_ERR_FORMAT;
    }
    e = base + PACK_HEADER_SIZE + index * PACK_ENTRY_SIZE;
    for (i = 0; i <= PACK_NAME_LEN && e[E_NAME + i] != 0u; i++) {
        if (e[E_NAME + i] < 0x20u || e[E_NAME + i] > 0x7Eu) {
            return ZX_ERR_FORMAT;
        }
    }
    if (i > PACK_NAME_LEN) {
        return ZX_ERR_FORMAT;   /* no NUL within the 29 bytes */
    }
    offset = rd32(e + E_OFFSET);
    stored = rd32(e + E_STORED);
    raw = rd32(e + E_RAW);
    if (e[E_KIND] < PACK_SNA || e[E_KIND] > PACK_TAP || e[E_CODEC] > PACK_LZ4 || e[E_CROP] > 16u) {
        return ZX_ERR_FORMAT;
    }
    if ((offset & 3u) != 0u || offset < PACK_HEADER_SIZE + n * PACK_ENTRY_SIZE || offset > total || stored > total - offset) {
        return ZX_ERR_FORMAT;
    }
    if (raw > PACK_MAX_RAW || (e[E_CODEC] == PACK_STORED && stored != raw)) {
        return ZX_ERR_FORMAT;
    }
    memcpy(g->name, e + E_NAME, i + 1u);   /* includes the NUL */
    g->kind = e[E_KIND];
    g->codec = e[E_CODEC];
    g->crop_x = e[E_CROP];
    for (i = 0; i < KM_INPUTS; i++) {
        g->keys.act[i][0] = e[E_KEYS + 2u * i];
        g->keys.act[i][1] = e[E_KEYS + 2u * i + 1u];
    }
    g->data = base + offset;
    g->stored_len = stored;
    g->raw_len = raw;
    g->crc32 = rd32(e + E_CRC);
    return ZX_OK;
}

/* Header: sound_offset at 20, sound_len at 24. Section: u8 version, u8 0, u16 count, u32 list_offset[count]. */
#define H_SOUND_OFFSET 20u
#define H_SOUND_LEN 24u
#define SOUND_VERSION 1u

int pack_sound_at(const uint8_t *base, uint32_t index, const uint8_t **list, uint32_t *len)
{
    uint32_t n = rd16(base + 10), total = rd32(base + 12);
    uint32_t so = rd32(base + H_SOUND_OFFSET), sl = rd32(base + H_SOUND_LEN);
    uint32_t i, off, end, o, table;
    const uint8_t *s;

    *list = NULL;
    *len = 0;
    if (so == 0u && sl == 0u) {
        return ZX_OK;   /* no section */
    }
    table = 4u + 4u * n;   /* n <= 65535: no overflow */
    if (index >= n || (so & 3u) != 0u || so < PACK_HEADER_SIZE + n * PACK_ENTRY_SIZE || so > total || sl > total - so || sl < table) {
        return ZX_ERR_FORMAT;
    }
    s = base + so;
    if (s[0] != SOUND_VERSION || rd16(s + 2) != n) {
        return ZX_ERR_FORMAT;
    }
    off = rd32(s + 4u + 4u * index);
    if (off == 0u) {
        return ZX_OK;   /* none for this game */
    }
    if ((off & 3u) != 0u || off < table || off >= sl) {
        return ZX_ERR_FORMAT;
    }
    end = sl;   /* the next list's start, or the section's end */
    for (i = 0; i < n; i++) {
        o = rd32(s + 4u + 4u * i);
        if (o > off && o < end) {
            end = o;
        }
    }
    *list = s + off;
    *len = end - off;
    return ZX_OK;
}

int pack_extract(const pack_game *g, uint8_t *out, uint32_t cap, uint32_t *len)
{
    uint32_t got;

    if (g->raw_len > cap || g->raw_len > PACK_MAX_RAW) {
        return ZX_ERR_FORMAT;
    }
    if (g->codec == PACK_STORED) {
        if (g->stored_len != g->raw_len) {
            return ZX_ERR_FORMAT;
        }
        memcpy(out, g->data, g->raw_len);
        got = g->raw_len;
    } else if (g->codec == PACK_LZ4) {
        if (lz4_block_decode(g->data, g->stored_len, out, g->raw_len, &got) != ZX_OK || got != g->raw_len) {
            return ZX_ERR_FORMAT;
        }
    } else {
        return ZX_ERR_FORMAT;
    }
    if (pack_crc32(out, got) != g->crc32) {
        return ZX_ERR_FORMAT;
    }
    *len = got;
    return ZX_OK;
}
