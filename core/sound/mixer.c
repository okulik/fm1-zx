/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The mixer: Beeper mode copies the beeper; an FM mode plays the sound 3 frames
 * (SND_DELAY_FRAMES) late, each moment either the FM voices or the delayed
 * beeper, as the decoder decides.
 *
 * The delay is a sample FIFO (the ring): every frame pushes its n beeper
 * samples, and once the first SND_DELAY_FRAMES frames are in, every frame pops
 * n first. Its fill then stays at those first frames' sample count, so the
 * delayed stream is the beeper stream shifted by a constant: nothing repeated
 * or dropped, though frames have 880 or 881 samples. The decoder's decision for
 * frame t - 3 arrives with frame t, when the ring plays frame t - 3's sound.
 *
 * Three Q15 gains ramp by `step` per sample (a 4 ms crossfade):
 *   live_g  the live beeper: 1 in Beeper mode, 0 in an FM mode;
 *   del_g   the delayed path: 1 in an FM mode once `mute` has run out;
 *   fm_g    within the delayed path, FM against the delayed beeper: follows
 *           the hysteresis' fm_on.
 * Beeper -> FM: live_g fades out at once; `mute` holds del_g at 0 until the
 * ring reaches the switch frame's first sample, then del_g fades in, so the
 * two fades meet on the same sound (60 ms apart) and the 3 frames already
 * played are not played again. FM -> Beeper: live_g and del_g cross at once
 * (the FM synth keeps rendering while del_g fades). In steady Beeper mode the
 * output is a plain copy and the ring and decoder are still fed. */

#include "sound.h"

_Static_assert((-1 >> 1) == -1, "signed right shift must be arithmetic");

#define ONE 32768

void snd_mixer_init(snd_mixer *m, uint32_t rate_num, uint32_t rate_den)
{
	uint32_t i, xf;

	if (!rate_num) rate_num = 1;
	if (!rate_den) rate_den = 1;
	snd_decoder_init(&m->dec);
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
	m->started = 0;
	m->fm_on = 0;
	m->run = 0;
}

static int32_t toward(int32_t g, int32_t target, int32_t step)
{
	if (g < target) {
		g += step;
		if (g > target) g = target;
	} else if (g > target) {
		g -= step;
		if (g < target) g = target;
	}
	return g;
}

/* FM after SND_ENTER_FRAMES confident decisions in a row, beeper after
 * SND_LEAVE_FRAMES unconfident ones. */
static void hysteresis(snd_mixer *m, uint8_t confident)
{
	if ((confident != 0) == (m->fm_on != 0)) {
		m->run = 0;
		return;
	}
	m->run++;
	if (m->run >= (m->fm_on ? SND_LEAVE_FRAMES : SND_ENTER_FRAMES)) {
		m->fm_on = !m->fm_on;
		m->run = 0;
	}
}

void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_edge *e, uint32_t n_edges, uint8_t start_level,
                     const int16_t *beeper, uint32_t n, int16_t *out)
{
	snd_frame fr;
	int fm_mode, primed;
	uint32_t i;

	if (n > SND_MAX_FRAME_SAMPLES) {
		for (i = 0; i < n; i++)
			out[i] = beeper[i];
		return;
	}
	if (mode >= SND_MODE_COUNT) mode = SND_MODE_BEEPER;
	fm_mode = mode != SND_MODE_BEEPER;
	primed = m->primed >= SND_DELAY_FRAMES;

	snd_decoder_frame(&m->dec, e, n_edges, start_level, &fr);
	if (primed) hysteresis(m, fr.confident);   /* earlier decisions are about frames before init */

	if (!m->started) {
		/* the first frame after init: no fade from the silence before it */
		m->started = 1;
		m->live_g = fm_mode ? 0 : ONE;
		m->del_g = fm_mode ? ONE : 0;
		m->fm_g = m->fm_on ? ONE : 0;
	} else if (fm_mode && m->mode == SND_MODE_BEEPER) {
		m->mute = m->fill;   /* the samples before this frame's */
		snd_fm_init(&m->fm, m->fm.rate_num, m->fm.rate_den);
		m->fm_g = m->fm_on ? ONE : 0;
	} else if (!fm_mode && m->mode != SND_MODE_BEEPER) {
		m->mute = 0;
	}
	m->mode = mode;

	if (!fm_mode && m->live_g == ONE && m->del_g == 0) {
		for (i = 0; i < n; i++)
			out[i] = beeper[i];
		if (primed) {
			uint32_t k = n < m->fill ? n : m->fill;
			m->rd += k;
			if (m->rd >= SND_RING_SAMPLES) m->rd -= SND_RING_SAMPLES;
			m->fill -= k;
		}
	} else {
		if (primed && (fm_mode || m->del_g)) {
			snd_frame silent;
			const snd_frame *src = &fr;
			if (!fr.confident) {   /* FM voices only what the decoder is sure of */
				for (i = 0; i < SND_TONAL; i++) {
					silent.tone[i].period_q8 = 0;
					silent.tone[i].level = 0;
				}
				silent.drum = 0;
				silent.style = SND_STYLE_NONE;
				silent.confident = 0;
				src = &silent;
			}
			snd_fm_render(&m->fm, src, out, n);
		} else {
			for (i = 0; i < n; i++)
				out[i] = 0;
		}
		for (i = 0; i < n; i++) {
			int32_t live = beeper[i], bd = 0, del, o;
			int popped = 0;

			if (primed && m->fill) {
				bd = m->ring[m->rd];
				if (++m->rd == SND_RING_SAMPLES) m->rd = 0;
				m->fill--;
				popped = 1;
			}
			m->fm_g = toward(m->fm_g, m->fm_on ? ONE : 0, m->step);
			del = bd + (((out[i] - bd) * m->fm_g) >> 15);
			m->live_g = toward(m->live_g, fm_mode ? 0 : ONE, m->step);
			m->del_g = toward(m->del_g, fm_mode && !m->mute ? ONE : 0, m->step);
			if (popped && m->mute) m->mute--;
			o = (live * m->live_g + del * m->del_g) >> 15;
			if (o > 32767) o = 32767;
			if (o < -32768) o = -32768;
			out[i] = (int16_t)o;
		}
	}

	for (i = 0; i < n; i++) {
		if (m->fill == SND_RING_SAMPLES) {   /* only if frames outgrow the first ones threefold */
			if (++m->rd == SND_RING_SAMPLES) m->rd = 0;
			m->fill--;
		}
		m->ring[m->wr] = beeper[i];
		if (++m->wr == SND_RING_SAMPLES) m->wr = 0;
		m->fill++;
	}
	if (m->primed < SND_DELAY_FRAMES) m->primed++;
}
