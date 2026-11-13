/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The FM synth: three 2-operator tonal voices and a noise drum.
 *
 * Tonal voice: out = sin(p + I * sin(p)), carrier and modulator at a 1:1
 * ratio (so one phase serves both). The index I starts at 2.5 for bass notes
 * (below 196 Hz) and 1.6 above, and decays towards 1.0 / 0.6 with a 120 ms
 * time constant. Envelope: 4 ms linear attack, then a fall to the 0.75
 * sustain level (80 ms time constant), a 50 ms linear release when the voice
 * stops. A note restarts (attack from the current level, index back to its
 * start) when the voice starts or its period moves by more than 1/32
 * (half a semitone); smaller changes are glides of the same note.
 *
 * Drum: xorshift noise, one-pole low-passed, with a 40 ms decay while the
 * decoder reports it and a 50 ms linear release after.
 *
 * Fixed point: phase 32-bit (2^32 = one cycle); sine table 256 entries of
 * Q15, linearly interpolated with 8 more phase bits; envelope Q16; index in
 * depth units (2^16 / 2 pi of phase per radian), so the phase offset is
 * depth * sin(p) [Q15] * 2. Envelope, index and level change on an 8 kHz
 * tick (0.125 ms, whatever the sample rate), so the decay factors below are
 * constants: exp(-0.125 / tau) in Q16. A voice at level 255 peaks at 8160. */

#include "sound.h"

_Static_assert((-1 >> 1) == -1, "signed right shift must be arithmetic");

#define TICK_HZ 8000u
#define ENV_ONE 65536
#define ENV_SUSTAIN 49152             /* 0.75 */
#define ATTACK_STEP (ENV_ONE / 32)    /* 4 ms = 32 ticks */
#define RELEASE_TICKS 400             /* 50 ms */
#define K_ENV 65434u                  /* tau 80 ms */
#define K_INDEX 65468u                /* tau 120 ms */
#define K_DRUM 65332u                 /* tau 40 ms */
#define LVL_STEP (65280 / 32)         /* a level change glides over 4 ms */

#define BASS_PERIOD_Q8 4571429u       /* 196 Hz: 3500000 * 256 / 196 */
#define DEPTH_BASS_START 26076        /* 2.5 */
#define DEPTH_BASS_END 10430          /* 1.0 */
#define DEPTH_LEAD_START 16689        /* 1.6 */
#define DEPTH_LEAD_END 6258           /* 0.6 */

#define CPU_Q8 896000000u             /* 3500000 * 256: frequency = CPU_Q8 / period_q8 */

enum { OFF, ATTACK, DECAY, RELEASE };

static const int16_t sine_q15[256] = {
	     0,    804,   1608,   2410,   3212,   4011,   4808,   5602,   6393,   7179,   7962,   8739,
	  9512,  10278,  11039,  11793,  12539,  13279,  14010,  14732,  15446,  16151,  16846,  17530,
	 18204,  18868,  19519,  20159,  20787,  21403,  22005,  22594,  23170,  23731,  24279,  24811,
	 25329,  25832,  26319,  26790,  27245,  27683,  28105,  28510,  28898,  29268,  29621,  29956,
	 30273,  30571,  30852,  31113,  31356,  31580,  31785,  31971,  32137,  32285,  32412,  32521,
	 32609,  32678,  32728,  32757,  32767,  32757,  32728,  32678,  32609,  32521,  32412,  32285,
	 32137,  31971,  31785,  31580,  31356,  31113,  30852,  30571,  30273,  29956,  29621,  29268,
	 28898,  28510,  28105,  27683,  27245,  26790,  26319,  25832,  25329,  24811,  24279,  23731,
	 23170,  22594,  22005,  21403,  20787,  20159,  19519,  18868,  18204,  17530,  16846,  16151,
	 15446,  14732,  14010,  13279,  12539,  11793,  11039,  10278,   9512,   8739,   7962,   7179,
	  6393,   5602,   4808,   4011,   3212,   2410,   1608,    804,      0,   -804,  -1608,  -2410,
	 -3212,  -4011,  -4808,  -5602,  -6393,  -7179,  -7962,  -8739,  -9512, -10278, -11039, -11793,
	-12539, -13279, -14010, -14732, -15446, -16151, -16846, -17530, -18204, -18868, -19519, -20159,
	-20787, -21403, -22005, -22594, -23170, -23731, -24279, -24811, -25329, -25832, -26319, -26790,
	-27245, -27683, -28105, -28510, -28898, -29268, -29621, -29956, -30273, -30571, -30852, -31113,
	-31356, -31580, -31785, -31971, -32137, -32285, -32412, -32521, -32609, -32678, -32728, -32757,
	-32767, -32757, -32728, -32678, -32609, -32521, -32412, -32285, -32137, -31971, -31785, -31580,
	-31356, -31113, -30852, -30571, -30273, -29956, -29621, -29268, -28898, -28510, -28105, -27683,
	-27245, -26790, -26319, -25832, -25329, -24811, -24279, -23731, -23170, -22594, -22005, -21403,
	-20787, -20159, -19519, -18868, -18204, -17530, -16846, -16151, -15446, -14732, -14010, -13279,
	-12539, -11793, -11039, -10278,  -9512,  -8739,  -7962,  -7179,  -6393,  -5602,  -4808,  -4011,
	 -3212,  -2410,  -1608,   -804,
};

