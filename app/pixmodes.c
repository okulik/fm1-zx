/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The screen modes (pixels.h): pix_compose_row and the dirty-row maps. Crop is pix_compose_zx +
 * pix_pack (pixels.c); Fit, Fit smooth and Border are here. A file of its own so that the firmware
 * can place it apart from pixels.c: pix_compose_zx runs in SRAM next to the core, this file from
 * flash, and its device copy reaches the core's accessors and pix_compose_zx through the firmware
 * (platform/fm1/CMakeLists.txt). */

#include "pixels.h"

#include <string.h>

#define BORDER_TOP 14u     /* Border: the picture at LCD rows 14-225 */
#define BORDER_ROWS 212u
#define BORDER_SRC_W 272u  /* 8 border pixels, the 256 of the paper, 8 border pixels */
#define BORDER_SRC_LINES 240u /* frame lines 40-279 */

/* Border's area filter, separable, with weights in 256ths along each axis. Along an axis, output
 * pixels of span S units cover source pixels of U units, in periods of n outputs; output j of a
 * period starts at unit S*j and its first source pixel is first = floor(S*j/U). With C_k the
 * overlap of the output with source pixels first..first+k, in units, weight k is
 * round(256*C_k/S) - round(256*C_{k-1}/S) (round(v) = floor(v + 1/2), C_-1 = 0), so a pixel's
 * weights sum to 256 and a flat area stays exact. Each entry is {first, w0, w1, w2}.
 * Columns: U = 15, S = 17, n = 15 (17 source columns a period): border_period below, which has
 * the 15 entries written out (an entry with w2 = 0 blends two columns):
 *   {0, 226, 30, 0}, {1, 196, 60, 0}, {2, 166, 90, 0}, {3, 136, 120, 0}, {4, 105, 151, 0},
 *   {5, 75, 181, 0}, {6, 45, 211, 0}, {7, 15, 226, 15}, {9, 211, 45, 0}, {10, 181, 75, 0},
 *   {11, 151, 105, 0}, {12, 120, 136, 0}, {13, 90, 166, 0}, {14, 60, 196, 0}, {15, 30, 226, 0}.
 * Lines: U = 53, S = 60, n = 53 (60 source lines a period): 240 lines to 212 rows. */
static const uint8_t border_v[53][4] = {
	{0, 226, 30, 0}, {1, 196, 60, 0}, {2, 166, 90, 0}, {3, 137, 119, 0}, {4, 107, 149, 0},
	{5, 77, 179, 0}, {6, 47, 209, 0}, {7, 17, 226, 13}, {9, 213, 43, 0}, {10, 183, 73, 0},
	{11, 154, 102, 0}, {12, 124, 132, 0}, {13, 94, 162, 0}, {14, 64, 192, 0}, {15, 34, 222, 0},
	{16, 4, 226, 26}, {18, 201, 55, 0}, {19, 171, 85, 0}, {20, 141, 115, 0}, {21, 111, 145, 0},
	{22, 81, 175, 0}, {23, 51, 205, 0}, {24, 21, 226, 9}, {26, 218, 38, 0}, {27, 188, 68, 0},
	{28, 158, 98, 0}, {29, 128, 128, 0}, {30, 98, 158, 0}, {31, 68, 188, 0}, {32, 38, 218, 0},
	{33, 9, 226, 21}, {35, 205, 51, 0}, {36, 175, 81, 0}, {37, 145, 111, 0}, {38, 115, 141, 0},
	{39, 85, 171, 0}, {40, 55, 201, 0}, {41, 26, 226, 4}, {43, 222, 34, 0}, {44, 192, 64, 0},
	{45, 162, 94, 0}, {46, 132, 124, 0}, {47, 102, 154, 0}, {48, 73, 183, 0}, {49, 43, 213, 0},
	{50, 13, 226, 17}, {52, 209, 47, 0}, {53, 179, 77, 0}, {54, 149, 107, 0}, {55, 119, 137, 0},
	{56, 90, 166, 0}, {57, 60, 196, 0}, {58, 30, 226, 0},
};
/* The rows of a period that source line l of the period (0-59) feeds, the rows whose border_v
 * weight for it is not 0: row (v & 0x7F), and the next row too when v & 0x80. */
