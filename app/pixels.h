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

unsigned pix_row_bytes(int fmt); /* 360 or 480 */
void pix_pack(int fmt, const uint8_t idx[LCD_W], uint8_t *out); /* palette indices 0-15 -> panel bytes */
void pix_compose_zx(const zx_machine *m, unsigned row, unsigned crop_x, uint8_t idx[LCD_W]);

#endif
