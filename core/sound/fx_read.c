/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The effects follower's reader (the rules: sound.h, "effects follower"). Once a frame, after the
 * decoder and method B have taken frame t, it reads the decoder's stored speaker transitions:
 *
 * - frame t on its own, for its irregular flag: a history of the frame's own, and at each
 *   transition from its 4th on (a window) the tone/noise rule; noise in over half of the windows
 *   is irregular. The flags of the frames after the one played are then known, so the first frame
 *   of a sustained irregular run already stays on the beeper.
 * - frame t - 3, the one the mixer plays now, through the follower's history carried across
 *   frames: each transition updates the sound (a tick after a new-sound gap; the tone's step or
 *   the noise's rate and low-pass); a stop falls where the sound's time runs out with no
 *   transition, possibly in a later frame. Each change becomes an event at its sample,
 *   T x n / ZX_FRAME_TSTATES, after one at sample 0 with the state at the frame's start.
 *   Pitch updates closer than n / SND_FX_EVENTS samples to the one before replace it (keeping its
 *   sample), and on overflow the last event takes the latest state, so a fast sweep updates
 *   evenly through the frame.
 *
 * The phase step of a period P (T-states) is 3.5 MHz / P / rate x 2^32 = kstep x 2^12 / P, in
 * two 32-bit divisions. The noise's low-pass coefficient 1 - exp(-x), x = 2 pi fc / rate (Q16),
 * is x / (1 + x / 2 + x^2 / 12) (within 0.2% up to 8 kHz).
 *
 * On the device this file runs from flash (its own library, zx_sound_fx) and the mixer calls it
 * through the follower's read pointer: it calls nothing in the other sound files. */

#include "sound_internal.h"

#define GAP 30000u          /* T-states: a longer gap starts a new sound */
#define NARROW 200u         /* a gap under this is a narrow pulse: multi-voice beeper music */
#define MIN_PERIOD 292u     /* 3.5 MHz / 12 kHz: the highest tone (the 30 Hz floor is never reached:
                             * gaps are under GAP) */

/* The step of a period of p T-states (saturating). */
static uint32_t step_of(const snd_fx *f, uint32_t p)
{
	uint32_t q = f->kstep / p;

	if (q >> 20) return 0xffffffffu;
	return (q << 12) + (((f->kstep % p) << 12) / p);
}

/* Appends a half period to the history h (n held). */
static void push(uint16_t *h, uint8_t *n, uint32_t g)
{
	uint32_t i;

	if (*n == SND_FX_HALVES) {
		for (i = 1; i < SND_FX_HALVES; i++)
			h[i - 1] = h[i];
		(*n)--;
	}
	h[(*n)++] = (uint16_t)g;
}

/* The tone/noise rule over the history h[0..n), n >= 2: 1 = noise. *med: the median of the last
 * up to 6 full periods (adjacent pairs). */
static int noisy(const uint16_t *h, uint32_t n, uint32_t *med)
{
	uint32_t q[6], m = 0, i, j;

	for (i = n - 1u; i >= 1u && m < 6u; i--) {
		uint32_t v = (uint32_t)h[i - 1u] + h[i];
		for (j = m; j > 0 && q[j - 1u] > v; j--)
			q[j] = q[j - 1u];
		q[j] = v;
		m++;
	}
	*med = q[m / 2u];
	return n >= 4u && (q[m - 1u] - q[0]) * 10u >= *med * 3u;
}

/* Frame t on its own: over half of its windows read noise. */
static uint32_t irregular(const uint32_t *t, uint32_t n)
{
	uint16_t h[SND_FX_HALVES];
	uint8_t nh = 0;
	uint32_t i, med, windows = 0, noise = 0;

	for (i = 1; i < n; i++) {
		uint32_t g = t[i] - t[i - 1u];
		if (g >= GAP)
			nh = 0;
		else
			push(h, &nh, g);
		if (i >= 3u) {
			windows++;
			if (nh >= 2u && noisy(h, nh, &med)) noise++;
		}
	}
	return noise * 2u > windows;
}

/* The follower's state from frame T-state `ft` on (a tick there if `tick`), at its sample (an
 * edge of the instruction overshooting the frame's end at the last). */
static void emit(snd_fx *f, uint32_t ft, uint8_t tick, uint32_t n)
{
	snd_fx_event *e;
	uint32_t at = (ft < ZX_FRAME_TSTATES ? ft : ZX_FRAME_TSTATES - 1u) * n / ZX_FRAME_TSTATES;

	if (f->n_ev) {
		int same;
		e = &f->ev[f->n_ev - 1u];
		same = e->sound == f->sound;
		if (!tick && same && (f->sound == SND_FX_SILENT || (e->step == f->step && e->k == f->k))) return;
		if (e->at == at || f->n_ev == SND_FX_EVENTS || (!tick && same && at - e->at < n / SND_FX_EVENTS))
			goto set;
	}
	e = &f->ev[f->n_ev++];
	e->at = (uint16_t)at;
	e->tick = 0;
set:
	e->sound = f->sound;
	e->step = f->step;
	e->k = f->k;
	e->tick |= tick;
}