static const uint8_t border_line_rows[60] = {
	0, 128, 129, 130, 131, 132, 133, 134, 7, 135, 136, 137, 138, 139, 140, 141, 142, 15, 143, 144,
	145, 146, 147, 148, 149, 22, 150, 151, 152, 153, 154, 155, 156, 157, 30, 158, 159, 160, 161, 162,
	163, 164, 37, 165, 166, 167, 168, 169, 170, 171, 172, 45, 173, 174, 175, 176, 177, 178, 179, 52,
};

/* The palette for blending two channels in one word: R << 16 | G, and B. A lane holds 0-65535, so
 * palette values times weights that add up to at most 256 stay in their lanes. */
#define RG8(i) (((uint32_t)PIX_R8(i) << 16) | PIX_G8(i))
#define PAL_ROW(ch) \
	ch(0), ch(1), ch(2), ch(3), ch(4), ch(5), ch(6), ch(7), ch(8), ch(9), ch(10), ch(11), ch(12), ch(13), ch(14), ch(15)
static const uint32_t pal_rg[16] = {PAL_ROW(RG8)};
static const uint32_t pal_b[16] = {PAL_ROW(PIX_B8)};

/* The blended modes work in groups of 30 output pixels (two periods of 15), each pixel 8-bit
 * R << 16 | B << 8 | G, and pack a group to the panel's bytes as pix_pack packs: RGB565 keeps
 * 5/6/5 of the top bits, RGB444 the top 4 bits, two pixels in three bytes (R0G0, B0R1, G1B1).
 * A group starts at an even x, so its pairs are whole. Returns the end of the bytes written. */
#define GROUP 30u
static uint8_t *pack_group(int fmt, const uint32_t px[GROUP], uint8_t *out)
{
	unsigned x;
	if (fmt == PIX_RGB565) {
		for (x = 0; x < GROUP; x++) {
			uint32_t p = px[x], c = ((p >> 8) & 0xF800u) | ((p << 3) & 0x07E0u) | ((p >> 11) & 0x001Fu);
			*out++ = (uint8_t)(c >> 8);
			*out++ = (uint8_t)c;
		}
		return out;
	}
	for (x = 0; x < GROUP; x += 2u) {
		uint32_t a = px[x], b = px[x + 1u];
		*out++ = (uint8_t)(((a >> 16) & 0xF0u) | ((a >> 4) & 0x0Fu));
		*out++ = (uint8_t)(((a >> 8) & 0xF0u) | (b >> 20));
		*out++ = (uint8_t)((b & 0xF0u) | ((b >> 12) & 0x0Fu));
	}
	return out;
}

/* Fit: output column x is source column x + x/15, i.e. 15 of each 16. */
static void compose_fit(const zx_machine *m, unsigned y, uint8_t idx[LCD_W])
{
	uint8_t line[ZX_SCREEN_WIDTH];
	unsigned k, s, x = 0;
	zx_screen_line(m, y, line);
	for (k = 0; k < ZX_SCREEN_WIDTH; k += 16u)
		for (s = 0; s < 15u; s++)
			idx[x++] = line[k + s];
}

/* Fit smooth: output column x covers source columns [16x/15, 16(x+1)/15). With x = 15k + s, these
 * are columns 16k + s (weight 15 - s sixteenths) and 16k + s + 1 (weight 1 + s). Each channel is
 * (wa * a + wb * b + 8) >> 4, from the columns' rg (R << 16 | G: their sums are at most 16 * 255)
 * and bc (B). */
#define FIT_SMOOTH(s) \
	((((((15u - (s)) * rg[s] + (1u + (s)) * rg[(s) + 1u] + 0x80008u) >> 4) & 0xFF00FFu)) | \
	 ((((15u - (s)) * bc[s] + (1u + (s)) * bc[(s) + 1u] + 8u) >> 4) << 8))
#define FIT_SMOOTH_15(px) \
	do { \
		(px)[0] = FIT_SMOOTH(0); (px)[1] = FIT_SMOOTH(1); (px)[2] = FIT_SMOOTH(2); \
		(px)[3] = FIT_SMOOTH(3); (px)[4] = FIT_SMOOTH(4); (px)[5] = FIT_SMOOTH(5); \
		(px)[6] = FIT_SMOOTH(6); (px)[7] = FIT_SMOOTH(7); (px)[8] = FIT_SMOOTH(8); \
		(px)[9] = FIT_SMOOTH(9); (px)[10] = FIT_SMOOTH(10); (px)[11] = FIT_SMOOTH(11); \
		(px)[12] = FIT_SMOOTH(12); (px)[13] = FIT_SMOOTH(13); (px)[14] = FIT_SMOOTH(14); \
	} while (0)

