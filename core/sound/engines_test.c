/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B's table for host tests: the built-in engines plus the test fixtures' engines.
 * Compiled by the host build (zx-sys build.rs) only, never into the firmware. */

#include "engines_builtin.h"

/* "Test burst": burst_sna's engine (host/crates/fmzx-pack/src/testprogs.rs), anchored at its
 * first byte: one square channel from the OUT at +1Ah, its half-period count n the operand at
 * +13h (self-modified), so f = 3.5 MHz / (2 (13n + 54)); the absolute operands at +02h (the
 * melody) and +09h (the operand it rewrites) move with the code. */
static void read_burst(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	uint32_t n = zx_peek(m, (uint16_t)(a + 0x13));

	(void)en;
	(void)site;
	(void)r;
	if (n) {
		out->v[0].period_q8 = 2u * (13u * n + 54u) * 256u;
		out->v[0].level = 200;
	}
}

static const uint16_t burst_wild[] = { 0x13 };
static const uint16_t burst_reloc[] = { 0x02, 0x09 };
static const snd_engine_site burst_sites[] = { { 0x1A, SND_SITE_TONAL, 0 } };
static const uint16_t burst_pos[] = { 0 };

/* "Test flips": flips_sna's engine (testprogs.rs), anchored at its first byte: two channels
 * flipping the one OUT at +32h (SND_CHAN_ANY). IX points at the note [n0, n1, length]; a
 * channel's half-period is n passes (0: 256) of 96 T, 11 more every 256 passes, so period =
 * 2 x 256 x (96 + 11/256) n. B (from 0) and C (from the length) count the note's passes; the
 * absolute operand at +04h (the melody) moves with the code. */
static void read_flips(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	const uint32_t k = 256u * ((zx_peek(m, (uint16_t)(r->ix + 2u)) - r->bc) & 0xFFu) + ((0u - (r->bc >> 8)) & 0xFFu);   /* passes */
	uint32_t c;

	(void)en;
	(void)a;
	(void)site;
	for (c = 0; c < 2; c++) {
		uint32_t n = zx_peek(m, (uint16_t)(r->ix + c));
		if (!n) n = 256u;
		out->v[c].period_q8 = 49174u * n;   /* 2 x 256 x (96 + 11/256) */
		out->v[c].level = 200;
	}
	out->since = out->row = 96u * k + 11u * (k / 256u) + 82u;   /* (the edge 82 T into its pass) */
}

static const uint16_t flips_reloc[] = { 0x04 };
static const snd_engine_site flips_sites[] = { { 0x32, SND_SITE_TONAL, SND_CHAN_ANY } };

#define FLIPS_ENTRY(rd) { "Test flips", 0x351e1ec1682afe7eull, 0x41, 0, 1, 1, 2, 0, 0, flips_reloc, flips_sites, beep_pos, rd, 0, 1, 0, 0, 0, { 0 } }

const snd_engine snd_engine_test_table[] = {
	SND_ENGINES_BUILTIN,
	{ "Test burst", 0xd4b551f46da9de4cull, 0x2C, 1, 2, 1, 1, 0, burst_wild, burst_reloc, burst_sites, burst_pos, read_burst, 0, 0, 0, 0, 0, { 0 } },
	FLIPS_ENTRY(read_flips),
};
const uint32_t snd_engine_test_count = sizeof snd_engine_test_table / sizeof snd_engine_test_table[0];

/* The misread table: entries whose readers are off, for the check to reject. ROM BEEP's period a
 * semitone too long (x 18/17, 5.9%). */
static void read_beep_misread(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	read_beep(m, en, a, site, r, out);
	out->v[0].period_q8 += out->v[0].period_q8 / 17u;
}

/* Test flips' channel 1 period half the true one (an octave up: in the octave, the low channel
 * read at the high one's pitch). */
static void read_flips_misread(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	read_flips(m, en, a, site, r, out);
	out->v[1].period_q8 /= 2u;
}

const snd_engine snd_engine_misread_table[] = { SND_ENGINE_ROM_BEEP_READ(read_beep_misread), FLIPS_ENTRY(read_flips_misread) };
const uint32_t snd_engine_misread_count = sizeof snd_engine_misread_table / sizeof snd_engine_misread_table[0];
