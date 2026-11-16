/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The beeper renderer's start-up function (beeper.c renders). It runs once per game start, so on
 * the device it runs from flash (XIP): the firmware builds this file as its own library
 * (zx_core_init), apart from the rest of the core in SRAM (RAMTEXT). It calls nothing else.
 *
 * Samples per frame = rate_num * 69888 / (rate_den * 3500000). 69888 and 3500000 share the
 * factor 224, leaving rate_num * 312 / (rate_den * 15625); gcds are taken between the factors
 * with 32-bit modulo so the product never needs reducing in 64 bits. */

#include "zx.h"

static uint32_t gcd32(uint32_t a, uint32_t b)
{
	while (b) {
		uint32_t t = a % b;
		a = b;
		b = t;
	}
	return a;
}

void zx_beeper_init(zx_beeper *b, uint32_t rate_num, uint32_t rate_den)
{
	uint32_t a = rate_num, c = rate_den, n2 = 312u, d2 = 15625u, g;

	if (!a) a = 1;
	if (!c) c = 1;
	g = gcd32(a, c);
	a /= g;
	c /= g;
	g = gcd32(a, d2);   /* numerator vs 15625 */
	a /= g;
	d2 /= g;
	g = gcd32(n2, c);   /* 312 vs denominator */
	n2 /= g;
	c /= g;
	b->num = a * n2;
	b->den = c * d2;
	b->rem = 0;
	b->sum = 0;
	b->fill = 0;
	b->prev_x = 0;
	b->prev_y = 0;
	b->level = 0;
	b->dc_on = 1;
}
