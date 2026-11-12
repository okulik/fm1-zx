/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* .sna and .z80 snapshot loading (48K only).
 *
 * Every load parses the whole file twice with the same code: the first pass
 * (commit == 0) only checks it and collects the registers, so a bad file
 * changes nothing; the second decodes straight into RAM, which cannot fail
 * any more. No scratch copy of the 48 KB is needed. */

#include <string.h>

#include "zx.h"
#include "zx_internal.h"

#define SNA_HEADER 27u
#define SNA_48K_SIZE (SNA_HEADER + ZX_RAM_SIZE)
#define SNA_128K_SIZE 131103u         /* 128K .sna, 6 banks after the 3 fixed ones */
#define SNA_128K_SIZE_TRDOS 147487u   /* the same with a repeated bank */
#define Z80_HEADER 30u
#define Z80_PAGE_SIZE 16384u

typedef struct {
	zx_regs r;
	uint8_t border;
} snap;

static inline uint16_t le16(const uint8_t *p)
{
	return (uint16_t)(p[0] | (p[1] << 8));
}

static inline uint8_t *ram_bytes(zx_machine *m)
{
	return (uint8_t *)m->ram_words;
}

/* Decodes the ED ED nn bb run-length stream src[0..len) into exactly `want`
 * bytes (dst may be NULL to only check). With `marker` the stream must be
 * followed by 00 ED ED 00 or by nothing (version 1); without, it must end exactly there
 * (a version 2/3 page). Returns 0 or ZX_ERR_FORMAT. */
static int rle_decode(const uint8_t *src, uint32_t len, uint8_t *dst, uint32_t want, int marker)
{
	uint32_t p = 0, out = 0;

	while (out < want) {
		if (p >= len) return ZX_ERR_FORMAT;
		if (src[p] == 0xEDu && p + 1u < len && src[p + 1u] == 0xEDu) {
			uint32_t n;

			if (len - p < 4u) return ZX_ERR_FORMAT;
			n = src[p + 2u];
			if (n == 0 || n > want - out) return ZX_ERR_FORMAT;
			if (dst) memset(dst + out, src[p + 3u], n);
			out += n;
			p += 4u;
		} else {
			if (dst) dst[out] = src[p];
			out++;
			p++;
		}
	}
	if (marker) {
		if (p == len) return 0;   /* some writers omit the end marker */
		if (len - p < 4u) return ZX_ERR_FORMAT;
		if (src[p] != 0x00u || src[p + 1u] != 0xEDu || src[p + 2u] != 0xEDu || src[p + 3u] != 0x00u)
			return ZX_ERR_FORMAT;
		return 0;
	}
	return p == len ? 0 : ZX_ERR_FORMAT;
}

/* Puts the machine in the state a freshly loaded snapshot has: new registers,
 * a clean CPU and clock, the next frame starting at T=0, the border colour in
 * effect from line 0, no pending beeper or border changes, all lines dirty. */
static void commit(zx_machine *m, const snap *s)
{
	zx_set_regs(m, &s->r);
	m->cpu.halt_line = 0;
	m->cpu.resume = 0;
	m->cpu.request = 0;
	m->cpu.int_line = 0;
	m->cpu.q = 0;
	m->cpu.r7 = s->r.r;

	m->frame_tstate = 0;
	m->run_base = 0;
	m->insn_t0 = 0;
	m->wait = 0;
	m->insn_open = 0;
	m->io_t = 0;

	m->border_col = s->border;
	m->border_filled = 0;
	memset(m->border_lines, s->border, sizeof m->border_lines);
	m->edge_count = 0;
	m->pend_edge_count = 0;
	m->pend_border_count = 0;
	m->level = 0;
	m->ear_in = 0;

	memset(m->dirty, 0xFF, sizeof m->dirty);
}

/* ---- .sna ---------------------------------------------------------------- */

static int sna_parse(zx_machine *m, const uint8_t *d, uint32_t len, int do_commit, snap *s)
{
	zx_regs *r = &s->r;
	uint16_t sp;
	uint8_t lo, hi;

	if (len == SNA_128K_SIZE || len == SNA_128K_SIZE_TRDOS) return ZX_ERR_UNSUPPORTED;
	if (len != SNA_48K_SIZE) return ZX_ERR_FORMAT;
	if (d[25] > 2u) return ZX_ERR_FORMAT;

	r->i = d[0];
	r->hl_ = le16(d + 1);
	r->de_ = le16(d + 3);
	r->bc_ = le16(d + 5);
	r->af_ = le16(d + 7);
	r->hl = le16(d + 9);
	r->de = le16(d + 11);
	r->bc = le16(d + 13);
	r->iy = le16(d + 15);
	r->ix = le16(d + 17);
	r->iff1 = r->iff2 = (d[19] >> 2) & 1u;
	r->r = d[20];
	r->af = le16(d + 21);
	sp = le16(d + 23);
	r->im = d[25];
	s->border = d[26] & 7u;

	/* PC is on the stack; the bytes stay in RAM. Below 0x4000 it is the ROM's. */
	lo = sp >= ZX_ROM_SIZE ? d[SNA_HEADER + (sp - ZX_ROM_SIZE)] : m->rom[sp];
	{
		uint16_t sp1 = (uint16_t)(sp + 1u);

		hi = sp1 >= ZX_ROM_SIZE ? d[SNA_HEADER + (sp1 - ZX_ROM_SIZE)] : m->rom[sp1];
	}
	r->pc = (uint16_t)(lo | (hi << 8));
	r->sp = (uint16_t)(sp + 2u);

	if (do_commit) memcpy(ram_bytes(m), d + SNA_HEADER, ZX_RAM_SIZE);
	return 0;
}

