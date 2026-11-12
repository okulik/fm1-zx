/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "zx.h"
#include "zx_internal.h"

#define SCREEN_BASE 0x4000u
#define ATTR_BASE 0x5800u

/* The frame about to run ends with frame_count + 1; FLASH changes phase when
 * that is a multiple of 16, so the cells with FLASH set must be redrawn. */
void zx_screen_new_frame(zx_machine *m)
{
	unsigned i;

	if (m->force_dirty) {
		m->force_dirty = 0;
		for (i = 0; i < sizeof m->dirty; i++) m->dirty[i] = 0xFF;
		return;
	}
	for (i = 0; i < sizeof m->dirty; i++) m->dirty[i] = 0;
	if (((m->frame_count + 1u) & 15u) != 0) return;
	for (i = 0; i < 768u; i++) {
		if (zx_mem_read(m, (uint16_t)(ATTR_BASE + i)) & 0x80u) m->dirty[i >> 5] = 0xFF;
	}
}

int zx_line_dirty(const zx_machine *m, unsigned y)
{
	if (y >= ZX_SCREEN_LINES) return 0;
	return (m->dirty[y >> 3] >> (y & 7u)) & 1u;
}

void zx_screen_line(const zx_machine *m, unsigned y, uint8_t out[256])
{
	uint32_t bitmap, attr;
	unsigned cell, bit;
	unsigned flash_phase;

	if (y >= ZX_SCREEN_LINES) return;
	bitmap = SCREEN_BASE + ((y & 0xC0u) << 5) + ((y & 7u) << 8) + ((y & 0x38u) << 2);
	attr = ATTR_BASE + (y >> 3) * 32u;
	flash_phase = (m->frame_count >> 4) & 1u;

	for (cell = 0; cell < 32; cell++) {
		uint8_t a = zx_mem_read(m, (uint16_t)(attr + cell));
		uint8_t bits = zx_mem_read(m, (uint16_t)(bitmap + cell));
		uint8_t bright = (a & 0x40u) ? 8u : 0u;
		uint8_t ink = (uint8_t)((a & 7u) + bright);
		uint8_t paper = (uint8_t)(((a >> 3) & 7u) + bright);

		if ((a & 0x80u) && flash_phase) {
			uint8_t t = ink;
			ink = paper;
			paper = t;
		}
		for (bit = 0; bit < 8; bit++) {
			out[cell * 8u + bit] = (bits & (0x80u >> bit)) ? ink : paper;
		}
	}
}
