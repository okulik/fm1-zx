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
 * search repeats on the rest.
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

static uint32_t tol_of(uint32_t step)
{
	uint32_t t = step >> 8;
	return t > TOL_MIN ? t : TOL_MIN;
}

/* Index of the edge nearest to `target` within `tol`, searching forward from
 * index `from` (dir > 0) or backward from it (dir < 0), passing over edges
 * marked in `skip` (if given); n if none. */
static uint32_t nearest(const uint32_t *t, uint32_t n, uint32_t from, uint32_t target, uint32_t tol, int dir,
                        const uint8_t *skip)
{
	uint32_t best = n, best_d = tol + 1u, j = from;

	if (dir > 0) {
		for (; j < n && t[j] <= target + tol; j++) {
			uint32_t dd = t[j] > target ? t[j] - target : target - t[j];
			if (dd < best_d && !(skip && skip[j])) {
				best_d = dd;
				best = j;
			}
		}
	} else {
		for (;;) {
			uint32_t dd;
			if (t[j] + tol < target) break;
			dd = t[j] > target ? t[j] - target : target - t[j];
			if (dd < best_d && !(skip && skip[j])) {
				best_d = dd;
				best = j;
			}
			if (j == 0) break;
			j--;
		}
	}
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
static uint32_t link(const uint32_t *t, uint32_t n, uint32_t cur, uint32_t est, int dir, int may_skip,
                     const uint8_t *skip, uint32_t *k)
{
	uint32_t tol = tol_of(est), j = n;

	*k = 1;
	if (dir > 0) {
		if (cur + 1u >= n) return n;
		j = nearest(t, n, cur + 1u, t[cur] + est, tol, 1, skip);
		if (j == n && may_skip) {
			j = nearest(t, n, cur + 1u, t[cur] + 2u * est, 2u * tol, 1, skip);
			*k = 2;
		}
	} else {
		if (cur == 0) return n;
		if (t[cur] >= est + tol) j = nearest(t, n, cur - 1u, t[cur] - est, tol, -1, skip);
		if (j == n && may_skip && t[cur] >= 2u * (est + tol)) {
			j = nearest(t, n, cur - 1u, t[cur] - 2u * est, 2u * tol, -1, skip);
			*k = 2;
		}
	}
	return j;
}

/* Follows the chain of half-period `step` through edge `seed`, forward and
 * back, never through edges marked in `skip` (if given). */
static void follow(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t seed, uint32_t step, const uint8_t *skip,
                   chain_t *c)
{
	uint32_t cur = seed, est = step, k, j;
	int skipped = 0;

	c->nodes = 0;
	c->steps = 0;
	d->chain[c->nodes++] = (uint16_t)seed;
	/* forward, re-estimating the step from the chain so far */
	while ((j = link(t, n, cur, est, 1, !skipped, skip, &k)) != n) {
		skipped = k == 2;
		c->steps += k;
		cur = j;
		d->chain[c->nodes++] = (uint16_t)j;
		est = (t[cur] - t[seed] + c->steps / 2u) / c->steps;
	}
	c->last = cur;
	/* backward, at the step the forward chain measured */
	cur = seed;
	skipped = 0;
	while ((j = link(t, n, cur, est, -1, !skipped, skip, &k)) != n) {
		skipped = k == 2;
		c->steps += k;
		cur = j;
		d->chain[c->nodes++] = (uint16_t)j;
	}
	c->first = cur;
}

int snd_flips_multiple(uint32_t p_q8, uint32_t q_q8)
{
	uint32_t p = p_q8 >> 8, q = q_q8 >> 8, tol = tol_of(q), k;

	if (!q || p < 2u * q - tol) return 0;
	k = (p + q / 2u) / q;   /* the nearest whole multiple */
	return k <= MULT_MAX && (p > k * q ? p - k * q : k * q - p) <= tol;
}

#define BASES 8u

uint32_t snd_flips_decode(snd_decoder *d, uint32_t *t, uint32_t n, uint32_t *hp_q8)
{
	uint32_t cand[SEEDS * AHEAD];
	uint16_t from[SEEDS * AHEAD];
	uint32_t base[BASES];   /* periods (Q8) of voices found and of chains through 30% of the window */
	uint32_t v, explained = 0, nb = 0;

	for (v = 0; v < SND_TONAL; v++)
		hp_q8[v] = 0;
	for (v = 0; v < SND_TONAL && n >= 3u; v++) {
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
				if (gap < HMIN) continue;
				/* insertion into ascending order */
				for (i = nc; i > 0 && cand[i - 1u] > gap; i--) {
					cand[i] = cand[i - 1u];
					from[i] = from[i - 1u];
				}
				cand[i] = gap;
				from[i] = (uint16_t)seed;
				nc++;
			}
		}
		for (i = 0; i < nc && !found; i++) {
			uint32_t ext, per, b;
			int mult = 0;
			if (i && cand[i] == cand[i - 1u] && from[i] == from[i - 1u]) continue;
			follow(d, t, n, from[i], cand[i], 0, &c);
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
		if (nb < BASES) base[nb++] = hp_q8[v];
		/* the voice's edges leave the pool: its chain, and the stretches of it the
		 * chain did not reach (past two missing edges in a row) */
		for (i = 0; i < n; i++)
			d->used[i] = 0;
		for (i = 0; i < c.nodes; i++)
			d->used[d->chain[i]] = 1;
		explained += c.nodes;
		for (i = 0; i < n; i++) {
			if (d->used[i]) continue;
			follow(d, t, n, i, hp_q8[v] >> 8, d->used, &c);
			if (c.nodes < 3u) continue;
			for (j = 0; j < c.nodes; j++)
				d->used[d->chain[j]] = 1;
			explained += c.nodes;
		}
		for (i = 0, j = 0; i < n; i++)
			if (!d->used[i]) t[j++] = t[i];
		n = j;
	}
	return explained;
}

uint32_t snd_flips_cover(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t m, uint32_t hp_q8, uint32_t *extent)
{
	uint32_t hp = (hp_q8 + 128u) >> 8, i, j, k = 0, lo = 1, hi = 0;
	chain_t c;

	*extent = 0;
	if (hp < HMIN || !m) return 0;
	for (i = 0; i < m; i++) {
		uint32_t a, b;
		if (d->used[i]) continue;
		follow(d, t, n, i, hp, d->used, &c);
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
	}
	if (hi > lo && lo < m) *extent = t[hi] - t[lo];
	return k;
}
