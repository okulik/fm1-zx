/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The sound library's start-up functions: the mixer's, the decoder's (method A), method B's and
 * the FM synth's. They run once per game start, so on the device they run from flash (XIP): the
 * firmware builds this file as its own library (zx_sound_init), apart from the rest of
 * core/sound in SRAM (RAMTEXT). Code in flash cannot call RAMTEXT directly, so nothing here
 * calls the other sound files: the few helpers the start-up needs are copies (static) of
 * theirs, and the FM synth's turn-on reset inside a frame is fm.c's own snd_fm_reset.
 * The copies and the constants below must stay in step with the files they come from. */

#include "sound.h"

#define CPU_Q8 896000000u   /* 3500000 * 256: frequency = CPU_Q8 / period_q8 */
#define FM_OFF 0            /* fm.c: a voice's or the drum's stage OFF */
#define REST_FRAMES 50u     /* decoder.c */
#define NOWHERE 0xFFFFFFFFu /* engines.c: no carried event; no site */
#define ONE 32768           /* mixer.c: a Q15 gain of 1 */

/* fm.c's udiv64: n / d by restoring division, no 64-bit divide helper on the device. */
static uint64_t udiv64(uint64_t n, uint32_t d, uint32_t *rem)
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

/* slots.c's reset_channels and snd_slots_reset. */
static void reset_channels(snd_slots *s)
{
	uint32_t c;
	s->cur = 0;
	for (c = 0; c < SND_SLOTS; c++) {
		s->cand[c] = 0;
		s->run[c] = 0;
		s->nrise[c] = 0;
	}
}

static void slots_reset(snd_slots *s)
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

/* engines.c's silence. */
static void silence(snd_b *b)
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

void snd_decoder_init(snd_decoder *d)
{
	uint32_t i, k;

	d->frames = 0;
	for (i = 0; i < SND_HIST; i++) {
		for (k = 0; k < SND_TONAL; k++) {
			d->slot[i].hp_q8[k] = 0;
			d->slot[i].pair_q8[k] = 0;
		}
		d->slot[i].n = 0;
		d->slot[i].expl = 0;
		d->slot[i].busy = 0;
		d->slot[i].clean = 0;
		d->slot[i].change = 0;
		d->slot[i].full = 0;
		d->slot[i].steady = 0;
		d->slot[i].glide = 0;
		d->slot[i].alt = 0;
		d->slot[i].ear0 = 0;
		d->slot[i].pulse = 0;
		d->slot[i].width = 0;
	}
	d->timing = 0;
	d->frame_notes = 0;
	d->have_clean = 0;
	d->have_change = 0;
	d->broken = 0;
	d->work = 0;
	d->work_cap = 0xffffffffu;
	d->clean_frame = 0;
	d->change_frame = 0;
	for (k = 0; k < SND_TONAL; k++) {
		d->clean_q8[k] = 0;
		d->out_q8[k] = 0;
	}
	d->since_tonal = REST_FRAMES;
	d->prev_conf = 0;
	for (k = 0; k < SND_TONAL; k++)
		d->prev_q8[k] = 0;
	d->ref_change_frame = 0;
	d->ref_start = 0;
	d->prev_start = 0;
	d->have_prev = 0;
	d->ref_change = 0;
	d->returns = 0;
	d->alt = 0;
	d->pulse_style = 0;
	d->pulse_width = 0;
	d->busy_run = 0;
	d->clean_pulse = 0;
	d->ctx_pulse = 0;
	slots_reset(&d->sl);
}

