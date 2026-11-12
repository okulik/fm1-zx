/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include <string.h>

#include "zx.h"

static uint8_t *ram_bytes(zx_machine *m)
{
	return (uint8_t *)m->ram_words;
}

static const uint8_t *ram_bytes_const(const zx_machine *m)
{
	return (const uint8_t *)m->ram_words;
}

uint8_t zx_peek(const zx_machine *m, uint16_t addr)
{
	if (addr < ZX_ROM_SIZE) return m->rom[addr];
	return ram_bytes_const(m)[addr - ZX_ROM_SIZE];
}

void zx_poke(zx_machine *m, uint16_t addr, uint8_t value)
{
	if (addr < ZX_ROM_SIZE) return;
	ram_bytes(m)[addr - ZX_ROM_SIZE] = value;
}

static uint8_t cpu_read(void *context, uint16_t address)
{
	return zx_peek((const zx_machine *)context, address);
}

static void cpu_write(void *context, uint16_t address, uint8_t value)
{
	zx_poke((zx_machine *)context, address, value);
}

/* No port devices yet: reads float high, writes go nowhere. */
static uint8_t cpu_in(void *context, uint16_t port)
{
	(void)context;
	(void)port;
	return 0xFF;
}

static void cpu_out(void *context, uint16_t port, uint8_t value)
{
	(void)context;
	(void)port;
	(void)value;
}

void zx_init(zx_machine *m, const uint8_t *rom)
{
	memset(m, 0, sizeof *m);
	m->rom = rom;

	/* Optional callbacks (halt, nop, nmia, inta, int_fetch, ld_i_a, ld_r_a,
	 * reti, retn, hook, illegal) stay NULL; a NULL inta reads 0xFF from the
	 * data bus, which is what the 48K puts there. */
	m->cpu.context = m;
	m->cpu.fetch_opcode = cpu_read;
	m->cpu.fetch = cpu_read;
	m->cpu.read = cpu_read;
	m->cpu.write = cpu_write;
	m->cpu.in = cpu_in;
	m->cpu.out = cpu_out;
	m->cpu.options = Z80_MODEL_ZILOG_NMOS;
	z80_power(&m->cpu, 1);
}

void zx_run_frame(zx_machine *m)
{
	uint32_t target = ZX_FRAME_TSTATES - m->frame_tstate;
	uint32_t ran = (uint32_t)z80_run(&m->cpu, target);

	m->frame_tstate = ran - target;
	m->frame_count++;
}

void zx_get_regs(const zx_machine *m, zx_regs *r)
{
	const Z80 *c = &m->cpu;

	r->af = Z80_AF(*c);   r->bc = Z80_BC(*c);
	r->de = Z80_DE(*c);   r->hl = Z80_HL(*c);
	r->af_ = Z80_AF_(*c); r->bc_ = Z80_BC_(*c);
	r->de_ = Z80_DE_(*c); r->hl_ = Z80_HL_(*c);
	r->ix = Z80_IX(*c);   r->iy = Z80_IY(*c);
	r->sp = Z80_SP(*c);   r->pc = Z80_PC(*c);
	r->i = c->i;          r->r = c->r;
	r->iff1 = c->iff1;    r->iff2 = c->iff2;
	r->im = c->im;
}

void zx_set_regs(zx_machine *m, const zx_regs *r)
{
	Z80 *c = &m->cpu;

	Z80_AF(*c) = r->af;   Z80_BC(*c) = r->bc;
	Z80_DE(*c) = r->de;   Z80_HL(*c) = r->hl;
	Z80_AF_(*c) = r->af_; Z80_BC_(*c) = r->bc_;
	Z80_DE_(*c) = r->de_; Z80_HL_(*c) = r->hl_;
	Z80_IX(*c) = r->ix;   Z80_IY(*c) = r->iy;
	Z80_SP(*c) = r->sp;   Z80_PC(*c) = r->pc;
	c->i = r->i;          c->r = r->r;
	c->iff1 = r->iff1;    c->iff2 = r->iff2;
	c->im = r->im;
}

uint32_t zx_frame_tstate(const zx_machine *m)
{
	return m->frame_tstate;
}

uint32_t zx_frame_count(const zx_machine *m)
{
	return m->frame_count;
}
