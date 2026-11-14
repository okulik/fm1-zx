/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B's table for host tests: the built-in engines plus the test fixtures' engines.
 * Compiled by the host build (zx-sys build.rs) only, never into the firmware. */

#include "engines_builtin.h"

/* "Test burst": burst_sna's engine (host/crates/fmzx-pack/src/testprogs.rs), anchored at its
 * first byte: one square channel from the OUT at +1Ah, its half-period count n the operand at
 * +13h (self-modified), so f = 3.5 MHz / (2 (13n + 54)); the absolute operands at +02h (the
 * melody) and +09h (the operand it rewrites) move with the code. */
static void read_burst(const zx_machine *m, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	uint32_t n = zx_peek(m, (uint16_t)(a + 0x13));

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

const snd_engine snd_engine_test_table[] = {
	SND_ENGINES_BUILTIN,
	{ "Test burst", 0xd4b551f46da9de4cull, 0x2C, 1, 2, 1, 1, 0, burst_wild, burst_reloc, burst_sites, burst_pos, read_burst, 0, 0, 0 },
};
const uint32_t snd_engine_test_count = sizeof snd_engine_test_table / sizeof snd_engine_test_table[0];

/* ROM BEEP misread: the period a semitone too long (x 18/17, 5.9%), for the check to reject. */
static void read_beep_misread(const zx_machine *m, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	read_beep(m, a, site, r, out);
	out->v[0].period_q8 += out->v[0].period_q8 / 17u;
}

const snd_engine snd_engine_misread_table[] = { SND_ENGINE_ROM_BEEP_READ(read_beep_misread) };
const uint32_t snd_engine_misread_count = 1;