void snd_fm_init(snd_fm *f, uint32_t rate_num, uint32_t rate_den)
{
	uint32_t i, r;
	uint64_t q;

	if (!rate_num) rate_num = 1;
	if (!rate_den) rate_den = 1;
	f->rate_num = rate_num;
	f->rate_den = rate_den;
	/* kfreq = CPU_Q8 * 2^32 * den / num: the phase step is kfreq / period_q8 */
	q = udiv64((uint64_t)CPU_Q8 * rate_den, rate_num, &r);
	if (q >> 32)
		f->kfreq = ~(uint64_t)0;
	else
		f->kfreq = (q << 32) | udiv64((uint64_t)r << 32, rate_num, 0);
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
		v->stage = FM_OFF;
	}
	f->drum_env = 0;
	f->drum_rel = 0;
	f->drum_lvl = 0;
	f->drum_amp = 0;
	f->drum_lp = 0;
	f->seed = 0x12345678u;
	f->drum_stage = FM_OFF;
	f->drum_held = 0;
}

void snd_b_init(snd_b *b, const snd_engine *table, uint32_t n)
{
	uint32_t k;

	b->table = table;
	b->n = n;
	b->frames = 0;
	for (k = 0; k < SND_B_MATCHES; k++) {
		b->match[k].anchor = 0;
		b->match[k].entry = 0;
		b->match[k].ok = 0;
		b->match[k].frame = 0;
		b->match[k].used = 0;
	}
	for (k = 0; k < SND_B_UNKNOWN; k++) {
		b->unknown[k].pc = 0;
		b->unknown[k].valid = 0;
		b->unknown[k].sum = 0;
		b->unknown[k].used = 0;
	}
	/* the window window_sum hashes: every byte a digest with a site at the address reads (a
	 * relocated word's high byte may lie one past the signature) */
	b->back = b->ahead = 0;
	for (k = 0; k < n; k++) {
		uint32_t s;
		for (s = 0; s < table[k].n_sites; s++) {
			const uint32_t off = table[k].sites[s].off, end = table[k].len + 1u;
			if (off > b->back) b->back = (uint16_t)off;
			if (end > off && end - off > b->ahead) b->ahead = (uint16_t)(end - off);
		}
	}
	for (k = 0; k < SND_HIST; k++) {
		uint32_t v;
		b->rec[k].kind = SND_B_NONE;
		b->rec[k].entry = 0;
		b->rec[k].ok = 0;
		b->rec[k].read = 0;
		b->rec[k].carry = 0;
		b->rec[k].next = 0;
		b->rec[k].step = 0;
		b->rec[k].whole = b->rec[k].cont = 0;
		b->rec[k].t_first = b->rec[k].d_first = NOWHERE;
		b->rec[k].t_last = 0;
		for (v = 0; v < SND_TONAL; v++) {
			b->rec[k].v[v].period_q8 = 0;
			b->rec[k].v[v].level = 0;
		}
	}
	silence(b);
	for (k = 0; k < SND_TONAL; k++) {
		b->lastv[k].period_q8 = 0;
		b->lastv[k].level = 0;
	}
	b->last_entry = 0;
	b->engine = 0;
	b->held = 0;
}

void snd_mixer_init(snd_mixer *m, uint32_t rate_num, uint32_t rate_den)
{
	uint32_t i, xf;

	if (!rate_num) rate_num = 1;
	if (!rate_den) rate_den = 1;
	snd_decoder_init(&m->dec);
	snd_b_init(&m->b, snd_engine_table, snd_engine_count);
	snd_fm_init(&m->fm, rate_num, rate_den);
	for (i = 0; i < SND_RING_SAMPLES; i++)
		m->ring[i] = 0;
	m->rd = 0;
	m->wr = 0;
	m->fill = 0;
	m->mute = 0;
	m->live_g = ONE;
	m->del_g = 0;
	m->fm_g = 0;
	xf = rate_num / rate_den / SND_XFADE_DIV;
	if (!xf) xf = 1;
	m->step = (int32_t)((ONE + xf - 1u) / xf);
	m->primed = 0;
	m->mode = SND_MODE_BEEPER;
	m->fm_choice = SND_MODE_FM_A;
	m->started = 0;
	m->fm_on = 0;
	m->run = 0;
}

void snd_mixer_set_engines(snd_mixer *m, const snd_engine *table, uint32_t n)
{
	snd_b_init(&m->b, table, n);
}
