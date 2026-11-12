/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Declarations shared between the core's own .c files. Not public API. */

#ifndef ZX_INTERNAL_H
#define ZX_INTERNAL_H

#include "zx.h"

/* Raw memory access, no timing: ROM below 0x4000, RAM above. */
static inline uint8_t zx_mem_read(const zx_machine *m, uint16_t addr)
{
	if (addr < ZX_ROM_SIZE) return m->rom[addr];
	return ((const uint8_t *)m->ram_words)[addr - ZX_ROM_SIZE];
}

/* ROM writes are ignored. A write to the screen area that changes a byte
 * marks the lines it affects dirty: a bitmap byte its line, an attribute
 * byte the 8 lines of its cell row. */
static inline void zx_mem_write(zx_machine *m, uint16_t addr, uint8_t value)
{
	uint8_t *p;
	uint32_t a;

	if (addr < ZX_ROM_SIZE) return;
	a = (uint32_t)addr - ZX_ROM_SIZE;
	p = (uint8_t *)m->ram_words + a;
	if (a < 0x1B00u && *p != value) {
		if (a < 0x1800u) {
			uint32_t y = ((a >> 8) & 7u) | ((a >> 2) & 0x38u) | ((a >> 5) & 0xC0u);
			m->dirty[y >> 3] |= (uint8_t)(1u << (y & 7u));
		} else {
			m->dirty[(a - 0x1800u) >> 5] = 0xFF;
		}
	}
	*p = value;
}

/* Port devices (ula.c); the timing glue calls them after applying contention. */
uint8_t zx_port_in(zx_machine *m, uint16_t port);
void zx_port_out(zx_machine *m, uint16_t port, uint8_t value);

/* Resets edges and border lines for a new frame (ula.c) and applies the
 * changes the previous frame's last instruction made past its end. */
void zx_ula_new_frame(zx_machine *m);
void zx_ula_init(zx_machine *m);

/* Clears the dirty lines and marks FLASH cells when the phase flips (screen.c). */
void zx_screen_new_frame(zx_machine *m);

/* Installs the CPU callbacks and the instruction clock (timing.c). */
void zx_timing_init(zx_machine *m);

#endif
