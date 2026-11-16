/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The FM synth: three 2-operator tonal voices, each playing the instrument (patch) of its role
 * in the tune (bass, lead, pad), and a drum.
 *
 * Tonal voice: out = sin(p + I * sin(r * p)), the modulator at r times the carrier (its phase is
 * the carrier's times r, so one phase serves both). The patches (the instrument set "D"):
 *
 *            r   index: start -> settles, tau   attack  decay tau -> sustain  release  gain
 *   bass     1   0.7 -> 0.5, 100 ms             3 ms    200 ms -> 0.8         50 ms    1.2
 *   lead     2   1.6 -> 1.3, 200 ms             4 ms    150 ms -> 0.8         50 ms    0.8
 *   pad      1   0.4 -> 0.3, 200 ms             10 ms   200 ms -> 0.85        100 ms   0.75
 *
 * the lead with a vibrato of 18 cents at 6.5 Hz, from 120 ms into the note, ramped in over
 * 200 ms; the pad with a second carrier 4 cents up (same modulator), the two mixed half and half.
 * Attack and release are linear, the decay and the index exponential. A note restarts (attack
 * from the current level, index back to its start, vibrato from its delay) when the voice starts
 * or its period moves by more than 1/32 (half a semitone); smaller changes are glides of the same
 * note. A voice takes the frame's role (snd_frame.role; AUTO plays as lead) only when a note
 * restarts on it, so an instrument never changes inside a note.
 *
 * Drum: xorshift noise, one-pole low-passed (coefficient 0.5), decaying with a 70 ms time
 * constant, weight 0.8; plus a 190 Hz sine decaying with a 30 ms one, weight 0.4; its strength
 * the decoder's drum level, a 50 ms linear release after the decoder stops reporting it.
 *
 * A frame gives the voices at its start and may list note events (method B): each changes one
 * voice, or the drum, from its own sample on, exactly as a change between two frames would.
 *
 * Fixed point: phase 32-bit (2^32 = one cycle); sine table 256 entries of Q15, linearly
 * interpolated with 8 more phase bits; envelope Q16; index in depth units (2^16 / 2 pi of phase
 * per radian), so the phase offset is depth * sin(r p) [Q15] * 2. Envelope, index, vibrato and
 * level change on an 8 kHz tick (0.125 ms, whatever the sample rate), so the decay factors below
 * are constants: exp(-0.125 / tau) in Q16. A voice of gain 1 at level 255 peaks at 8160 * GAIN
 * / 256 (GAIN the overall gain, matching the mix's loudness before the instruments). */

#include "sound_internal.h"

_Static_assert((-1 >> 1) == -1, "signed right shift must be arithmetic");

#define TICK_HZ 8000u
#define ENV_ONE 65536
#define RELEASE_TICKS 400             /* the drum's release: 50 ms */
#define LVL_STEP (65280 / 32)         /* a level change glides over 4 ms */

/* Overall gain, Q8 (256: a gain-1 voice at level 255 peaks at 8160, as every voice did before the
 * instruments); calibrated so the corpus' FM mix keeps its RMS loudness (tests/instruments.rs,
 * with every voice a lead: AUTO). The lead's 2:1 FM is square-ish, more RMS for its peak, so the
 * mix's peaks are 2-3.5 dB below before. */
#define GAIN 210
/* A patch gain given in thousandths, as Q8 with GAIN in (at most 512, so amp stays below 2^16). */
#define G(milli) (((milli) * GAIN + 500) / 1000)

/* The lead's vibrato: delay and ramp in ticks, the LFO's step per tick (6.5 Hz of 2^32), and
 * the depth: 18 cents is a step change of 2^(18/1200) - 1 ~ ln 2 * 18 / 1200 = 681 / 65536. */
#define VIB_DELAY 960u                /* 120 ms */
#define VIB_RAMP 1600u                /* 200 ms */
#define VIB_LFO_INC 3489661u          /* 6.5 / 8000 * 2^32 */
#define VIB_RAMP_MUL 109              /* (ticks into the ramp * 109) >> 8: 0 .. 681 */

/* The pad's second carrier: step = inc * 2^(4/1200) = inc + inc * 152 / 65536. */
#define CHORUS_MUL 152u

/* The drum: its envelopes are Q24 (2^24 = 1), so their slow decay to nothing keeps its time
 * constant: env -= (env >> 8) * DEC >> 8 per tick, DEC = (1 - exp(-0.125 / tau)) in Q16. Its
 * weights are Q8 of a gain-1 voice's 8160: noise 0.8 and tone 0.4 of the drum, the drum mixed at
 * 0.6 against the voices (amplitude = (env >> 12) * lvl * W >> 12: the noise's 31334 at gain 1,
 * out = lp * amp >> 18). */
#define DRUM_ONE (1 << 24)
#define DRUM_DEC 117                  /* noise: tau 70 ms */
#define DRUM_TONE_DEC 273             /* tone: tau 30 ms */
#define DRUM_NOISE_W G(480)
#define DRUM_TONE_W G(240)

typedef struct {
	int32_t depth_start, depth_end;   /* index at note start and settled, depth units */
	int32_t sustain;                  /* Q16 */
	uint16_t attack_step;             /* env increase per tick: ENV_ONE / attack ticks, rounded up */
	uint16_t k_env;                   /* decay to sustain per tick, Q16 */
	uint16_t k_index;                 /* index decay per tick, Q16 */
	uint16_t rel_mul;                 /* release: env decrement per tick = env * rel_mul >> 16, + 1 */
	uint16_t gain;                    /* Q8, GAIN in */
	uint8_t ratio;                    /* modulator : carrier */
	uint8_t vibrato, chorus;          /* the lead's vibrato, the pad's second carrier */
} fm_patch;

/* By enum snd_role (AUTO plays as LEAD; its row is never used). */
static const fm_patch patches[4] = {
	{ 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 },
	/* bass */ { 7301, 5215, 52429, 2731, 65495, 65454, 164, G(1200), 1, 0, 0 },
	/* lead */ { 16689, 13559, 52429, 2048, 65481, 65495, 164, G(800), 2, 1, 0 },
	/* pad  */ { 4172, 3129, 55706, 820, 65495, 65495, 82, G(750), 1, 0, 1 },
};
/* index 0.7/0.5, 1.6/1.3, 0.4/0.3 * 10430.378; sustain 0.8, 0.85 * 65536; attack 3, 4, 10 ms =
 * 24, 32, 80 ticks; decay tau 200, 150, 200 ms; index tau 100, 200, 200 ms; release 400, 400,
 * 800 ticks (rel_mul 65536 / ticks) */

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

static int32_t sine(uint32_t ph)
{
	uint32_t i = ph >> 24;
	int32_t a = sine_q15[i], b = sine_q15[(i + 1u) & 255u];

	return a + (((b - a) * (int32_t)((ph >> 16) & 255u)) >> 8);
}

void snd_fm_reset(snd_fm *f)
{
	fm_rest(f);
}

static int new_note(uint32_t p, uint32_t old)
{
	uint32_t d = p > old ? p - old : old - p;

	return !old || d > old / 32u;
}

/* Applies one frame's decision to a voice; role: the frame's for it, taken if a note starts. */
static void voice_frame(snd_fm *f, snd_fm_voice *v, const snd_voice *in, uint8_t role)
{
	uint32_t p = in->period_q8;
	uint64_t inc = p ? udiv64(f->kfreq, p, 0) : 0;

	if (p && in->level && inc && inc < 0x80000000u) {   /* below half the sample rate */
		v->inc = v->step = (uint32_t)inc;
		v->inc2 = v->inc + (((v->inc >> 8) * CHORUS_MUL) >> 8);
		v->lvl_target = (int32_t)in->level << 8;
		if (v->stage == OFF || v->stage == RELEASE || new_note(p, v->period_q8)) {
			const fm_patch *pt;
			if (role != SND_ROLE_BASS && role != SND_ROLE_PAD) role = SND_ROLE_LEAD;
			if (v->stage == OFF) {
				v->phase = 0;
				v->lvl = v->lvl_target;
			}
			if (role == SND_ROLE_PAD && (v->stage == OFF || v->role != SND_ROLE_PAD))
				v->phase2 = v->phase;   /* the two carriers start together */
			v->role = role;
			pt = &patches[role];
			v->depth_base = pt->depth_end;
			v->depth_ex = pt->depth_start - pt->depth_end;
			v->ticks = 0;
			v->stage = ATTACK;
		}
		v->period_q8 = p;
	} else {
		if (v->stage == ATTACK || v->stage == DECAY) {
			v->stage = RELEASE;
			v->rel_step = (int32_t)(((uint32_t)v->env * patches[v->role].rel_mul) >> 16) + 1;
		}
		v->period_q8 = 0;
	}
}

static void tick(snd_fm *f)
{
	uint32_t i;

	for (i = 0; i < SND_TONAL; i++) {
		snd_fm_voice *v = &f->v[i];
		const fm_patch *pt = &patches[v->role];
		int32_t d;

		switch (v->stage) {
		case ATTACK:
			v->env += pt->attack_step;
			if (v->env >= ENV_ONE) {
				v->env = ENV_ONE;
				v->stage = DECAY;
			}
			break;
		case DECAY:
			v->env = pt->sustain + (int32_t)(((uint32_t)(v->env - pt->sustain) * pt->k_env) >> 16);
			break;
		case RELEASE:
			v->env -= v->rel_step;
			if (v->env <= 0) {
				v->env = 0;
				v->stage = OFF;
			}
			break;
		default:
			continue;
		}
		if (v->stage == ATTACK || v->stage == DECAY)
			v->depth_ex = (int32_t)(((uint32_t)v->depth_ex * pt->k_index) >> 16);
		v->ticks++;
		if (pt->vibrato && v->ticks > VIB_DELAY) {
			/* step = inc * (1 + depth * sin): depth 0 .. 681 / 65536 over the ramp */
			uint32_t r = v->ticks - VIB_DELAY;
			int32_t kv;
			if (r > VIB_RAMP) r = VIB_RAMP;
			kv = (sine(v->ticks * VIB_LFO_INC) * (int32_t)((r * VIB_RAMP_MUL) >> 8)) >> 15;
			v->step = v->inc + (uint32_t)(((int32_t)(v->inc >> 10) * kv) >> 6);
		}
		d = v->lvl_target - v->lvl;
		if (d > LVL_STEP) d = LVL_STEP;
		if (d < -LVL_STEP) d = -LVL_STEP;
		v->lvl += d;
		v->amp = ((v->env >> 8) * (v->lvl >> 8) * pt->gain) >> 9;
	}
	if (f->drum_stage == ATTACK) {
		f->drum_env -= ((f->drum_env >> 8) * DRUM_DEC) >> 8;
		f->drum_tenv -= ((f->drum_tenv >> 8) * DRUM_TONE_DEC) >> 8;
	} else if (f->drum_stage == RELEASE) {
		f->drum_env -= f->drum_rel;
		f->drum_tenv -= f->drum_trel;
		if (f->drum_tenv < 0) f->drum_tenv = 0;
		if (f->drum_env <= 0) {
			f->drum_env = f->drum_tenv = 0;
			f->drum_stage = OFF;
		}
	}
	f->drum_amp = ((f->drum_env >> 12) * f->drum_lvl * DRUM_NOISE_W) >> 12;
	f->drum_tamp = ((f->drum_tenv >> 12) * f->drum_lvl * DRUM_TONE_W) >> 12;
}

/* The drum on (strength 1-255: it starts unless it was already on) or off (0: a release). */
static void drum_set(snd_fm *f, uint8_t strength)
{
	if (strength) {
		if (!f->drum_held) {
			f->drum_env = f->drum_tenv = DRUM_ONE;
			f->drum_phase = 0;
			f->drum_lvl = strength;
			f->drum_stage = ATTACK;
		}
	} else if (f->drum_stage == ATTACK) {
		f->drum_stage = RELEASE;
		f->drum_rel = f->drum_env / RELEASE_TICKS + 1;
		f->drum_trel = f->drum_tenv / RELEASE_TICKS + 1;
	}
	f->drum_held = strength != 0;
}

/* Applies a note event: the voice changes as it would between two frames. */
static void apply_event(snd_fm *f, const snd_frame *fr, const snd_event *ev)
{
	snd_voice in;

	if (ev->voice < SND_TONAL) {
		in.period_q8 = ev->period_q8;
		in.level = ev->level;
		voice_frame(f, &f->v[ev->voice], &in, fr->role[ev->voice]);
	} else if (ev->voice == SND_TONAL) {
		drum_set(f, ev->level);
	}
}

/* The sample of a frame of n samples that an event at T-state t starts on (t * n < 2^32 for
 * n <= SND_MAX_FRAME_SAMPLES). */
static uint32_t event_sample(uint32_t t, uint32_t n)
{
	if (t >= ZX_FRAME_TSTATES) t = ZX_FRAME_TSTATES - 1u;
	return t * n / ZX_FRAME_TSTATES;
}

void snd_fm_render(snd_fm *f, const snd_frame *fr, int16_t *out, uint32_t n)
{
	const uint32_t tick_add = TICK_HZ * f->rate_den;
	const uint32_t n_ev = fr->n_ev < SND_EVENTS ? fr->n_ev : SND_EVENTS;
	uint32_t i, k, ev = 0, next;

	for (k = 0; k < SND_TONAL; k++)
		voice_frame(f, &f->v[k], &fr->tone[k], fr->role[k]);
	drum_set(f, fr->drum);
	next = n_ev ? event_sample(fr->ev[0].t, n) : n;

	for (i = 0; i < n; i++) {
		int32_t acc = 0;

		while (i >= next) {   /* the events that start on this sample */
			apply_event(f, fr, &fr->ev[ev++]);
			next = ev < n_ev ? event_sample(fr->ev[ev].t, n) : n;
		}
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
				const fm_patch *pt = &patches[v->role];
				uint32_t off = (uint32_t)((v->depth_base + v->depth_ex) * sine(ph * pt->ratio)) << 1;
				int32_t x = sine(ph + off);
				if (pt->chorus) x = (x + sine(v->phase2 + off)) >> 1;
				acc += (x * v->amp) >> 17;
			}
			v->phase = ph + v->step;
			v->phase2 += v->inc2;
		}
		if (f->drum_stage != OFF) {
			uint32_t s = f->seed;
			s ^= s << 13;
			s ^= s >> 17;
			s ^= s << 5;
			f->seed = s;
			f->drum_lp += (((int32_t)(s >> 16) - 32768) - f->drum_lp) >> 1;
			acc += (f->drum_lp * f->drum_amp) >> 18;
			acc += (sine(f->drum_phase) * f->drum_tamp) >> 18;
			f->drum_phase += f->drum_inc;
		}
		if (acc > 32767) acc = 32767;
		if (acc < -32768) acc = -32768;
		out[i] = (int16_t)acc;
	}
	while (ev < n_ev)   /* none past the frame's end, unless n is 0 */
		apply_event(f, fr, &fr->ev[ev++]);
}
