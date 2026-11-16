/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The live rule: each tonal voice slot's role by its pitch against the others'.
 *
 * Per slot, a slow running average of the pitch it plays (log2 of its period, so a semitone is
 * the same step anywhere) over the frames it sounds in (the last pitch it sounds in each frame,
 * its events included): an exponential average of time constant 2^TAU_SHIFT frames, starting as
 * a mean (the first frame's pitch alone, then a weight halving at each doubling of the frames
 * heard), so a slot's first notes are ranked by their own pitch. The slots are kept in an order,
 * lowest average first, slots never heard last; two neighbours swap only when the higher one's
 * average lies MARGIN below the other's, so a brief crossing leaves them (two slots both heard
 * for the first time swap on their pitch alone; a slot new among others starts above them and
 * moves down past each it lies MARGIN below). The slots heard within the last WINDOW frames are
 * ranked: of those that carry no role of their own in the frame (SND_ROLE_AUTO), the lowest is
 * the bass, the highest the lead, a third the pad, a lone one the lead. A slot silent for longer
 * leaves the ranking and keeps its history, so a bass line that pauses comes back as the bass.
 * Every sounding slot is tracked, whatever decides its role.
 *
 * The rule gives a role every frame; the synth takes a voice's role only when a note starts on it
 * (fm.c), so a voice never changes instrument inside a note.
 *
 * The values (2.56 s, 3 semitones, 4 s: the starting points kept), checked on the corpus over 3
 * minutes each in the mixer (tests/roles.rs): in FM: engines Colony 8 settles on its channels'
 * roles (0 bass, 2 pad, 1 lead) as each channel enters and never changes them again, and
 * Castaway (two voices) keeps the roles of its first notes; in FM: sound Castaway's two slots
 * swap once, at 7 s. Time constants of 1.28 and 5.12 s, margins of 4 and 5 semitones and windows
 * of 3 and 5 s change none of that. FM: sound on Colony 8 is the one that moves: the decoder's
 * slots are its voices in the order found, not channels, and they trade roles 28 times in the 3
 * minutes (38 at 1.28 s, 24 at 5.12 s or 5 semitones, 54 with a 3 s window, 28 with 5 s). */

#include "sound_internal.h"

#define TAU_SHIFT 7       /* the averaging time constant: 128 frames, 2.56 s */
#define MARGIN (64 << 8)  /* a swap needs 3 semitones (a quarter octave; Q16 octaves) */
#define WINDOW 200u       /* "heard recently": within 4 s (frames) */

_Static_assert(WINDOW <= 0xff && TAU_SHIFT <= 8, "the counters are bytes");

/* log2(p) in Q8, p > 0: the exponent, then the mantissa's 8 bits by squaring. */
static int32_t log2_q8(uint32_t p)
{
	int32_t e = 31 - __builtin_clz(p), r = e << 8, i;
	uint32_t x = e >= 15 ? p >> (e - 15) : p << (15 - e);   /* Q15, 1 <= x < 2 */

	for (i = 128; i; i >>= 1) {
		x = (x * x) >> 15;
		if (x >= 0x10000u) {
			x >>= 1;
			r += i;
		}
	}
	return r;
}

void snd_roles_init(snd_roles *r)
{
	roles_rest(r);
}

void snd_roles_frame(snd_roles *r, snd_frame *fr)
{
	const uint32_t n_ev = fr->n_ev < SND_EVENTS ? fr->n_ev : SND_EVENTS;
	uint32_t k, i, s, fresh = 0, ranked = 0;   /* fresh: bit k, slot k heard for the first time */

	for (k = 0; k < SND_TONAL; k++) {
		/* the pitch the slot last sounds in the frame */
		uint32_t p = fr->tone[k].level ? fr->tone[k].period_q8 : 0;
		for (i = 0; i < n_ev; i++)
			if (fr->ev[i].voice == k && fr->ev[i].period_q8 && fr->ev[i].level)
				p = fr->ev[i].period_q8;
		if (p) {
			/* the weight: 1, 1/2, 1/4 (2 frames), 1/8 (4), ... down to 2^-TAU_SHIFT */
			s = 31 - __builtin_clz(2u * r->n[k] + 1u);
			if (s > TAU_SHIFT) s = TAU_SHIFT;
			if (!s) fresh |= 1u << k;
			r->avg[k] += ((log2_q8(p) << 8) - r->avg[k]) >> s;
			if (r->n[k] < 0xff) r->n[k]++;
			r->left[k] = WINDOW;
		} else if (r->left[k]) {
			r->left[k]--;
		}
		/* ranked: heard within the window (so with history), with no role in the frame */
		if (r->left[k] && fr->role[k] == SND_ROLE_AUTO) ranked++;
	}

	/* neighbours swap past the margin (two heard for the first time on their pitch alone; a slot
	 * never heard, at SND_ROLES_NONE, goes last): pairs 0-1, 1-2, 0-1 sort three */
	for (s = 0; s < 3; s++) {
		const uint32_t a = r->order[s & 1], b = r->order[(s & 1) + 1];
		if (r->avg[b] > r->avg[a] + (fresh >> a & fresh >> b & 1 ? 0 : MARGIN)) {
			r->order[s & 1] = (uint8_t)b;
			r->order[(s & 1) + 1] = (uint8_t)a;
		}
	}

	for (i = 0, k = 0; i < SND_TONAL; i++) {
		s = r->order[i];
		if (fr->role[s] != SND_ROLE_AUTO) continue;
		if (r->left[s]) {
			r->role[s] = k + 1 == ranked ? SND_ROLE_LEAD : k ? SND_ROLE_PAD : SND_ROLE_BASS;
			k++;
		}
		fr->role[s] = r->role[s];
	}
}
