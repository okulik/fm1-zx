/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The speaker-flip decoder: engines whose voices each flip the speaker when
 * their counter wraps (XOR mixing), so every edge, rising or falling, is a
 * half-period of exactly one voice.
 *
 * Chains: a chain of half-period p through an edge links it to the edge
 * nearest one half-period on, within 28 T or 1/256 of p, whichever is more,
 * and so on forward (re-estimating p from the chain so far) and back. A link
 * may skip one missing edge (two half-periods, at twice the tolerance, since
 * the two half-periods' errors add up), never two in a row.
 *
 * Voice search, up to SND_TONAL times: the candidates are the gaps from 8
 * seed edges spread over the window to each seed's next 12 edges, within the
 * half-periods of 2.5 kHz .. 30 Hz. In ascending order, each is followed as a
 * chain from its seed. The first whose chain has at least 4 edges and runs
 * through at least 70% of the window's span is a voice: the shortest chaining
 * period, so two voices in a simple ratio (3:4) never come out as their joint
 * period, and a chain through the window rejects the isolated equal gaps of a
 * drum waveform. A chain whose period is a whole multiple (2-32 times) of a
 * voice already found, or of a chain through 30% of the window, is skipped:
 * it is that voice's sub-chain or a chord's joint period, extended by chance
 * hits where the window holds a change of note. The voice's half-period is
 * its chain's span over its steps; its edges, and any further 3-edge chains
 * of its period (stretches past two missing edges), leave the pool, and the
 * search repeats on the rest. A voice drifts if the two halves of its chain
 * (split at the edge nearest the middle) measure periods more than 1/128
 * apart (about 0.14 semitone) plus 2 T: a pitch sweep, not a held note.
 *
 * The short-pulse decoder (pulses.c) runs the same search on rising edges
 * only, where each step is a whole period (the shortest 1,400 T). There a
 * multiple of a voice found is kept: that voice's rises have left the pool
 * (a pulse is one voice's, never a part of two), so a chain at twice its
 * period is another voice an octave below.
 *
 * Why the tolerances: XOR engines lose an edge where both counters wrap in
 * the same loop pass (the skip), and a reload in one counter stretches the
 * other's half-period by about 21 T (the 28 T tolerance; a skip across a lost
 * edge was measured 31 T off twice the half-period). */

#include "sound.h"

#define HMIN 700u      /* 2.5 kHz */
#define HMAX 58333u    /* 30 Hz */
#define SEEDS 8u
#define AHEAD 12u
#define TOL_MIN 28u
#define MULT_MAX 32u   /* whole multiples of a voice's period checked */
#define MIN_NODES 4u   /* edges of a voice found in a window (3 suffice to follow a known voice) */

/* Short pulses (pulses.c): a step is a whole period, and the engine's work
 * for its other voices can shift a pulse by tens of T, so 28 T or 1/64 of
 * the step (a quarter of a semitone): with 1/256 a generated pin-pulse tune
 * kept 47% of its voice-frames with pulses shifted by up to 20 T but only 14%
 * up to 40 T; with 1/64, 54% and 49%, and none wrong. */
#define TOL_DIV_P 64u

static uint32_t tol_pulse(uint32_t step)
{
	uint32_t t = step / TOL_DIV_P;
	return t > TOL_MIN ? t : TOL_MIN;
}

static uint32_t tol_of(uint32_t step)
{
	uint32_t t = step >> 8;
	return t > TOL_MIN ? t : TOL_MIN;
}

/* Work accounting: every binary-search step, every edge looked at, every
 * candidate moved while sorting and every edge of a pass over the pool costs
 * one unit. Once d->work reaches d->work_cap the searches find nothing, so
 * the decoder stops; it then treats the frame as undecodable (decoder.c). */
static int spent(const snd_decoder *d)
{
	return d->work >= d->work_cap;
}

/* Index of the edge nearest to `target` within `tol`, searching from index
 * `from` forward (dir > 0) or backward (dir < 0), passing over edges marked in
 * `skip` (if given); n if none. A galloping search finds the edges within the
 * tolerance, so a link costs at most about 2 log2(n) whatever the gap. */
static uint32_t nearest(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t from, uint32_t target, uint32_t tol,
                        int dir, const uint8_t *skip)
{
	uint32_t best = n, best_d = tol + 1u, lo, hi, j, lim = target > tol ? target - tol : 0, w = 0;

	if (spent(d)) return n;
	/* the first edge at or after target - tol: a few steps out from `from`
	 * (it is near for most links), then galloping and halving */
	if (dir > 0) {
		uint32_t step = 1;
		lo = from;
		hi = n;
		while (lo < n && lo < from + 4u && t[lo] < lim) {   /* the near case first */
			w++;
			lo++;
		}
		if (lo == n || t[lo] >= lim) step = 0;   /* found (or none): no search */
		while (step && lo + step < n && t[lo + step - 1u] < lim) {
			w++;
			step *= 2u;
		}
		if (!step) hi = lo;
		else {
			if (lo + step < n) hi = lo + step;
			if (step > 1u) lo += step / 2u;
		}
	} else {
		uint32_t step = 1;
		lo = 0;
		hi = from + 1u;
		while (step <= from && t[from - step + 1u] >= lim) {
			w++;
			step *= 2u;
		}
		if (step <= from) lo = from - step + 1u;
		if (step > 1u) hi = from - step / 2u + 2u;
		if (hi > from + 1u) hi = from + 1u;
	}
	while (lo < hi) {
		uint32_t mid = (lo + hi) / 2u;
		w++;
		if (t[mid] < lim) lo = mid + 1u;
		else hi = mid;
	}
	for (j = lo; j < (dir > 0 ? n : from + 1u) && t[j] <= target + tol; j++) {
		uint32_t dd = t[j] > target ? t[j] - target : target - t[j];
		w++;
		if (dd < best_d && !(skip && skip[j])) {
			best_d = dd;
			best = j;
		}
	}
	d->work += w;   /* a local count: t may alias d's own arrays */
	return best;
}

typedef struct {
	uint32_t nodes;     /* edges in the chain, listed in d->chain */
	uint32_t steps;     /* half-periods it spans (a skip counts 2) */
	uint32_t first, last;
} chain_t;

/* One link from edge `cur` in direction dir: the edge one step on, or (if
 * `may_skip`) two steps on, with twice the tolerance since the error of two
 * half-periods adds up. Returns the edge (n if none) and the steps in *k. */
static uint32_t link(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t cur, uint32_t est, int dir, int may_skip,
                     const uint8_t *skip, int pulses, uint32_t *k)
{
	uint32_t tol = pulses ? tol_pulse(est) : tol_of(est), j = n;

	*k = 1;
	if (dir > 0) {
		if (cur + 1u >= n) return n;
		j = nearest(d, t, n, cur + 1u, t[cur] + est, tol, 1, skip);
		if (j == n && may_skip) {
			j = nearest(d, t, n, cur + 1u, t[cur] + 2u * est, 2u * tol, 1, skip);
			*k = 2;
		}
	} else {
		if (cur == 0) return n;
		if (t[cur] >= est + tol) j = nearest(d, t, n, cur - 1u, t[cur] - est, tol, -1, skip);
		if (j == n && may_skip && t[cur] >= 2u * (est + tol)) {
			j = nearest(d, t, n, cur - 1u, t[cur] - 2u * est, 2u * tol, -1, skip);
			*k = 2;
		}
	}
	return j;
}

/* Follows the chain of half-period `step` through edge `seed`, forward and
 * back, never through edges marked in `skip` (if given). pulses: a chain of
 * skips alone is no chain (it would be a voice at twice the step: for short
 * pulses, a voice an octave below, whose rises are no lost ones). */
static void follow(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t seed, uint32_t step, const uint8_t *skip,
                   int pulses, chain_t *c)
{
	uint32_t cur = seed, est = step, k, j, singles = 0;
	int skipped = 0;

	c->nodes = 0;
	c->steps = 0;
	d->chain[c->nodes++] = (uint16_t)seed;
	/* forward, re-estimating the step from the chain so far */
	while ((j = link(d, t, n, cur, est, 1, !skipped, skip, pulses, &k)) != n) {
		skipped = k == 2;
		singles += k == 1;
		c->steps += k;
		cur = j;
		d->chain[c->nodes++] = (uint16_t)j;
		est = (t[cur] - t[seed] + c->steps / 2u) / c->steps;
	}
	c->last = cur;
	/* backward, at the step the forward chain measured */
	cur = seed;
	skipped = 0;
	while ((j = link(d, t, n, cur, est, -1, !skipped, skip, pulses, &k)) != n) {
		skipped = k == 2;
		singles += k == 1;
		c->steps += k;
		cur = j;
		d->chain[c->nodes++] = (uint16_t)j;
	}
	c->first = cur;
	if (pulses && !singles) {   /* skips alone: only the seed */
		c->nodes = 1;
		c->steps = 0;
		c->first = c->last = seed;
	}
}

int snd_flips_multiple(uint32_t p_q8, uint32_t q_q8)
{
	uint32_t p = p_q8 >> 8, q = q_q8 >> 8, tol = tol_of(q), k;

	if (!q || p < 2u * q - tol) return 0;
	k = (p + q / 2u) / q;   /* the nearest whole multiple */
	return k <= MULT_MAX && (p > k * q ? p - k * q : k * q - p) <= tol;
}

#define BASES 8u

/* Do the two halves of chain c (step hp_q8) measure different periods? */
static int drifts(snd_decoder *d, const uint32_t *t, const chain_t *c, uint32_t hp_q8)
{
	uint32_t ext = t[c->last] - t[c->first], mid = t[c->first] + ext / 2u, best = c->first, best_d = ext, i, k, a, b;

	if (c->steps < 4u) return 0;   /* too short to split */
	d->work += c->nodes;
	for (i = 0; i < c->nodes; i++) {
		uint32_t x = d->chain[i], dd = t[x] > mid ? t[x] - mid : mid - t[x];
		if (dd < best_d) {
			best_d = dd;
			best = x;
		}
	}
	k = ((t[best] - t[c->first]) * c->steps + ext / 2u) / ext;   /* steps before the split */
	if (k == 0 || k >= c->steps) return 0;
	a = ((t[best] - t[c->first]) << 8) / k;
	b = ((t[c->last] - t[best]) << 8) / (c->steps - k);
	return (a > b ? a - b : b - a) > (hp_q8 >> 7) + (2u << 8);
}

uint32_t snd_flips_decode(snd_decoder *d, uint32_t *t, uint32_t n, int pulses, uint32_t *hp_q8, uint8_t *drift)
{
	uint32_t hmin = pulses ? 2u * HMIN : HMIN;
	uint32_t cand[SEEDS * AHEAD];
	uint16_t from[SEEDS * AHEAD];
	uint32_t base[BASES];   /* periods (Q8) of voices found and of chains through 30% of the window */
	uint32_t v, explained = 0, nb = 0;

	for (v = 0; v < SND_TONAL; v++)
		hp_q8[v] = 0;
	if (drift) *drift = 0;
	for (v = 0; v < SND_TONAL && n >= 3u && !spent(d); v++) {
		uint32_t nc = 0, s, i, j, last_seed = n, span = t[n - 1u] - t[0];
		chain_t c;
		int found = 0;

		for (s = 0; s < SEEDS; s++) {
			uint32_t seed = s * n / SEEDS;
			if (seed == last_seed) continue;
			last_seed = seed;
			for (j = seed + 1u; j < n && j <= seed + AHEAD; j++) {
				uint32_t gap = t[j] - t[seed];
				if (gap > HMAX) break;
				if (gap < hmin) continue;
				/* insertion into ascending order */
				for (i = nc; i > 0 && cand[i - 1u] > gap; i--) {
					cand[i] = cand[i - 1u];
					from[i] = from[i - 1u];
				}
				cand[i] = gap;
				from[i] = (uint16_t)seed;
				d->work += nc - i + 1u;
				nc++;
			}
		}
		for (i = 0; i < nc && !found; i++) {
			uint32_t ext, per, b;
			int mult = 0;
			if (spent(d)) break;
			if (i && cand[i] == cand[i - 1u] && from[i] == from[i - 1u]) continue;
			follow(d, t, n, from[i], cand[i], 0, pulses, &c);
			if (c.nodes < MIN_NODES) continue;
			ext = t[c.last] - t[c.first];
			per = (ext << 8) / c.steps;
			/* a whole multiple of a voice's period is that voice's sub-chain or a chord's
			 * joint period, not a voice: the shortest chaining period wins */
			for (b = 0; b < nb && !mult; b++)
				mult = snd_flips_multiple(per, base[b]);
			if (mult) continue;
			if (ext * 10u >= span * 7u) found = 1;
			else if (ext * 10u >= span * 3u && nb < BASES) base[nb++] = per;
		}
		if (!found) break;
		hp_q8[v] = ((t[c.last] - t[c.first]) << 8) / c.steps;
		if (drift && drifts(d, t, &c, hp_q8[v])) *drift = 1;
		if (nb < BASES && !pulses) base[nb++] = hp_q8[v];
		/* the voice's edges leave the pool: its chain, and the stretches of it the
		 * chain did not reach (past two missing edges in a row) */
		for (i = 0; i < n; i++)
			d->used[i] = 0;
		for (i = 0; i < c.nodes; i++)
			d->used[d->chain[i]] = 1;
		explained += c.nodes;
		for (i = 0; i < n && !spent(d); i++) {
			if (d->used[i]) continue;
			follow(d, t, n, i, hp_q8[v] >> 8, d->used, pulses, &c);
			if (c.nodes < 3u) continue;
			for (j = 0; j < c.nodes; j++)
				d->used[d->chain[j]] = 1;
			explained += c.nodes;
		}
		for (i = 0, j = 0; i < n; i++)
			if (!d->used[i]) t[j++] = t[i];
		d->work += 2u * n;   /* the passes over the pool */
		n = j;
	}
	return explained;
}

uint32_t snd_flips_cover(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t m, uint32_t hp_q8, int pulses,
                         uint32_t *extent)
{
	uint32_t hp = (hp_q8 + 128u) >> 8, i, j, k = 0, lo = 1, hi = 0, lo_t = 0xffffffffu, hi_t = 0;
	chain_t c;

	*extent = 0;
	if (hp < HMIN || !m || spent(d)) return 0;
	d->work += m;
	for (i = 0; i < m && !spent(d); i++) {
		uint32_t a, b;
		if (d->used[i]) continue;
		follow(d, t, n, i, hp, d->used, pulses, &c);
		if (c.nodes < 3u) continue;
		for (j = 0; j < c.nodes; j++) {
			uint32_t x = d->chain[j];
			if (x < m && !d->used[x]) {
				d->used[x] = 1;
				k++;
			}
		}
		/* the time the chains span inside t[0..m) */
		a = c.first;
		b = c.last < m ? c.last : m - 1u;
		if (lo > hi) {
			lo = a;
			hi = b;
		}
		if (a < lo) lo = a;
		if (b > hi) hi = b;
		if (pulses) {
			/* short pulses: the time the pulse train sounds in the frame (t
			 * from 0 to ZX_FRAME_TSTATES): from its first pulse there (from the
			 * frame's start if that comes within a period: the one before fell
			 * in the frame before) to a period after its last, or the frame's
			 * end if it runs on */
			uint32_t x = t[c.first] < hp ? 0 : t[c.first];
			uint32_t y = c.last >= m || t[c.last] + hp > ZX_FRAME_TSTATES ? ZX_FRAME_TSTATES : t[c.last] + hp;
			if (x < lo_t) lo_t = x;
			if (y > hi_t) hi_t = y;
		}
	}
	if (pulses) {
		if (hi_t > lo_t) *extent = hi_t - lo_t;
	} else if (hi > lo && lo < m) {
		*extent = t[hi] - t[lo];
	}
	return k;
}
