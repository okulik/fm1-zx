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

/* ------------------------------------------------------------------------------------------------
 * Screen modes. Crop is pix_compose_zx + pix_pack. The other modes are separate static functions,
 * so Crop's path stays as it is wherever they are placed. */

#define BORDER_TOP 14u     /* Border: the picture at LCD rows 14-225 */
#define BORDER_ROWS 212u
#define BORDER_SRC_W 272u  /* 8 border pixels, the 256 of the paper, 8 border pixels */
#define BORDER_SRC_LINES 240u /* frame lines 40-279 */

/* 8-bit R, G, B of each palette index. */
#define PAL8_ROW(ch) \
	ch(0), ch(1), ch(2), ch(3), ch(4), ch(5), ch(6), ch(7), ch(8), ch(9), ch(10), ch(11), ch(12), ch(13), ch(14), ch(15)
static const uint8_t pal8[3][16] = {{PAL8_ROW(R8)}, {PAL8_ROW(G8)}, {PAL8_ROW(B8)}};

/* Border's area filter, separable, with weights in 256ths along each axis. Along an axis, output
 * pixels of span S units cover source pixels of U units, in periods of n outputs; output j of a
 * period starts at unit S*j and its first source pixel is first = floor(S*j/U). With C_k the
 * overlap of the output with source pixels first..first+k, in units, weight k is
 * round(256*C_k/S) - round(256*C_{k-1}/S) (round(v) = floor(v + 1/2), C_-1 = 0), so a pixel's
 * weights sum to 256 and a flat area stays exact. Each entry is {first, w0, w1, w2}.
 * Columns: U = 15, S = 17, n = 15 (17 source columns a period). */
static const uint8_t border_h[15][4] = {
	{0, 226, 30, 0}, {1, 196, 60, 0}, {2, 166, 90, 0}, {3, 136, 120, 0}, {4, 105, 151, 0},
	{5, 75, 181, 0}, {6, 45, 211, 0}, {7, 15, 226, 15}, {9, 211, 45, 0}, {10, 181, 75, 0},
	{11, 151, 105, 0}, {12, 120, 136, 0}, {13, 90, 166, 0}, {14, 60, 196, 0}, {15, 30, 226, 0},
};
/* Lines: U = 53, S = 60, n = 53 (60 source lines a period): 240 lines to 212 rows. */
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

/* Pixel x of a row from 8-bit r, g, b: RGB444 keeps the top 4 bits, RGB565 5/6/5, as pix_pack.
 * In RGB444 an even x writes its pair's first byte and a half; the odd x after it the rest. */
static void put_rgb(int fmt, uint8_t *out, unsigned x, unsigned r, unsigned g, unsigned b)
{
	if (fmt == PIX_RGB565) {
		unsigned c = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3);
		out[2u * x] = (uint8_t)(c >> 8);
		out[2u * x + 1u] = (uint8_t)c;
		return;
	}
	out += (x >> 1) * 3u;
	if (!(x & 1u)) {
		out[0] = (uint8_t)((r & 0xF0u) | (g >> 4));
		out[1] = (uint8_t)(b & 0xF0u);
	} else {
		out[1] = (uint8_t)(out[1] | (r >> 4));
		out[2] = (uint8_t)((g & 0xF0u) | (b >> 4));
	}
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
 * are columns 16k + s (weight 15 - s sixteenths) and 16k + s + 1 (weight 1 + s). */
static void compose_fit_smooth(const zx_machine *m, unsigned y, int fmt, uint8_t *out)
{
	uint8_t line[ZX_SCREEN_WIDTH];
	unsigned k, s, x = 0;
	zx_screen_line(m, y, line);
	for (k = 0; k < ZX_SCREEN_WIDTH; k += 16u)
		for (s = 0; s < 15u; s++, x++) {
			unsigned a = line[k + s] & 15u, b = line[k + s + 1u] & 15u, wa = 15u - s, wb = 1u + s;
			put_rgb(fmt, out, x, (wa * pal8[0][a] + wb * pal8[0][b] + 8u) >> 4,
				(wa * pal8[1][a] + wb * pal8[1][b] + 8u) >> 4, (wa * pal8[2][a] + wb * pal8[2][b] + 8u) >> 4);
		}
}

/* Border, picture row o (0-211). */
static void compose_border(const zx_machine *m, unsigned o, int fmt, uint8_t *out)
{
	uint8_t src[3][BORDER_SRC_W];
	const uint8_t *wv;
	unsigned l0 = 0, n, k, c, p, s, x = 0;

	while (o >= 53u) { /* 53 rows a period of 60 lines */
		o -= 53u;
		l0 += 60u;
	}
	wv = border_v[o];
	l0 += wv[0];
	n = wv[3] ? 3u : 2u;
	for (k = 0; k < n; k++) { /* source line l0 + k, frame line 40 + l0 + k, as palette indices */
		unsigned l = l0 + k;
		uint8_t b = zx_border_line(m, ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER + l);
		memset(src[k], b, BORDER_SRC_W);
		if (l >= LCD_TOP_BORDER && l < LCD_TOP_BORDER + ZX_SCREEN_LINES)
			zx_screen_line(m, l - LCD_TOP_BORDER, src[k] + 8);
	}
	for (p = 0; p < BORDER_SRC_W; p += 17u) { /* 17 source columns, 15 output pixels */
		uint16_t v[3][18]; /* the period's columns blended down the lines, in 256ths; [17] = 0 */
		for (c = 0; c < 17u; c++) {
			unsigned r = 0, g = 0, b = 0;
			for (k = 0; k < n; k++) {
				unsigned i = src[k][p + c] & 15u, w = wv[1 + k];
				r += w * pal8[0][i];
				g += w * pal8[1][i];
				b += w * pal8[2][i];
			}
			v[0][c] = (uint16_t)r;
			v[1][c] = (uint16_t)g;
			v[2][c] = (uint16_t)b;
		}
		v[0][17] = v[1][17] = v[2][17] = 0;
		for (s = 0; s < 15u; s++, x++) {
			const uint8_t *wh = border_h[s];
			const unsigned f = wh[0];
			unsigned ch[3];
			for (k = 0; k < 3u; k++)
				ch[k] = (wh[1] * v[k][f] + wh[2] * v[k][f + 1u] + wh[3] * v[k][f + 2u] + 32768u) >> 16;
			put_rgb(fmt, out, x, ch[0], ch[1], ch[2]);
		}
	}
}

void pix_compose_row(const zx_machine *m, unsigned row, unsigned mode, unsigned crop_x, int fmt, uint8_t *out)
{
	uint8_t idx[LCD_W];
	unsigned paper = row >= LCD_TOP_BORDER && row < LCD_TOP_BORDER + ZX_SCREEN_LINES;

	if (mode == PIX_BORDER) {
		if (row >= BORDER_TOP && row < BORDER_TOP + BORDER_ROWS) {
			compose_border(m, row - BORDER_TOP, fmt, out);
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
	} else {
		pix_compose_zx(m, row, mode == PIX_CROP ? crop_x : 0, idx);
	}
	pix_pack(fmt, idx, out);
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
