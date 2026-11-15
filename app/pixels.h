/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Palette indices to LCD bytes, and composing one LCD row from a ZX frame. */

#ifndef APP_PIXELS_H
#define APP_PIXELS_H

#include <stdint.h>

#include "zx.h"

#define LCD_W 240u
#define LCD_ROWS 240u
#define LCD_TOP_BORDER 24u /* LCD rows 0-23 = frame lines 40-63; 216-239 = frame lines 256-279 */
#define LCD_MAX_ROW_BYTES 480u

enum { PIX_RGB444 = 0, PIX_RGB565 = 1 };

/* The 8-bit R, G, B of palette index i (bit 0 blue, bit 1 red, bit 2 green, bit 3 bright): a lit
 * channel is 0xD7 normally and 0xFF when bright. */
#define PIX_CH8(i, bit) (((i) & (bit)) ? (((i) & 8) ? 0xFFu : 0xD7u) : 0u)
#define PIX_R8(i) PIX_CH8(i, 2)
#define PIX_G8(i) PIX_CH8(i, 4)
#define PIX_B8(i) PIX_CH8(i, 1)

/* Screen modes: Crop (240 of 256 columns from crop_x), Fit (one column in 16 dropped),
 * Fit smooth (256 columns area-weighted to 240), Border (the paper with 8 border pixels each side,
 * 272 x 240, area-weighted to 240 x 212 at LCD rows 14-225). */
enum { PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH, PIX_BORDER, PIX_MODE_COUNT };

unsigned pix_row_bytes(int fmt); /* 360 or 480 */
void pix_pack(int fmt, const uint8_t idx[LCD_W], uint8_t *out); /* palette indices 0-15 -> panel bytes */
void pix_compose_zx(const zx_machine *m, unsigned row, unsigned crop_x, uint8_t idx[LCD_W]);

/* The screen modes are in pixmodes.c. */

/* LCD row `row` (0-239) of the game picture in `mode`, as panel bytes (pix_row_bytes(fmt) of them).
 * crop_x is used by Crop only. Crop's bytes equal pix_compose_zx + pix_pack, and any mode other
 * than Fit, Fit smooth and Border composes as Crop. */
void pix_compose_row(const zx_machine *m, unsigned row, unsigned mode, unsigned crop_x, int fmt, uint8_t *out);
/* Rows row0 .. row0 + rows - 1 (within 0-239), as pix_compose_row composes each, back to back in out.
 * Faster than row by row in Border, where consecutive rows share source lines. */
void pix_compose_rows(const zx_machine *m, unsigned row0, unsigned rows, unsigned mode, unsigned crop_x, int fmt,
	uint8_t *out);
/* Marks in dirty[LCD_ROWS/8] (bit r&7 of byte r>>3) every LCD row whose bytes in `mode` depend on
 * paper line y (0-191). */
void pix_mark_paper_line(unsigned mode, unsigned y, uint8_t dirty[LCD_ROWS / 8u]);
/* Marks every LCD row whose bytes in `mode` depend on the border colour of frame line `line`. */
void pix_mark_border_line(unsigned mode, unsigned line, uint8_t dirty[LCD_ROWS / 8u]);

#endif
