/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The short-pulse decoder: engines whose voices each fire a short pulse once
 * per period (pin-pulse engines), the pulses OR-ed onto the speaker. Every
 * rising edge then starts one voice's pulse; the falling edges only end it
 * (the pulse width is the engine's, not a pitch), so the voices are the
 * chains of rising edges a whole period apart. Where two voices' pulses
 * overlap they merge into one longer pulse and the later rise is lost; the
 * engine's width is known (the shortest of the commonest widths: a merge
 * only lengthens), so a pulse 24 T or more longer than it (and shorter than
 * two) gives the lost rise back: one width before its end.
 *
 * The selector's measure (snd_pulses_style): the speaker is high for a small
 * share of the time (each pulse is short against its period), so a frame
 * whose speaker-high time is under 20% of its busy time (both summed over the
 * gaps between its edges under 35,000 T, so silence between sounds does not
 * count) is short pulses, if its pulses also keep one width: at least half of
 * them within 96 T of the commonest width (an engine's pulse is a fixed
 * delay between two OUTs). The second test is there for speaker-flip engines
 * playing two voices in detuned unison: while the voices are nearly in phase
 * their XOR is narrow pulses at twice the pitch, whose width drifts by 270 T
 * or more a frame as the voices beat (N = 255 against 256; 1,100 T at N =
 * 60), so most widths fall outside any 96 T.
 *
 * Voice search: the speaker-flip decoder's (flips.c) on the rising edges,
 * where a step is a whole period, so the shortest is 1,400 T (2.5 kHz). The
 * shortest chaining period wins and a whole multiple of a voice found is
 * skipped, so a voice's every other pulse (an octave down) or a chord's joint
 * period never comes out as a voice. Voices are reported as half-periods
 * (period / 2), so the decisions about every style compare and report alike
 * (decoder.c). */

#include "sound.h"

#define BUSY_GAP 35000u   /* a longer gap between edges is silence, not part of the sound */
#define HIGH_PCT 20u      /* % of the busy time: below it the speaker-high time is short pulses */
#define WBIN 32u          /* pulse-width histogram: bins of 32 T, up to 1,024 T */
#define WBINS 32u
#define WSHARE 50u        /* % of the pulses within 3 bins (96 T) of the commonest width */
#define MERGE_TOL 24u     /* a pulse this much longer than the engine's width holds two */

int snd_pulses_style(snd_decoder *d, const uint32_t *t, uint32_t n, uint8_t ear0, uint32_t *width)
{
	uint16_t w[WBINS];
	uint32_t i, h = 0, b = 0, pulses = 0, best = 0, peak = 0, sum = 0, cnt = 0, wmin = 0xffffffffu;
	uint8_t level = ear0;

	for (i = 0; i < WBINS; i++)
		w[i] = 0;
	for (i = 0; i + 1u < n; i++) {
		uint32_t gap = t[i + 1u] - t[i];
		level ^= 1u;   /* every stored edge flips the speaker */
		if (gap >= BUSY_GAP) continue;
		b += gap;
		if (!level) continue;
		h += gap;
		pulses++;
		if (gap < WBIN * WBINS) w[gap / WBIN]++;
	}
	d->work += n + WBINS;
	if (!b || h * 100u >= b * HIGH_PCT) return 0;
	for (i = 0; i < WBINS; i++) {
		uint32_t c = w[i] + (i ? w[i - 1u] : 0u) + (i + 1u < WBINS ? w[i + 1u] : 0u);
		if (c > best) {
			best = c;
			peak = i;
		}
	}
	if (!best || best * 100u < pulses * WSHARE) return 0;
	/* the engine's pulse width: the shortest widths around the peak (merged
	 * pulses are only ever longer), averaged over 16 T */
	level = ear0;
	for (i = 0; i + 1u < n; i++) {
		uint32_t gap = t[i + 1u] - t[i];
		level ^= 1u;
		if (level && gap / WBIN + 1u >= peak && gap / WBIN <= peak + 1u && gap < wmin) wmin = gap;
	}
	level = ear0;
	for (i = 0; i + 1u < n; i++) {
		uint32_t gap = t[i + 1u] - t[i];
		level ^= 1u;
		if (level && gap >= wmin && gap <= wmin + 16u) {
			sum += gap;
			cnt++;
		}
	}
	d->work += 2u * n;
	*width = cnt ? sum / cnt : 0;
	return 1;
}

uint32_t snd_pulses_join(snd_decoder *d, uint32_t f, uint32_t count, uint32_t *first_n)
{
	uint32_t n = 0, k, i;

	for (k = 0; k < count; k++) {
		const snd_slot *s = &d->slot[(f + k) % SND_HIST];
		const uint32_t *t = d->t[(f + k) % SND_HIST];
		uint32_t base = k * ZX_FRAME_TSTATES;
		/* edge i takes the speaker to ear0 ^ 1 for even i: a rise when that is 1;
		 * a frame that does not fit in the pool whole is left out */
		if (n + s->n + 1u > 2u * SND_EDGES_MAX) break;   /* rises and rises given back: n + 1 at most */
		for (i = s->ear0 ? 1u : 0u; i < s->n; i += 2u) {
			d->pool[n++] = t[i] + base;
			/* a pulse longer than the engine's holds a second one, which began
			 * one width before its end (its rise was lost in the first) */
			if (s->width && i + 1u < s->n && t[i + 1u] - t[i] > s->width + MERGE_TOL &&
			    t[i + 1u] - t[i] < 2u * s->width + MERGE_TOL)
				d->pool[n++] = t[i + 1u] - s->width + base;
		}
		if (k == 0 && first_n) *first_n = n;
	}
	d->work += n;
	return n;
}

uint32_t snd_pulses_decode(snd_decoder *d, uint32_t *t, uint32_t n, uint32_t *hp_q8, uint8_t *drift)
{
	uint32_t k, explained = snd_flips_decode(d, t, n, 1, hp_q8, drift);

	for (k = 0; k < SND_TONAL; k++)
		hp_q8[k] /= 2u;
	return explained;
}
