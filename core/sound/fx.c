/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The effects follower's voice: renders the reader's event list (fx_read.c) for the mixer.
 *
 *   tone   sin(p + 1.3 sin 2p) (the lead's ratio and index), gain 0.8; its step glides to the
 *          target exponentially with a 2 ms time constant (an exponential approach of the phase
 *          step, so linear in frequency, not log-frequency: at 2 ms the difference is
 *          inaudible), and jumps there when the tone starts
 *          from silence
 *   noise  xorshift32 sampled and held at the edge rate, one-pole low-passed (the reader's
 *          coefficient), x 1.6 (the low-pass's loss) at gain 0.55
 *   tick   a 1.5 kHz sine decaying with a 4 ms time constant, gain 0.4
 *
 * Tone and noise have their own envelope: linear attack 1 ms, release 10 ms. Gain 1 peaks at 8000.
 * As in fm.c: 32-bit phase (2^32 = one cycle), its sine table, and envelopes, glide and decay on
 * an 8 kHz tick (0.125 ms, whatever the sample rate), so their constants need no run-time maths.
 * Envelopes are Q16; out = x (Q15) * amp >> 15. Per sample it runs from SRAM on the device: kept
 * small, the rest is the reader's. */

#include "sound_internal.h"

#define TICK_HZ 8000u
#define ENV_ONE 65536
#define ATTACK_STEP 8192      /* ENV_ONE / 8 ticks: 1 ms */
#define RELEASE_STEP 820      /* ENV_ONE / 80 ticks: 10 ms, rounded up */
#define GLIDE_K 3971          /* 1 - exp(-0.125 / 2), Q16 */
#define TICK_DEC 63520u       /* exp(-0.125 / 4), Q16 */
#define DEPTH 13559           /* the index 1.3 in fm.c's depth units (10430.378 per radian) */
#define TONE_AMP 6400         /* 0.8 x 8000 */
#define NOISE_AMP 7040        /* 1.6 x 0.55 x 8000 */
#define TICK_AMP 3200         /* 0.4 x 8000 */

static int32_t env_to(int32_t env, int on)
{
	if (on) return env + ATTACK_STEP < ENV_ONE ? env + ATTACK_STEP : ENV_ONE;
	return env > RELEASE_STEP ? env - RELEASE_STEP : 0;
}

static void tick(snd_fx *f)
{
	int32_t d = (int32_t)(f->target - f->cur);

	if (!f->env_tone || (d < 65536 && d > -65536))
		f->cur = f->target;
	else
		f->cur += (uint32_t)((d >> 16) * GLIDE_K);
	f->env_tone = env_to(f->env_tone, f->on == SND_FX_TONE);
	f->env_noise = env_to(f->env_noise, f->on == SND_FX_NOISE);
	f->env_tick = (int32_t)(((uint32_t)f->env_tick * TICK_DEC) >> 16);
	f->amp_tone = (f->env_tone * TONE_AMP) >> 16;
	f->amp_noise = (f->env_noise * NOISE_AMP) >> 16;
	f->amp_tick = (f->env_tick * TICK_AMP) >> 16;
}

void snd_fx_render(snd_fx *f, int16_t *out, uint32_t n)
{
	const uint32_t tick_add = TICK_HZ * f->rate_den;
	uint32_t i, ev = 0;

	for (i = 0; i < n; i++) {
		int32_t acc = 0;

		while (ev < f->n_ev && f->ev[ev].at <= i) {
			const snd_fx_event *e = &f->ev[ev++];
			f->on = e->sound;
			if (e->sound == SND_FX_TONE) f->target = e->step;
			if (e->sound == SND_FX_NOISE) {
				f->nstep = e->step;
				f->lp_k = e->k;
			}
			if (e->tick) {
				f->env_tick = ENV_ONE;
				f->tick_ph = 0;
			}
		}
		f->tick_acc += tick_add;
		while (f->tick_acc >= f->rate_num) {
			f->tick_acc -= f->rate_num;
			tick(f);
		}
		if (f->amp_tone) {
			uint32_t off = (uint32_t)(DEPTH * sine(f->phase << 1)) << 1;
			acc += (sine(f->phase + off) * f->amp_tone) >> 15;
		}
		f->phase += f->cur;
		if (f->amp_noise) {
			uint32_t h = f->hold_ph;
			f->hold_ph += f->nstep;
			if (f->hold_ph < h) {   /* a new value */
				uint32_t s = f->seed;
				s ^= s << 13;
				s ^= s >> 17;
				s ^= s << 5;
				f->seed = s;
				f->hold = (int32_t)(s >> 16) - 32768;
			}
			f->lp += ((f->hold - f->lp) * (int32_t)f->lp_k) >> 15;
			acc += (f->lp * f->amp_noise) >> 15;
		}
		if (f->amp_tick) {
			acc += (sine(f->tick_ph) * f->amp_tick) >> 15;
			f->tick_ph += f->tick_inc;
		}
		out[i] = (int16_t)acc;   /* at most 6400 + 7040 + 3200: no clipping */
	}
}
