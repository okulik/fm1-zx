/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "pixels.h"

#include <string.h>

/* Palette index i: bit 0 blue, bit 1 red, bit 2 green, bit 3 bright.
 * A lit channel is 0xD7 normally and 0xFF when bright. */
#define CH8(i, bit) (((i) & (bit)) ? (((i) & 8) ? 0xFFu : 0xD7u) : 0u)
#define R8(i) CH8(i, 2)
#define G8(i) CH8(i, 4)
#define B8(i) CH8(i, 1)

#define N4(v) ((v) >> 4)
#define R4(i) N4(R8(i))
#define G4(i) N4(G8(i))
#define B4(i) N4(B8(i))

#define C565(i) ((uint16_t)(((R8(i) >> 3) << 11) | ((G8(i) >> 2) << 5) | (B8(i) >> 3)))

/* Two pixels a, b in three bytes: R0G0, B0R1, G1B1. */
#define PAIR(a, b) {(uint8_t)((R4(a) << 4) | G4(a)), (uint8_t)((B4(a) << 4) | R4(b)), (uint8_t)((G4(b) << 4) | B4(b))}
#define PAIR_ROW(a) \
	PAIR(a, 0), PAIR(a, 1), PAIR(a, 2), PAIR(a, 3), PAIR(a, 4), PAIR(a, 5), PAIR(a, 6), PAIR(a, 7), \
	PAIR(a, 8), PAIR(a, 9), PAIR(a, 10), PAIR(a, 11), PAIR(a, 12), PAIR(a, 13), PAIR(a, 14), PAIR(a, 15)

/* Indexed by (first << 4) | second. */
static const uint8_t pair444[256][3] = {
	PAIR_ROW(0), PAIR_ROW(1), PAIR_ROW(2), PAIR_ROW(3), PAIR_ROW(4), PAIR_ROW(5), PAIR_ROW(6), PAIR_ROW(7),
	PAIR_ROW(8), PAIR_ROW(9), PAIR_ROW(10), PAIR_ROW(11), PAIR_ROW(12), PAIR_ROW(13), PAIR_ROW(14), PAIR_ROW(15),
};

static const uint16_t pal565[16] = {
	C565(0), C565(1), C565(2), C565(3), C565(4), C565(5), C565(6), C565(7),
	C565(8), C565(9), C565(10), C565(11), C565(12), C565(13), C565(14), C565(15),
};

unsigned pix_row_bytes(int fmt)
{
	return fmt == PIX_RGB565 ? LCD_W * 2u : LCD_W * 3u / 2u;
}

void pix_pack(int fmt, const uint8_t idx[LCD_W], uint8_t *out)
{
	unsigned x;
	if (fmt == PIX_RGB565) {
		for (x = 0; x < LCD_W; x++) {
			uint16_t c = pal565[idx[x] & 15u];
			*out++ = (uint8_t)(c >> 8);
			*out++ = (uint8_t)c;
		}
		return;
	}
	for (x = 0; x < LCD_W; x += 2) {
		const uint8_t *p = pair444[((idx[x] & 15u) << 4) | (idx[x + 1] & 15u)];
		*out++ = p[0];
		*out++ = p[1];
		*out++ = p[2];
	}
}

void pix_compose_zx(const zx_machine *m, unsigned row, unsigned crop_x, uint8_t idx[LCD_W])
{
	uint8_t line[ZX_SCREEN_WIDTH];
	unsigned y;

	if (crop_x > ZX_SCREEN_WIDTH - LCD_W)
		crop_x = ZX_SCREEN_WIDTH - LCD_W;
	if (row < LCD_TOP_BORDER || row >= LCD_TOP_BORDER + ZX_SCREEN_LINES) {
		/* LCD row 0 is frame line 40 */
		memset(idx, zx_border_line(m, ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER + row), LCD_W);
		return;
	}
	y = row - LCD_TOP_BORDER;
	zx_screen_line(m, y, line);
	memcpy(idx, line + crop_x, LCD_W);
}
