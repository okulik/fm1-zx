/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The sound library's start-up functions: the mixer's, the decoder's (method A), method B's, the
 * FM synth's and the effects follower's. They run once per game start, so on the device they run
 * from flash (XIP): the firmware builds this file as its own library (zx_sound_init), apart from
 * the rest of core/sound in SRAM (RAMTEXT). Code in flash cannot call RAMTEXT directly, so nothing here
 * calls the other sound files: the helpers it shares with them are static inline, in
 * sound_internal.h. */

#include "sound_internal.h"

#define CPU_Q8 896000000u   /* 3500000 * 256: frequency = CPU_Q8 / period_q8 */
#define DRUM_TONE_PERIOD_Q8 4715789u   /* CPU_Q8 / 190: the FM drum's tone (fm.c) */

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
	slots_clear(&d->sl);
}

void snd_fm_init(snd_fm *f, uint32_t rate_num, uint32_t rate_den)
{
	uint32_t r;
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
	f->drum_inc = (uint32_t)udiv64(f->kfreq, DRUM_TONE_PERIOD_Q8, 0);   /* the drum's 190 Hz tone */
	fm_rest(f);
}

void snd_fx_init(snd_fx *f, uint32_t rate_num, uint32_t rate_den)
{
	uint64_t q;
	uint32_t i;

	if (!rate_num) rate_num = 1;
	if (!rate_den) rate_den = 1;
	f->read = snd_fx_read;
	f->rate_num = rate_num;
	f->rate_den = rate_den;
	q = udiv64((uint64_t)3500000u * rate_den << 20, rate_num, 0);
	f->kstep = q >> 32 ? 0xffffffffu : (uint32_t)q;
	/* 2 pi x 0.35 x 3.5 MHz / rate, Q16 (144121 = 2 pi x 0.35 x 2^16): x for a mean gap of 1 T */
	q = udiv64((uint64_t)144121u * 3500000u * rate_den, rate_num, 0);
	f->kx = q >> 32 ? 0xffffffffu : (uint32_t)q;
	f->x_lo = (uint32_t)udiv64((uint64_t)411775u * 150u * rate_den, rate_num, 0);    /* 2 pi x 2^16 */
	f->x_hi = (uint32_t)udiv64((uint64_t)411775u * 8000u * rate_den, rate_num, 0);
	f->tick_inc = (uint32_t)udiv64((uint64_t)1500u * rate_den << 32, rate_num, 0);
	f->last = f->stop = f->step = f->irr = f->rej = 0;
	for (i = 0; i < SND_FX_HALVES; i++)
		f->half[i] = 0;
	f->k = 0;
	f->n_half = f->have_last = 0;
	f->sound = SND_FX_SILENT;
	f->gate = SND_FX_OFF;
	f->n_ev = 0;
	f->tick_acc = f->phase = f->cur = f->target = f->tick_ph = f->hold_ph = f->nstep = 0;
	f->seed = 0x12345678u;
	f->hold = f->lp = 0;
	f->env_tone = f->env_noise = f->env_tick = 0;
	f->amp_tone = f->amp_noise = f->amp_tick = 0;
	f->lp_k = 0;
	f->on = SND_FX_SILENT;
	f->idle = 0;
	for (i = 0; i < SND_MAX_FRAME_SAMPLES; i++)
		f->buf[i] = 0;
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
	roles_rest(&m->roles);
	snd_fx_init(&m->fx, rate_num, rate_den);
	for (i = 0; i < SND_RING_SAMPLES; i++)
		m->ring[i] = 0;
	m->rd = 0;
	m->wr = 0;
	m->fill = 0;
	m->mute = 0;
	m->live_g = ONE;
	m->del_g = 0;
	m->fm_g = 0;
	m->fx_g = 0;
	m->effects = 1;
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