/* n / d by restoring division: no 64-bit divide helper on the device. */
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

static int32_t sine(uint32_t ph)
{
	uint32_t i = ph >> 24;
	int32_t a = sine_q15[i], b = sine_q15[(i + 1u) & 255u];

	return a + (((b - a) * (int32_t)((ph >> 16) & 255u)) >> 8);
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

static int new_note(uint32_t p, uint32_t old)
{
	uint32_t d = p > old ? p - old : old - p;

	return !old || d > old / 32u;
}

/* Applies one frame's decision to a voice. */
static void voice_frame(snd_fm *f, snd_fm_voice *v, const snd_voice *in)
{
	uint32_t p = in->period_q8;
	uint64_t inc = p ? udiv64(f->kfreq, p, 0) : 0;

	if (p && in->level && inc && inc < 0x80000000u) {   /* below half the sample rate */
		v->inc = (uint32_t)inc;
		v->lvl_target = (int32_t)in->level << 8;
		if (v->stage == OFF || v->stage == RELEASE || new_note(p, v->period_q8)) {
			int bass = p > BASS_PERIOD_Q8;
			if (v->stage == OFF) {
				v->phase = 0;
				v->lvl = v->lvl_target;
			}
			v->depth_base = bass ? DEPTH_BASS_END : DEPTH_LEAD_END;
			v->depth_ex = (bass ? DEPTH_BASS_START : DEPTH_LEAD_START) - v->depth_base;
			v->stage = ATTACK;
		}
		v->period_q8 = p;
	} else {
		if (v->stage == ATTACK || v->stage == DECAY) {
			v->stage = RELEASE;
			v->rel_step = v->env / RELEASE_TICKS + 1;
		}
		v->period_q8 = 0;
	}
}

static void tick(snd_fm *f)
{
	uint32_t i;

	for (i = 0; i < SND_TONAL; i++) {
		snd_fm_voice *v = &f->v[i];
		int32_t d;

		switch (v->stage) {
		case ATTACK:
			v->env += ATTACK_STEP;
			if (v->env >= ENV_ONE) {
				v->env = ENV_ONE;
				v->stage = DECAY;
			}
			break;
		case DECAY:
			v->env = ENV_SUSTAIN + (int32_t)(((uint32_t)(v->env - ENV_SUSTAIN) * K_ENV) >> 16);
			break;
		case RELEASE:
			v->env -= v->rel_step;
			if (v->env <= 0) {
				v->env = 0;
				v->stage = OFF;
			}
			break;
		default:
			break;
		}
		if (v->stage == ATTACK || v->stage == DECAY)
			v->depth_ex = (int32_t)(((uint32_t)v->depth_ex * K_INDEX) >> 16);
		d = v->lvl_target - v->lvl;
		if (d > LVL_STEP) d = LVL_STEP;
		if (d < -LVL_STEP) d = -LVL_STEP;
		v->lvl += d;
		v->amp = (v->env >> 8) * (v->lvl >> 8);
	}
	if (f->drum_stage == ATTACK) {
		f->drum_env = (int32_t)(((uint32_t)f->drum_env * K_DRUM) >> 16);
	} else if (f->drum_stage == RELEASE) {
		f->drum_env -= f->drum_rel;
		if (f->drum_env <= 0) {
			f->drum_env = 0;
			f->drum_stage = OFF;
		}
	}
	f->drum_amp = (f->drum_env >> 8) * f->drum_lvl;
}

void snd_fm_render(snd_fm *f, const snd_frame *fr, int16_t *out, uint32_t n)
{
	const uint32_t tick_add = TICK_HZ * f->rate_den;
	uint32_t i, k;

	for (k = 0; k < SND_TONAL; k++)
		voice_frame(f, &f->v[k], &fr->tone[k]);
	if (fr->drum) {
		if (!f->drum_held) {
			f->drum_env = ENV_ONE;
			f->drum_lvl = fr->drum;
			f->drum_stage = ATTACK;
		}
	} else if (f->drum_stage == ATTACK) {
		f->drum_stage = RELEASE;
		f->drum_rel = f->drum_env / RELEASE_TICKS + 1;
	}
	f->drum_held = fr->drum != 0;

	for (i = 0; i < n; i++) {
		int32_t acc = 0;

		f->tick_acc += tick_add;
		while (f->tick_acc >= f->rate_num) {
			f->tick_acc -= f->rate_num;
			tick(f);
		}
		for (k = 0; k < SND_TONAL; k++) {
			snd_fm_voice *v = &f->v[k];
			uint32_t ph = v->phase;

			if (v->stage == OFF) continue;
			if (v->amp) {
				uint32_t off = (uint32_t)((v->depth_base + v->depth_ex) * sine(ph)) << 1;
				acc += (sine(ph + off) * v->amp) >> 18;
			}
			v->phase = ph + v->inc;
		}
		if (f->drum_stage != OFF) {
			uint32_t s = f->seed;
			s ^= s << 13;
			s ^= s >> 17;
			s ^= s << 5;
			f->seed = s;
			f->drum_lp += (((int32_t)(s >> 16) - 32768) - f->drum_lp) >> 1;
			acc += (f->drum_lp * f->drum_amp) >> 18;
		}
		if (acc > 32767) acc = 32767;
		if (acc < -32768) acc = -32768;
		out[i] = (int16_t)acc;
	}
}
