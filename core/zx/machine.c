/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include <string.h>

#include "zx.h"
#include "zx_internal.h"

uint8_t zx_peek(const zx_machine *m, uint16_t addr)
{
	return zx_mem_read(m, addr);
}

void zx_poke(zx_machine *m, uint16_t addr, uint8_t value)
{
	zx_mem_write(m, addr, value);
}

void zx_init(zx_machine *m, const uint8_t *rom)
{
#ifdef ZX_TESTING
	zx_trace_fn trace_fn = m->trace_fn;
	void *trace_ctx = m->trace_ctx;
	uint8_t trace_mask = m->trace_mask;
#endif

	memset(m, 0, sizeof *m);   /* also tape_ear and tape_sig: NULL, no tape signal */
	m->rom = rom;
#ifdef ZX_TESTING
	m->trace_fn = trace_fn;
	m->trace_ctx = trace_ctx;
	m->trace_mask = trace_mask;
#endif
	memset(m->dirty, 0xFF, sizeof m->dirty);
	m->force_dirty = 1;

	zx_timing_init(m);
	zx_ula_init(m);
	z80_power(&m->cpu, 1);
}

#ifdef ZX_TESTING
void zx_trace_set(zx_machine *m, zx_trace_fn fn, void *ctx, uint8_t mask)
{
	m->trace_fn = fn;
	m->trace_ctx = ctx;
	m->trace_mask = mask;
}
#endif

/* INT is asserted at T=0 and released once ZX_INT_TSTATES have run. The frame
 * is run in two slices so the line drops at the first instruction boundary at
 * or after T=32; the overshoot of the last instruction carries over. */
void zx_run_frame(zx_machine *m)
{
	uint32_t t = m->frame_tstate;

	zx_tape_frame(m);
	zx_ula_new_frame(m);
	zx_screen_new_frame(m);
	z80_int(&m->cpu, 1);
	if (t < ZX_INT_TSTATES) {
		m->run_base = t;
		t += (uint32_t)z80_run(&m->cpu, ZX_INT_TSTATES - t);
	}
	z80_int(&m->cpu, 0);
	if (t < ZX_FRAME_TSTATES) {
		m->run_base = t;
		t += (uint32_t)z80_run(&m->cpu, ZX_FRAME_TSTATES - t);
	}

	m->frame_tstate = t - ZX_FRAME_TSTATES;
	m->frame_count++;
	if (m->insn_open) m->insn_t0 -= ZX_FRAME_TSTATES;
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
	c->r7 = r->r;
	c->halt_line = 0;
	c->resume = 0;
	c->request = 0;
	c->int_line = 0;
	c->q = 0;
	Z80_MEMPTR(*c) = 0;
	m->insn_open = 0;
}

uint32_t zx_frame_tstate(const zx_machine *m)
{
	return m->frame_tstate;
}

uint32_t zx_frame_count(const zx_machine *m)
{
	return m->frame_count;
}