static void compose_fit_smooth(const zx_machine *m, unsigned y, int fmt, uint8_t *out)
{
	uint8_t line[ZX_SCREEN_WIDTH];
	unsigned k, c, h;
	zx_screen_line(m, y, line);
	for (k = 0; k < ZX_SCREEN_WIDTH; k += 32u) { /* two periods of 16 columns: 30 pixels */
		uint32_t px[GROUP];
		for (h = 0; h < 2u; h++) {
			const uint8_t *l = line + k + 16u * h;
			uint32_t rg[16], bc[16];
			for (c = 1; c < 16u && l[c] == l[0]; c++)
				;
			if (c == 16u) { /* every column alike: each pixel is its colour, as the weights sum to 16 */
				unsigned i = l[0] & 15u;
				uint32_t v = pal_rg[i] | (pal_b[i] << 8);
				for (c = 0; c < 15u; c++)
					px[15u * h + c] = v;
				continue;
			}
			for (c = 0; c < 16u; c++) {
				unsigned i = l[c] & 15u;
				rg[c] = pal_rg[i];
				bc[c] = pal_b[i];
			}
			FIT_SMOOTH_15(px + 15u * h);
		}
		out = pack_group(fmt, px, out);
	}
}

/* Border, one period across: 15 output pixels from 17 columns blended down the lines (vr, vg, vb,
 * in 256ths), with the column weights above. Each channel is (sum + 32768) >> 16. */
#define BH2(v, f, w0, w1) (((w0) * (v)[f] + (w1) * (v)[(f) + 1] + 32768u) >> 16)
#define BH3(v, f, w0, w1, w2) (((w0) * (v)[f] + (w1) * (v)[(f) + 1] + (w2) * (v)[(f) + 2] + 32768u) >> 16)
#define BPX2(s, f, w0, w1) px[s] = (BH2(vr, f, w0, w1) << 16) | (BH2(vb, f, w0, w1) << 8) | BH2(vg, f, w0, w1)
static void border_period(uint32_t px[15], const uint32_t vr[17], const uint32_t vg[17], const uint32_t vb[17])
{
	BPX2(0, 0, 226u, 30u);
	BPX2(1, 1, 196u, 60u);
	BPX2(2, 2, 166u, 90u);
	BPX2(3, 3, 136u, 120u);
	BPX2(4, 4, 105u, 151u);
	BPX2(5, 5, 75u, 181u);
	BPX2(6, 6, 45u, 211u);
	px[7] = (BH3(vr, 7, 15u, 226u, 15u) << 16) | (BH3(vb, 7, 15u, 226u, 15u) << 8) | BH3(vg, 7, 15u, 226u, 15u);
	BPX2(8, 9, 211u, 45u);
	BPX2(9, 10, 181u, 75u);
	BPX2(10, 11, 151u, 105u);
	BPX2(11, 12, 120u, 136u);
	BPX2(12, 13, 90u, 166u);
	BPX2(13, 14, 60u, 196u);
	BPX2(14, 15, 30u, 226u);
}

/* Border's source lines, kept across the rows of a band (consecutive rows share lines): slot k holds
 * source line tag[k] (frame line 40 + tag) as palette indices, 8 border pixels, the paper, 8 border
 * pixels. A row needs at most 3 lines, and the rows of a band need lines in increasing order. */
#define BORDER_SLOTS 4u
struct border_lines {
	uint8_t line[BORDER_SLOTS][BORDER_SRC_W];
	unsigned tag[BORDER_SLOTS];
};

/* Source line l (0-239) of a row whose lines start at lo: a slot's, or fetched into an empty slot
 * or one holding a line before lo. There is always one: the slots hold at most lines lo and lo + 1
 * from the band's earlier rows (a row's first line is past the row before's), and this row's lines
 * lo .. l - 1, so at most two of the four hold lines from lo on. */
