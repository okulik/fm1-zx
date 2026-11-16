/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The mixer: Beeper mode copies the beeper; an FM mode plays the sound 3 frames
 * (SND_DELAY_FRAMES) late, each moment either the FM voices or the delayed
 * beeper, as the mode's method decides: FM_A the decoder (method A), FM_B the
 * known engines (method B). Both methods are fed every frame in every mode.
 *
 * The delay is a sample FIFO (the ring): every frame pushes its n beeper
 * samples, and once the first SND_DELAY_FRAMES frames are in, every frame pops
 * n first. Its fill then stays at those first frames' sample count, so the
 * delayed stream is the beeper stream shifted by a constant: nothing repeated
 * or dropped, though frames have 880 or 881 samples. Each method's decision for
 * frame t - 3 arrives with frame t, when the ring plays frame t - 3's sound.
 *
 * Three Q15 gains ramp by `step` per sample (a 4 ms crossfade):
 *   live_g  the live beeper: 1 in Beeper mode, 0 in an FM mode;
 *   del_g   the delayed path: 1 in an FM mode once `mute` has run out;
 *   fm_g    within the delayed path, FM against the delayed beeper: follows
 *           the hysteresis' fm_on (one hysteresis, fed by the mode's method).
 * The hysteresis leaves FM on the second unconfident decision in a row, not the
 * first: a lone one inside music (a note boundary, a frame partly explained)
 * keeps the synth's voices as they are (the last confident decision's), so the
 * music does not drop to the beeper for 2-3 frames (a click); other sound loses
 * its first frame.
 * Beeper -> FM: live_g fades out at once; `mute` holds del_g at 0 until the
 * ring reaches the switch frame's first sample, then del_g fades in, so the
 * two fades meet on the same sound (60 ms apart) and the 3 frames already
 * played are not played again. FM -> Beeper: live_g and del_g cross at once
 * (the FM synth keeps rendering while del_g fades). FM_A <-> FM_B: nothing but
 * the method (and the live rule, afresh) changes; the ring, the gains and the
 * synth carry on, so only fm_g moves if the new method decides otherwise. In
 * steady Beeper mode the output is a plain copy and the ring and both methods
 * are still fed.
 *
 * The effects follower (fx_read.c, fx.c) reads every frame in every mode, so its history is
 * current at a switch; in FM: engines with effects on, where its gate says PLAY, it replaces the
 * delayed beeper under a fourth gain, fx_g (the same 4 ms ramps): the delayed path's beeper side
 * is bd + (fx - bd) * fx_g. It renders only while fx_g is or is heading above 0, so with effects
 * off (or another mode) the output is exactly what it was without it.
 *
 * The live rule (roles.c) sees every frame the synth renders, just before it, and gives each
 * voice the method left without a role its role by pitch; it restarts with the synth at a
 * Beeper -> FM switch, so the roles there come from the music after the switch, and on its own at
 * an FM_A <-> FM_B switch (method A's slots are not method B's channels). */

#include "sound_internal.h"

_Static_assert((-1 >> 1) == -1, "signed right shift must be arithmetic");

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

/* FM after `enter` confident decisions in a row, beeper after SND_LEAVE_FRAMES
 * unconfident ones. */
static void hysteresis(snd_mixer *m, uint8_t confident, uint32_t enter)
{
	if ((confident != 0) == (m->fm_on != 0)) {
		m->run = 0;
		return;
	}
	m->run++;
	if (m->run >= (m->fm_on ? SND_LEAVE_FRAMES : enter)) {
		m->fm_on = !m->fm_on;
		m->run = 0;
	}
}

void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_machine *zm, const zx_edge *e, uint32_t n_edges,
                     uint8_t start_level, const int16_t *beeper, uint32_t n, int16_t *out)
{
	snd_frame fa, fb, *fr;
	int fm_mode, primed;
	int32_t fx_t;
	uint32_t i;

	if (n > SND_MAX_FRAME_SAMPLES) {
		for (i = 0; i < n; i++)
			out[i] = beeper[i];
		return;
	}
	if (mode >= SND_MODE_COUNT) mode = SND_MODE_BEEPER;
	fm_mode = mode != SND_MODE_BEEPER;
	primed = m->primed >= SND_DELAY_FRAMES;

	snd_decoder_frame(&m->dec, e, n_edges, start_level, &fa);
	snd_b_frame(&m->b, zm, e, n_edges, start_level, &fb);
	m->fx.read(&m->fx, &m->dec, &m->b, n);
	if (!m->effects || mode != SND_MODE_FM_B) m->fx.gate = SND_FX_OFF;
	fx_t = m->fx.gate == SND_FX_PLAY ? ONE : 0;
	if (!fx_t && !m->fx_g) m->fx.idle = 1;   /* not rendered this frame: its voice restarts afresh */
	/* the FM mode's method decides (Beeper mode: the last FM mode's, so its fade-out and a
	 * switch back carry on with it); B's claims are exact, so one is enough to enter FM */
	if (fm_mode) m->fm_choice = mode;
	fr = m->fm_choice == SND_MODE_FM_B ? &fb : &fa;
	if (primed)   /* earlier decisions are about frames before init */
		hysteresis(m, fr->confident, m->fm_choice == SND_MODE_FM_B ? 1u : SND_ENTER_FRAMES);

	if (!m->started) {
		/* the first frame after init: no fade from the silence before it */
		m->started = 1;
		m->live_g = fm_mode ? 0 : ONE;
		m->del_g = fm_mode ? ONE : 0;
		m->fm_g = m->fm_on ? ONE : 0;
	} else if (fm_mode && m->mode != mode) {
		if (m->mode == SND_MODE_BEEPER) {
			m->mute = m->fill;   /* the samples before this frame's */
			snd_fm_reset(&m->fm);
			m->fm_g = m->fm_on ? ONE : 0;
		}
		/* the live rule afresh at an FM turn-on, and at a switch between methods A and B (A's
		 * slots are not B's channels) */
		roles_rest(&m->roles);
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
			if (!fr->confident) {
				/* FM voices only what the method is sure of; still in FM (a lone
				 * unconfident decision), the synth's voices carry on as they are */
				for (i = 0; i < SND_TONAL; i++) {
					fr->tone[i].period_q8 = m->fm_on ? m->fm.v[i].period_q8 : 0;
					fr->tone[i].level = m->fm_on ? (uint8_t)(m->fm.v[i].lvl_target >> 8) : 0;
				}
				fr->drum = 0;
				fr->n_ev = 0;
			}
			snd_roles_frame(&m->roles, fr);   /* the live rule's roles for the voices with none */
			snd_fm_render(&m->fm, fr, out, n);
		} else {
			for (i = 0; i < n; i++)
				out[i] = 0;
		}
		if (fx_t || m->fx_g) snd_fx_render(&m->fx, m->fx.buf, n);
		for (i = 0; i < n; i++) {
			int32_t live = beeper[i], bd = 0, del, o;
			int popped = 0;

			if (primed && m->fill) {
				bd = m->ring[m->rd];
				if (++m->rd == SND_RING_SAMPLES) m->rd = 0;
				m->fill--;
				popped = 1;
			}
			m->fx_g = toward(m->fx_g, fx_t, m->step);
			bd += ((m->fx.buf[i] - bd) * m->fx_g) >> 15;
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
