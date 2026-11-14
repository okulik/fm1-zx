/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B: known music engines, recognised by the code around the OUT that drives the speaker,
 * their notes read from memory and registers and checked against the beeper's edges.
 *
 * For now a stub: the table holds no engine (an unused entry, as C has no empty arrays) and no
 * frame is B's, so FM: engines plays the delayed beeper throughout. */

#include "sound.h"

const snd_engine snd_engine_table[] = {
	{ "(none)" },   /* unused: snd_engine_count is 0 */
};
const uint32_t snd_engine_count = 0;

const char *snd_engine_name(const snd_engine *table, uint32_t index)
{
	return table[index].name;
}

void snd_b_init(snd_b *b, const snd_engine *table, uint32_t n)
{
	b->table = table;
	b->n = n;
	b->frames = 0;
}

void snd_b_frame(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out)
{
	uint32_t k;

	(void)zm;
	(void)e;
	(void)n;
	(void)start_level;
	for (k = 0; k < SND_TONAL; k++) {
		out->tone[k].period_q8 = 0;
		out->tone[k].level = 0;
	}
	out->drum = 0;
	out->style = SND_STYLE_NONE;
	out->confident = 0;
	out->engine = 0;
	out->n_ev = 0;
	b->frames++;
}