static const uint8_t *border_line(const zx_machine *m, struct border_lines *lines, unsigned l, unsigned lo)
{
	unsigned k, slot = 0;
	uint8_t b, *d;
	for (k = 0; k < BORDER_SLOTS; k++) {
		if (lines->tag[k] == l)
			return lines->line[k];
		if (lines->tag[k] < lo || lines->tag[k] == ~0u)
			slot = k;
	}
	lines->tag[slot] = l;
	d = lines->line[slot];
	b = zx_border_line(m, ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER + l);
	if (l >= LCD_TOP_BORDER && l < LCD_TOP_BORDER + ZX_SCREEN_LINES) {
		memset(d, b, 8);
		zx_screen_line(m, l - LCD_TOP_BORDER, d + 8);
		memset(d + 8 + ZX_SCREEN_WIDTH, b, 8);
	} else {
		memset(d, b, BORDER_SRC_W);
	}
	return d;
}

/* The 17 palette indices at s are all one. */
static int flat17(const uint8_t *s)
{
	unsigned c;
	for (c = 1; c < 17u; c++)
		if (s[c] != s[0])
			return 0;
	return 1;
}

/* Border, picture row o (0-211): its 2 or 3 source lines blended down with the row's line weights
 * (the palette premultiplied by each), then across, two periods (34 columns, 30 pixels) at a time. */
static void compose_border(const zx_machine *m, struct border_lines *lines, unsigned o, int fmt, uint8_t *out)
{
	uint32_t trg[3][16], tb[3][16];
	const uint8_t *wv, *s0, *s1, *s2;
	unsigned l0 = 0, n, k, c, p, i;

	while (o >= 53u) { /* 53 rows a period of 60 lines */
		o -= 53u;
		l0 += 60u;
	}
	wv = border_v[o];
	l0 += wv[0];
	n = wv[3] ? 3u : 2u;
	s0 = border_line(m, lines, l0, l0);
	s1 = border_line(m, lines, l0 + 1u, l0);
	s2 = n == 3u ? border_line(m, lines, l0 + 2u, l0) : s1;
	for (k = 0; k < n; k++)
		for (i = 0; i < 16u; i++) {
			trg[k][i] = wv[1 + k] * pal_rg[i];
			tb[k][i] = wv[1 + k] * pal_b[i];
		}
	for (p = 0; p < BORDER_SRC_W; p += 34u) {
		uint32_t px[GROUP];
		unsigned h;
		for (h = 0; h < 2u; h++, s0 += 17, s1 += 17, s2 += 17) { /* a period: 17 columns, 15 pixels */
			const uint32_t *rg0 = trg[0], *rg1 = trg[1], *rg2 = trg[2], *b0 = tb[0], *b1 = tb[1], *b2 = tb[2];
			uint32_t vr[17], vg[17], vb[17];
			if (flat17(s0) && flat17(s1) && (n == 2u || flat17(s2))) {
				/* every column alike: each pixel is the column, as the weights sum to 256 */
				unsigned i0 = s0[0] & 15u, i1 = s1[0] & 15u, i2 = s2[0] & 15u;
				uint32_t rg = rg0[i0] + rg1[i1] + (n == 3u ? rg2[i2] : 0u);
				uint32_t bv = b0[i0] + b1[i1] + (n == 3u ? b2[i2] : 0u);
				uint32_t v = ((((rg >> 16) + 128u) >> 8) << 16) | (((bv + 128u) >> 8) << 8) |
					(((rg & 0xFFFFu) + 128u) >> 8);
				for (c = 0; c < 15u; c++)
					px[15u * h + c] = v;
				continue;
			}
			if (n == 2u) {
				for (c = 0; c < 17u; c++) {
					unsigned i0 = s0[c] & 15u, i1 = s1[c] & 15u;
					uint32_t rg = rg0[i0] + rg1[i1];
					vr[c] = rg >> 16;
					vg[c] = rg & 0xFFFFu;
					vb[c] = b0[i0] + b1[i1];
				}
			} else {
				for (c = 0; c < 17u; c++) {
					unsigned i0 = s0[c] & 15u, i1 = s1[c] & 15u, i2 = s2[c] & 15u;
					uint32_t rg = rg0[i0] + rg1[i1] + rg2[i2];
					vr[c] = rg >> 16;
					vg[c] = rg & 0xFFFFu;
					vb[c] = b0[i0] + b1[i1] + b2[i2];
				}
			}
			border_period(px + 15u * h, vr, vg, vb);
		}
		out = pack_group(fmt, px, out);
	}
}

