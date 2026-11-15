/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* ULA ports: keyboard, Kempston, border, beeper edges and the floating bus.
 *
 * Every change is stamped with m->io_t, the frame T-state at which the I/O
 * M-cycle starts (timing.c). A change at or past ZX_FRAME_TSTATES, made by the
 * instruction that overshoots the frame end, is held back and belongs to the
 * next frame at t - ZX_FRAME_TSTATES.
 *
 * Border: a change at T applies from line ceil(T / 224). Lines are filled
 * lazily, only when the colour changes, so a tape loader writing thousands of
 * times a frame costs at most ZX_FRAME_LINES byte stores in all. */

#include <string.h>

#include "zx.h"
#include "zx_internal.h"

void zx_ula_init(zx_machine *m)
{
	m->border_col = 7;
	memset(m->border_lines, 7, sizeof m->border_lines);
}

static void border_apply(zx_machine *m, uint32_t t, uint8_t colour)
{
	uint32_t line = (t + ZX_LINE_TSTATES - 1u) / ZX_LINE_TSTATES;

	if (line > ZX_FRAME_LINES) line = ZX_FRAME_LINES;
	if (line < m->border_filled) line = m->border_filled;
	while (m->border_filled < line) m->border_lines[m->border_filled++] = m->border_col;
	m->border_col = colour;
}

/* Records an edge and counts it for its OUT's site; returns the register slot of that
 * site, or NULL when the edge is dropped (list full) or the site table has no room. */
static zx_regs *edge_apply(zx_machine *m, uint32_t t, uint8_t level, uint16_t pc)
{
	unsigned i;

	if (m->edge_count >= ZX_MAX_EDGES) return NULL;
	m->edges[m->edge_count].tstate = t;
	m->edges[m->edge_count].level = level;
	m->edges[m->edge_count].pc = pc;
	m->edge_count++;
	for (i = 0; i < m->site_count; i++) {
		if (m->sites[i].pc == pc) {
			m->sites[i].count++;
			return &m->sites[i].regs;
		}
	}
	if (m->site_count >= ZX_OUT_SITES) return NULL;
	m->sites[m->site_count].pc = pc;
	m->sites[m->site_count].count = 1u;
	return &m->sites[m->site_count++].regs;
}

void zx_ula_new_frame(zx_machine *m)
{
	unsigned i;

	m->start_level = m->base_level;
	m->edge_count = 0;
	m->site_count = 0;
	m->border_filled = 0;   /* border_col is the colour in effect at the last frame's end */
	for (i = 0; i < m->pend_border_count; i++)
		border_apply(m, m->pend_borders[i].tstate, m->pend_borders[i].level);
	for (i = 0; i < m->pend_edge_count; i++) {
		zx_regs *slot = edge_apply(m, m->pend_edges[i].tstate, m->pend_edges[i].level, m->pend_edges[i].pc);

		if (slot) *slot = m->pend_regs[i];
	}
	for (i = 0; i < m->pend_edge_count; i++) m->base_level = m->pend_edges[i].level;
	m->pend_border_count = 0;
	m->pend_edge_count = 0;
}

