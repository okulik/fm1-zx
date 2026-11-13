/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The beeper music decoder. For now a stub that never recognises music: every
 * decision is "no voices, style NONE, not confident", so an FM mode plays the
 * delayed beeper. The style decoders replace it. */

#include "sound.h"

void snd_decoder_init(snd_decoder *d)
{
	d->frames = 0;
}

void snd_decoder_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out)
{
	uint32_t i;

	(void)e;
	(void)n;
	(void)start_level;
	for (i = 0; i < SND_TONAL; i++) {
		out->tone[i].period_q8 = 0;
		out->tone[i].level = 0;
	}
	out->drum = 0;
	out->style = SND_STYLE_NONE;
	out->confident = 0;
	d->frames++;
}