/* One row in any mode; lines: Border's source lines so far in this band. */
static void compose_row(const zx_machine *m, struct border_lines *lines, unsigned row, unsigned mode, unsigned crop_x,
	int fmt, uint8_t *out)
{
	uint8_t idx[LCD_W];
	unsigned paper = row >= LCD_TOP_BORDER && row < LCD_TOP_BORDER + ZX_SCREEN_LINES;

	if (mode == PIX_BORDER) {
		if (row >= BORDER_TOP && row < BORDER_TOP + BORDER_ROWS) {
			compose_border(m, lines, row - BORDER_TOP, fmt, out);
			return;
		}
		/* above and below the picture: the border of frame line 40, or of 279 */
		memset(idx, zx_border_line(m, row < BORDER_TOP ? ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER
			: ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER + BORDER_SRC_LINES - 1u), LCD_W);
	} else if (mode == PIX_FIT_SMOOTH && paper) {
		compose_fit_smooth(m, row - LCD_TOP_BORDER, fmt, out);
		return;
	} else if (mode == PIX_FIT && paper) {
		compose_fit(m, row - LCD_TOP_BORDER, idx);
	} else { /* Crop, the border rows of Fit and Fit smooth, and any other mode as Crop */
		pix_compose_zx(m, row, crop_x, idx);
	}
	pix_pack(fmt, idx, out);
}

void pix_compose_rows(const zx_machine *m, unsigned row0, unsigned rows, unsigned mode, unsigned crop_x, int fmt,
	uint8_t *out)
{
	struct border_lines lines;
	unsigned i, rb = pix_row_bytes(fmt);
	for (i = 0; i < BORDER_SLOTS; i++)
		lines.tag[i] = ~0u;
	for (i = 0; i < rows; i++)
		compose_row(m, &lines, row0 + i, mode, crop_x, fmt, out + i * rb);
}

void pix_compose_row(const zx_machine *m, unsigned row, unsigned mode, unsigned crop_x, int fmt, uint8_t *out)
{
	pix_compose_rows(m, row, 1, mode, crop_x, fmt, out);
}

static void mark_row(uint8_t dirty[LCD_ROWS / 8u], unsigned r)
{
	dirty[r >> 3] = (uint8_t)(dirty[r >> 3] | (1u << (r & 7u)));
}

/* Border: the picture rows source line l (0-239, frame line 40 + l) feeds. */
static void mark_border_src(unsigned l, uint8_t dirty[LCD_ROWS / 8u])
{
	unsigned r = BORDER_TOP, v;
	while (l >= 60u) {
		l -= 60u;
		r += 53u;
	}
	v = border_line_rows[l];
	r += v & 0x7Fu;
	mark_row(dirty, r);
	if (v & 0x80u)
		mark_row(dirty, r + 1u);
}

void pix_mark_paper_line(unsigned mode, unsigned y, uint8_t dirty[LCD_ROWS / 8u])
{
	if (y >= ZX_SCREEN_LINES)
		return;
	if (mode == PIX_BORDER)
		mark_border_src(LCD_TOP_BORDER + y, dirty);
	else
		mark_row(dirty, LCD_TOP_BORDER + y);
}

void pix_mark_border_line(unsigned mode, unsigned line, uint8_t dirty[LCD_ROWS / 8u])
{
	unsigned l = line - (ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER); /* wraps below line 40 */
	if (l >= BORDER_SRC_LINES)
		return;
	if (mode != PIX_BORDER) {
		if (l < LCD_TOP_BORDER || l >= LCD_TOP_BORDER + ZX_SCREEN_LINES)
			mark_row(dirty, l);
		return;
	}
	mark_border_src(l, dirty);
	if (l == 0) {
		for (l = 0; l < BORDER_TOP; l++)
			mark_row(dirty, l);
	} else if (l == BORDER_SRC_LINES - 1u) {
		for (l = BORDER_TOP + BORDER_ROWS; l < LCD_ROWS; l++)
			mark_row(dirty, l);
	}
}
