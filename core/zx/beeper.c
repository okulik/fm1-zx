/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Beeper renderer: edge list -> box-filtered, DC-blocked samples.
 *
 * Samples per frame = rate_num * 69888 / (rate_den * 3500000), as num / den
 * (zx_beeper_init, in beeper_init.c: it runs from flash on the device).
 *
 * Time is scaled by N (= num): a T-state t is t * N units, the frame is
 * 69888 * N units and one sample is S = 69888 * D units (D = den). Both
 * T-states and sample boundaries are then integers. */

#include "zx.h"

static const int16_t amplitude[4] = {0, 1000, 12000, 13000};

#define DC_POLE 32604   /* 0.995 in Q15 */

void zx_beeper_set_level(zx_beeper *b, uint8_t level)
{
	b->level = level & 3u;
}

uint32_t zx_beeper_frame_samples(const zx_beeper *b)
{
	return (b->rem + b->num) / b->den;
}

/* sum / span rounded to nearest, for a quotient below 2^14 (max amplitude
 * 13000): restoring division, so no 64-bit divide helper is needed. */
static int32_t average(uint64_t sum, uint64_t span)
{
	uint32_t q = 0;
	int bit;

	sum += span >> 1;
	for (bit = 13; bit >= 0; bit--) {
		uint64_t s = span << bit;
		if (sum >= s) {
			sum -= s;
			q |= 1u << bit;
		}
	}
	return (int32_t)q;
}

static int16_t filter(zx_beeper *b, int32_t x)
{
	int32_t y;

	if (!b->dc_on) return (int16_t)x;
	y = x - b->prev_x + (DC_POLE * b->prev_y) / 32768;
	if (y > 32767) y = 32767;
	if (y < -32768) y = -32768;
	b->prev_x = x;
	b->prev_y = y;
	return (int16_t)y;
}

uint32_t zx_beeper_render(zx_beeper *b, const zx_edge *edges, uint32_t count, int16_t *out)
{
	const uint64_t span = (uint64_t)ZX_FRAME_TSTATES * b->den;
	const uint64_t scale = b->num;
	uint64_t pos = 0, sum = b->sum, fill = b->fill;
	uint32_t n = 0, i;

	for (i = 0; i <= count; i++) {
		uint64_t end;
		int32_t amp = amplitude[b->level & 3];

		if (i < count) {
			uint32_t t = edges[i].tstate;
			if (t > ZX_FRAME_TSTATES) t = ZX_FRAME_TSTATES;
			end = (uint64_t)t * scale;
			if (end < pos) end = pos;
		} else {
			end = (uint64_t)ZX_FRAME_TSTATES * scale;
		}
		while (pos < end) {
			uint64_t take = end - pos;
			if (take > span - fill) take = span - fill;
			sum += (uint64_t)amp * take;
			fill += take;
			pos += take;
			if (fill == span) {
				out[n++] = filter(b, average(sum, span));
				sum = 0;
				fill = 0;
			}
		}
		if (i < count) b->level = edges[i].level & 3;
	}
	b->sum = sum;
	b->fill = fill;
	b->rem = (b->rem + b->num) % b->den;
	return n;
}

#ifdef ZX_TESTING
void zx_beeper_test_dc_block(zx_beeper *b, int on)
{
	b->dc_on = on != 0;
}
#endif