/* A transition at absolute T-state `at`: the history and the sound after it. */
static uint8_t edge(snd_fx *f, uint32_t at)
{
	uint8_t tick = 0;
	uint32_t i, med, maxh = 0, len;

	if (f->have_last && at - f->last < GAP) {
		push(f->half, &f->n_half, at - f->last);
	} else {
		f->n_half = 0;
		tick = 1;
	}
	f->last = at;
	f->have_last = 1;
	if (f->n_half < 2u) {
		f->sound = SND_FX_SILENT;
		return tick;
	}
	if (!noisy(f->half, f->n_half, &med)) {
		f->sound = SND_FX_TONE;
		f->step = step_of(f, med < MIN_PERIOD ? MIN_PERIOD : med);
	} else {
		uint32_t sum = 0, c = f->n_half < 8u ? f->n_half : 8u, x, d;
		for (i = f->n_half - c; i < f->n_half; i++)
			sum += f->half[i];
		sum /= c;   /* the mean gap */
		f->sound = SND_FX_NOISE;
		f->step = step_of(f, sum ? sum : 1u);
		x = f->kx / (sum ? sum : 1u);
		if (x < f->x_lo) x = f->x_lo;
		if (x > f->x_hi) x = f->x_hi;
		d = 65536u + x / 2u + (x >> 4) * (x >> 4) / 3072u;
		f->k = (uint16_t)(((x << 15) / (d >> 1)) >> 1);
	}
	for (i = f->n_half > 4u ? f->n_half - 4u : 0; i < f->n_half; i++)
		if (f->half[i] > maxh) maxh = f->half[i];
	len = maxh * 5u / 2u;
	f->stop = at + (len < GAP ? len : GAP);
	return tick;
}

/* method B's own frame u that its check refused */
static inline uint32_t rejected(const snd_b *b, uint32_t u)
{
	return b->rec[u % SND_HIST].kind == SND_B_MINE && b->rec[u % SND_HIST].ok == 0;
}

/* the rejected frame fr (bit 0 of `rej`; bits 1, 2: the 2 before it, as they were played) is in a run of 3
 * or more rejected frames: one of the 3 windows holding it, with the 2 after it (`a`, `b`) */
static inline uint32_t rejected_run(uint32_t rej, uint32_t a, uint32_t b)
{
	return ((rej & 6u) == 6u) || ((rej & 2u) && a) || (a && b);
}

void snd_fx_read(snd_fx *f, const snd_decoder *d, const snd_b *b, uint32_t n)
{
	const uint32_t t = d->frames - 1u;
	const snd_slot *s;
	const uint32_t *tt;
	uint32_t fr, base, i, gaps = 0, narrow = 0;

	if (f->idle) {   /* the mixer skipped the voice last frame: it starts afresh (here, not in SRAM) */
		f->idle = 0;
		f->env_tone = f->env_noise = f->env_tick = 0;
		f->amp_tone = f->amp_noise = f->amp_tick = 0;
	}
	f->irr = (f->irr << 1) | irregular(d->t[t % SND_HIST], d->slot[t % SND_HIST].n);
	f->n_ev = 0;
	if (t < SND_DELAY_FRAMES) {
		f->gate = SND_FX_OFF;
		return;
	}
	fr = t - SND_DELAY_FRAMES;
	s = &d->slot[fr % SND_HIST];
	f->rej = (f->rej << 1) | rejected(b, fr);
	tt = d->t[fr % SND_HIST];
	base = fr * ZX_FRAME_TSTATES;
	emit(f, 0, 0, n);
	for (i = 0; i < s->n; i++) {
		uint8_t tick;
		if (f->sound && f->stop - base < tt[i]) {
			f->sound = SND_FX_SILENT;
			emit(f, f->stop - base, 0, n);
		}
		if (f->have_last) {
			gaps++;
			if (base + tt[i] - f->last < NARROW) narrow++;
		}
		tick = edge(f, base + tt[i]);
		emit(f, tt[i], tick, n);
	}
	if (f->sound && f->stop - base < ZX_FRAME_TSTATES) {
		f->sound = SND_FX_SILENT;
		emit(f, f->stop - base, 0, n);
	}
	if (f->have_last && base + ZX_FRAME_TSTATES - f->last >= GAP) f->have_last = 0;
	if (s->busy) f->have_last = f->n_half = 0;   /* the decoder kept 256 edges: `last` is mid-frame, the next gap would be bogus */

	if (rejected(b, fr) && !rejected_run(f->rej, rejected(b, fr + 1u), rejected(b, fr + 2u)))
		f->gate = SND_FX_B_REJECTED;   /* method B's own frame that its check refused, in a run of under 3: music, not an effect */
	else if (s->busy)
		f->gate = SND_FX_BUSY;
	else if (narrow * 5u > gaps * 2u)
		f->gate = SND_FX_TWO_VOICE;
	else if ((f->irr & 0x38u) == 0x38u || (f->irr & 0x1Cu) == 0x1Cu || (f->irr & 0x0Eu) == 0x0Eu)
		f->gate = SND_FX_IRREGULAR;   /* a run of 3 within frames t - 5 .. t - 1 holding t - 3 */
	else
		f->gate = SND_FX_PLAY;
}