int zx_load_sna(zx_machine *m, const uint8_t *data, uint32_t len)
{
	snap s;
	int e = sna_parse(m, data, len, 0, &s);

	if (e) return e;
	sna_parse(m, data, len, 1, &s);
	commit(m, &s);
	return 0;
}

/* ---- .z80 ---------------------------------------------------------------- */

/* Offset in RAM of a 48K memory page, or -1 when the page is ROM data
 * (ignored) and -2 when it does not exist on a 48K machine. */
static int32_t z80_page_base(uint8_t page)
{
	switch (page) {
	case 4: return 0x4000;    /* 0x8000 */
	case 5: return 0x8000;    /* 0xC000 */
	case 8: return 0x0000;    /* 0x4000 */
	case 0: case 1: case 2: return -1;
	default: return -2;
	}
}

static int z80_parse(zx_machine *m, const uint8_t *d, uint32_t len, int do_commit, snap *s)
{
	zx_regs *r = &s->r;
	uint8_t flags;
	uint16_t pc;
	uint32_t p, ext, seen;

	if (len < Z80_HEADER) return ZX_ERR_FORMAT;
	flags = d[12];
	if (flags == 255u) flags = 1;

	r->af = (uint16_t)((d[0] << 8) | d[1]);
	r->bc = le16(d + 2);
	r->hl = le16(d + 4);
	pc = le16(d + 6);
	r->sp = le16(d + 8);
	r->i = d[10];
	r->r = (uint8_t)((d[11] & 0x7Fu) | ((flags & 1u) << 7));
	s->border = (flags >> 1) & 7u;
	r->de = le16(d + 13);
	r->bc_ = le16(d + 15);
	r->de_ = le16(d + 17);
	r->hl_ = le16(d + 19);
	r->af_ = (uint16_t)((d[21] << 8) | d[22]);
	r->iy = le16(d + 23);
	r->ix = le16(d + 25);
	r->iff1 = d[27] ? 1u : 0u;
	r->iff2 = d[28] ? 1u : 0u;
	r->im = d[29] & 3u;
	if (r->im > 2u) return ZX_ERR_FORMAT;

	if (pc != 0) {   /* version 1: 48 KB follow, compressed or not */
		const uint8_t *src = d + Z80_HEADER;
		uint32_t n = len - Z80_HEADER;
		uint8_t *dst = do_commit ? ram_bytes(m) : NULL;
		int e;

		r->pc = pc;
		if (flags & 0x20u) {
			e = rle_decode(src, n, dst, ZX_RAM_SIZE, 1);
			if (e) return e;
		} else {
			if (n != ZX_RAM_SIZE) return ZX_ERR_FORMAT;
			if (dst) memcpy(dst, src, ZX_RAM_SIZE);
		}
		return 0;
	}

	/* Version 2 or 3: extended header, then pages. */
	if (len < Z80_HEADER + 2u) return ZX_ERR_FORMAT;
	ext = le16(d + 30);
	if (ext != 23u && ext != 54u && ext != 55u) return ZX_ERR_FORMAT;
	if (len - (Z80_HEADER + 2u) < ext) return ZX_ERR_FORMAT;
	r->pc = le16(d + 32);
	{
		uint8_t hw = d[34];
		int ok = ext == 23u ? (hw == 0 || hw == 1) : (hw == 0 || hw == 1 || hw == 3);

		if (!ok) return ZX_ERR_UNSUPPORTED;
		if (ext != 23u && (d[37] & 0x80u)) return ZX_ERR_UNSUPPORTED;   /* modified hardware */
	}

	p = Z80_HEADER + 2u + ext;
	seen = 0;
	while (p < len) {
		uint32_t blen, size;
		int32_t base;
		uint8_t page;

		if (len - p < 3u) return ZX_ERR_FORMAT;
		blen = le16(d + p);
		page = d[p + 2u];
		p += 3u;
		size = blen == 0xFFFFu ? Z80_PAGE_SIZE : blen;
		if (size > len - p) return ZX_ERR_FORMAT;
		base = z80_page_base(page);
		if (base == -2) return ZX_ERR_FORMAT;
		if (base >= 0) {
			uint8_t *dst = do_commit ? ram_bytes(m) + base : NULL;

			if (blen == 0xFFFFu) {
				if (dst) memcpy(dst, d + p, Z80_PAGE_SIZE);
			} else {
				int e = rle_decode(d + p, size, dst, Z80_PAGE_SIZE, 0);

				if (e) return e;
			}
			seen |= 1u << page;
		}
		p += size;
	}
	return seen == ((1u << 4) | (1u << 5) | (1u << 8)) ? 0 : ZX_ERR_FORMAT;
}

int zx_load_z80(zx_machine *m, const uint8_t *data, uint32_t len)
{
	snap s;
	int e = z80_parse(m, data, len, 0, &s);

	if (e) return e;
	z80_parse(m, data, len, 1, &s);
	commit(m, &s);
	return 0;
}
