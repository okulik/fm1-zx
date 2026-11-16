/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* What the sound library's files share that sound.h does not export: constants and small helpers.
 * On the device sound_init.c (the start-up functions) runs from flash and the other files from
 * SRAM, out of each other's branch reach, so the helpers are static inline: each file compiles
 * its own copy into its own code and no call crosses between the two, while the source is here
 * once. */
#ifndef SND_SOUND_INTERNAL_H
#define SND_SOUND_INTERNAL_H

#include "sound.h"

#define ONE 32768             /* a Q15 gain of 1 (mixer.c) */
#define NOWHERE 0xFFFFFFFFu   /* method B: no carried event; no site (engines.c) */
#define REST_FRAMES 50u       /* a rest keeps FM this long after the last confident voices (decoder.c) */

/* An FM voice's stage, and the drum's (fm.c). */
enum { OFF, ATTACK, DECAY, RELEASE };

/* n / d by restoring division: no 64-bit divide helper on the device. */
static inline uint64_t udiv64(uint64_t n, uint32_t d, uint32_t *rem)
{
	uint64_t q = 0, r = 0;
	int i;

	for (i = 63; i >= 0; i--) {
		r = (r << 1) | ((n >> i) & 1u);
		if (r >= d) {
			r -= d;
			q |= (uint64_t)1 << i;
		}
	}
	if (rem) *rem = (uint32_t)r;
	return q;
}

/* The FM synth's voices and drum to rest; the rate and kfreq stay (snd_fm_init, snd_fm_reset). */
static inline void fm_rest(snd_fm *f)
{
	uint32_t i;

	f->tick_acc = 0;
	for (i = 0; i < SND_TONAL; i++) {
		snd_fm_voice *v = &f->v[i];
		v->phase = 0;
		v->inc = 0;
		v->period_q8 = 0;
		v->env = 0;
		v->rel_step = 0;
		v->depth_base = 0;
		v->depth_ex = 0;
		v->lvl = 0;
		v->lvl_target = 0;
		v->amp = 0;
		v->stage = OFF;
	}
	f->drum_env = 0;
	f->drum_rel = 0;
	f->drum_lvl = 0;
	f->drum_amp = 0;
	f->drum_lp = 0;
	f->seed = 0x12345678u;
	f->drum_stage = OFF;
	f->drum_held = 0;
}

/* Method B's voices off and nothing carried (engines.c, snd_b_init). */
static inline void silence(snd_b *b)
{
	uint32_t k;

	for (k = 0; k < SND_TONAL; k++) {
		b->tone[k].period_q8 = 0;
		b->tone[k].level = 0;
	}
	b->drum = 0;
	for (k = 0; k <= SND_TONAL; k++)
		b->carry[k] = NOWHERE;
}

/* The time-slot channels afresh (slots.c). */
static inline void reset_channels(snd_slots *s)
{
	uint32_t c;
	s->cur = 0;
	for (c = 0; c < SND_SLOTS; c++) {
		s->cand[c] = 0;
		s->run[c] = 0;
		s->nrise[c] = 0;
	}
}

/* The time-slot decoder afresh: what snd_slots_reset does (slots.c, snd_decoder_init). */
static inline void slots_clear(snd_slots *s)
{
	uint32_t i;

	s->loop_q8 = 0;
	s->loop_frame = 0;
	s->other_q8 = 0;
	s->bare = 0;
	s->grid_q8 = 0;
	s->last_t = 0;
	s->cum = 0;
	s->h[0] = s->h[1] = 0;
	s->recip = 0;
	s->running = 0;
	s->level = 0;
	s->primed = 0;
	s->lv[0] = s->lv[1] = 0xffffu;
	for (i = 0; i < SND_LEVEL_BINS; i++)
		s->hist[i] = 0;
	s->hist_total = 0;
	s->learnt_frame = 0;
	s->k = 0;
	for (i = 0; i < SND_SLOTS; i++)
		s->w[i] = 0;
	for (i = 0; i < 257u; i++)
		s->lut[i] = 0;
	reset_channels(s);
	s->good = s->samples = 0;
	for (i = 0; i < SND_TONAL; i++)
		s->sref_q8[i] = s->pend_q8[i] = 0;
	s->sref_frame = s->schange_frame = s->pend_frame = 0;
	s->pend_n = 0;
	s->have_sref = s->have_schange = 0;
	s->sbroken = 0;
}

#endif
