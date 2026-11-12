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

static inline void zx_mem_write(zx_machine *m, uint16_t addr, uint8_t value)
{
	if (addr >= ZX_ROM_SIZE) ((uint8_t *)m->ram_words)[addr - ZX_ROM_SIZE] = value;
}

/* Port devices (machine.c); the timing glue calls them after applying contention. */
uint8_t zx_port_in(zx_machine *m, uint16_t port);
void zx_port_out(zx_machine *m, uint16_t port, uint8_t value);

/* Installs the CPU callbacks and the instruction clock (timing.c). */
void zx_timing_init(zx_machine *m);

#endif
