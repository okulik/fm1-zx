/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include <stddef.h>
#include <stdint.h>
#include "fmsave.h"
#include "save.h"
#include "pack.h"
#include "usbota.h"

/* the core's register read lives in RAMTEXT: main.c's XIP entry reaches it */
void fmzx_xip_get_regs(const zx_machine *m, zx_regs *r);

#define STEPS (SAVE_SECTORS + 1u) /* the sectors, then the header */

/* The flash programs from RAM only: the headers are static buffers, the 48 KB is the machine's own. */
static uint8_t hdr_z80[SAVE_Z80_HEADER];
static uint8_t hdr_save[SAVE_HEADER_SIZE];
static const uint8_t *ram_bytes;
static save_info info;
static uint32_t crc;  /* running CRC-32 (register form) of the .z80 */
static uint32_t step; /* next step */
static int busy;

static void crc_add(const uint8_t *p, uint32_t n)
{
    static const uint32_t nib[16] = {
        0x00000000u, 0x1DB71064u, 0x3B6E20C8u, 0x26D930ACu, 0x76DC4190u, 0x6B6B51F4u, 0x4DB26158u, 0x5005713Cu,
        0xEDB88320u, 0xF00F9344u, 0xD6D6A3E8u, 0xCB61B38Cu, 0x9B64C2B0u, 0x86D3D2D4u, 0xA00AE278u, 0xBDBDF21Cu,
    };
    uint32_t c = crc, i;
    for (i = 0; i < n; i++) {
        c ^= p[i];
        c = (c >> 4) ^ nib[c & 15u];
        c = (c >> 4) ^ nib[c & 15u];
    }
    crc = c;
}

void fmsave_begin(const zx_machine *m, uint32_t game, uint32_t game_crc, uint32_t tape_pos)
{
    zx_regs r;
    fmzx_xip_get_regs(m, &r);
    save_z80_header(&r, m->border_col, hdr_z80);
    ram_bytes = (const uint8_t *)m->ram_words;
    info.game = game;
    info.game_crc = game_crc;
    info.tape_pos = tape_pos;
    info.z80_crc = 0;
    crc = 0xFFFFFFFFu;
    crc_add(hdr_z80, SAVE_Z80_HEADER);
    step = 0;
    busy = 1;
}

/* Programs save positions [p, end) (at or after SAVE_HEADER_SIZE, within one sector's share). */
static int program(uint32_t p, uint32_t end)
{
    while (p < end) {
        uint32_t z = p - SAVE_HEADER_SIZE, n;
        const uint8_t *src;
        if (z < SAVE_Z80_HEADER) {
            n = SAVE_Z80_HEADER - z;
            src = hdr_z80 + z;
        } else {
            n = end - p;
            src = ram_bytes + (z - SAVE_Z80_HEADER);
        }
        if (n > end - p)
            n = end - p;
        if (fmzx_flash_write(FMZX_SAVE_FLASH + save_area_offset(p), src, n) != 0)
            return -1;
        p += n;
    }
    return 0;
}

int fmsave_step(void)
{
    int rc;
    if (!busy)
        return FMSAVE_IDLE;
    if (step < SAVE_SECTORS) {
        uint32_t p = step * SAVE_SECTOR_DATA, end = p + SAVE_SECTOR_DATA;
        if (end > SAVE_LEN)
            end = SAVE_LEN;
        rc = fmzx_flash_erase4k(FMZX_SAVE_FLASH + step * SAVE_SECTOR);
        if (rc == 0) {
            if (p < SAVE_HEADER_SIZE)
                p = SAVE_HEADER_SIZE;
            if (p < end) {
                /* the RAM part of this share, for the CRC (the .z80 header is in it already) */
                uint32_t z = p - SAVE_HEADER_SIZE;
                uint32_t from = z < SAVE_Z80_HEADER ? SAVE_Z80_HEADER : z;
                crc_add(ram_bytes + (from - SAVE_Z80_HEADER), (end - SAVE_HEADER_SIZE) - from);
                rc = program(p, end);
            }
        }
    } else {
        info.z80_crc = ~crc;
        save_header_build(&info, hdr_save);
        rc = fmzx_flash_write(FMZX_SAVE_FLASH, hdr_save, SAVE_HEADER_SIZE);
    }
    step++;
    if (rc != 0) {
        busy = 0;
        return FMSAVE_FAILED;
    }
    if (step < STEPS)
        return FMSAVE_BUSY;
    busy = 0;
    return FMSAVE_DONE;
}

int fmsave_finish(void)
{
    int r = FMSAVE_IDLE;
    while (busy)
        r = fmsave_step();
    return r;
}

int fmsave_busy(void)
{
    return busy;
}

/* The header at the area's start, if it is a valid save for the pack. */
static int read_header(const uint8_t *pack_base, save_info *out)
{
    uint8_t h[SAVE_HEADER_SIZE];
    if (!pack_base || fmzx_flash_read(FMZX_SAVE_FLASH, h, SAVE_HEADER_SIZE) != 0)
        return ZX_ERR_FORMAT;
    return save_header_check(h, pack_base, out);
}

uint32_t fmsave_find(const uint8_t *pack_base)
{
    save_info s;
    return read_header(pack_base, &s) == ZX_OK ? s.game : SAVE_NONE;
}

int fmsave_load_z80(const uint8_t *pack_base, uint8_t *buf, uint32_t cap, save_info *out)
{
    uint32_t p = SAVE_HEADER_SIZE;
    if (cap < SAVE_Z80_LEN || read_header(pack_base, out) != ZX_OK)
        return ZX_ERR_FORMAT;
    while (p < SAVE_LEN) {
        uint32_t n = SAVE_SECTOR_DATA - p % SAVE_SECTOR_DATA;
        if (n > SAVE_LEN - p)
            n = SAVE_LEN - p;
        if (fmzx_flash_read(FMZX_SAVE_FLASH + save_area_offset(p), buf + (p - SAVE_HEADER_SIZE), n) != 0)
            return ZX_ERR_FORMAT;
        p += n;
    }
    return pack_crc32(buf, SAVE_Z80_LEN) == out->z80_crc ? ZX_OK : ZX_ERR_FORMAT;
}

void fmsave_forget(void)
{
    static uint8_t zeros[8]; /* RAM: the flash programs from RAM only */
    (void)fmzx_flash_write(FMZX_SAVE_FLASH, zeros, sizeof zeros);
}
