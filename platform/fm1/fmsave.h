/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Writing the saved game to flash (layout: docs/save-format.md), one step at a time so that the
 * LCD and the buttons stay alive. The steps run from XIP; the flash driver is in usbota.c. */
#ifndef FMZX_FMSAVE_H
#define FMZX_FMSAVE_H

#include <stdint.h>
#include "save.h"
#include "zx.h"

enum { FMSAVE_IDLE, FMSAVE_BUSY, FMSAVE_DONE, FMSAVE_FAILED };

/* Snapshots the registers and the border into a RAM buffer holding the 30-byte .z80 header, and
 * the save's identity (pack index, the pack entry's crc32, the tape position). The RAM is
 * written from m->ram_words in place as the steps run, so the machine must not run until
 * fmsave_step returns FMSAVE_DONE or FMSAVE_FAILED. m's PC must be non-zero. */
void fmsave_begin(const zx_machine *m, uint32_t game, uint32_t game_crc, uint32_t tape_pos);
/* Sector k of the save area, k = 0-12: erased, then its share of the save programmed at its
 * start; the step after them programs the 64-byte header last. FMSAVE_BUSY while steps remain,
 * then FMSAVE_DONE or FMSAVE_FAILED (the first failure ends the save, the header unwritten);
 * FMSAVE_IDLE when nothing is in progress. */
int fmsave_step(void);
/* Runs the remaining steps: FMSAVE_DONE, FMSAVE_FAILED, or FMSAVE_IDLE when nothing was in progress. */
int fmsave_finish(void);
int fmsave_busy(void);

/* The index of the pack game that owns the saved game (pack_base: the opened pack), or SAVE_NONE
 * when the area holds no valid save for this pack. */
uint32_t fmsave_find(const uint8_t *pack_base);
/* Reads the header again and the .z80 (SAVE_Z80_LEN bytes) into buf (cap at least that), checks
 * the CRC: ZX_OK with *out, or ZX_ERR_FORMAT. */
int fmsave_load_z80(const uint8_t *pack_base, uint8_t *buf, uint32_t cap, save_info *out);
/* Programs 8 zero bytes over the magic (no erase): the save counts as none from then on. */
void fmsave_forget(void);

#endif