void zx_port_out(zx_machine *m, uint16_t port, uint8_t value)
{
	uint32_t t = m->io_t;
	uint8_t level = (uint8_t)((value >> 3) & 3u);
	uint8_t colour = value & 7u;
	int late = t >= ZX_FRAME_TSTATES;

#ifdef ZX_TESTING
	/* host only: an OUT a 128K takes as its paging port 7FFDh (A15 = 0, A1 = 0; never OUT (FEh))
	 * that pages RAM bank 1-7 in or shows the second screen (bits 0-3). Bank 0 with the ROM or
	 * lock bits alone (10h, 30h, 00h) is what 48K games write to run as 48K on a 128K. */
	if (!(port & 0x8002u) && (value & 0x0Fu)) m->test_out_7ffd++;
#endif
	if (port & 1u) return;
	m->ear_in = (uint8_t)((value >> 4) & 1u);

	if (!late) {
		border_apply(m, t, colour);
	} else if (m->pend_border_count < 4u) {
		m->pend_borders[m->pend_border_count].tstate = t - ZX_FRAME_TSTATES;
		m->pend_borders[m->pend_border_count++].level = colour;
	}

	if (level != m->level) {
		m->level = level;
		if (!late) {
			zx_regs *slot;

			m->base_level = level;
			slot = edge_apply(m, t, level, m->insn_pc);
			if (slot) {
				zx_get_regs(m, slot);
				slot->pc = m->insn_pc;
			}
		} else if (m->pend_edge_count < 4u) {
			unsigned n = m->pend_edge_count++;

			m->pend_edges[n].tstate = t - ZX_FRAME_TSTATES;
			m->pend_edges[n].level = level;
			m->pend_edges[n].pc = m->insn_pc;
			zx_get_regs(m, &m->pend_regs[n]);
			m->pend_regs[n].pc = m->insn_pc;
		}
	}
}

/* What the ULA has on the bus at frame T-state t.
 *
 * Relative to the first contended T-state (14335, the same origin zx_contention
 * uses) paper line y spans [14335 + 224y, +224); its first 128 T-states are the
 * 16 groups of 8 in which the ULA reads two bitmap/attribute pairs: phase 0
 * bitmap, 1 attribute, 2 next bitmap, 3 next attribute, 4-7 idle. The CPU
 * contention pattern 6,5,4,3,2,1,0,0 delays an access in phases 0-5 to phase 6,
 * which is where the bus goes idle. Idle, border and retrace read 0xFF. */
static uint8_t floating_bus(const zx_machine *m, uint32_t t)
{
	uint32_t r = t - ZX_FIRST_CONTENDED;   /* wraps to a huge value before the paper */
	uint32_t y, x, col;
	uint32_t phase;

	if (r >= ZX_SCREEN_LINES * ZX_LINE_TSTATES) return 0xFF;
	y = r / ZX_LINE_TSTATES;
	x = r - y * ZX_LINE_TSTATES;
	if (x >= 128u) return 0xFF;
	phase = x & 7u;
	if (phase >= 4u) return 0xFF;
	col = (x >> 3) * 2u + (phase >> 1);
	if (phase & 1u)
		return zx_mem_read(m, (uint16_t)(0x5800u + (y >> 3) * 32u + col));
	return zx_mem_read(m, (uint16_t)(0x4000u | ((y & 0xC0u) << 5) | ((y & 7u) << 8)
	                                 | ((y & 0x38u) << 2) | col));
}

uint8_t zx_port_in(zx_machine *m, uint16_t port)
{
	if (!(port & 1u)) {
		uint8_t keys = 0x1F;
		unsigned r;
		uint8_t ear;

		for (r = 0; r < 8u; r++)
			if (!((port >> (8u + r)) & 1u)) keys &= (uint8_t)~m->keys[r];
		/* EAR: the tape signal while one plays (host only), else the last OUT's bit 4 */
		ear = m->tape_ear ? m->tape_ear(m, m->io_t) : m->ear_in;
		return (uint8_t)(keys | 0xA0u | (ear << 6));
	}
	if ((port & 0xFFu) == 0x1Fu) return m->kempston;
	return floating_bus(m, m->io_t);
}

void zx_set_keys(zx_machine *m, const uint8_t rows[8])
{
	memcpy(m->keys, rows, 8);
}

void zx_set_kempston(zx_machine *m, uint8_t bits)
{
	m->kempston = bits;
}

const zx_out_site *zx_out_sites(const zx_machine *m, uint32_t *count)
{
	*count = m->site_count;
	return m->sites;
}

const zx_edge *zx_edges(const zx_machine *m, uint32_t *count)
{
	*count = m->edge_count;
	return m->edges;
}

uint8_t zx_edges_start_level(const zx_machine *m)
{
	return m->start_level;
}

uint8_t zx_border_line(const zx_machine *m, unsigned frame_line)
{
	if (frame_line < m->border_filled) return m->border_lines[frame_line];
	return m->border_col;
}
