/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The time-slot decoder: engines that run one fixed loop of P T-states with
 * an OUT per channel, so each channel holds the speaker for its own slot and
 * the speaker's average over exactly one loop is sum(w_c * s_c), w_c being
 * slot_c / P and s_c the channel's state (0 or 1).
 *
 * The loop: a frame has a carrier when at least 64 rising edges, and half of
 * them, follow the one before by the same interval (within 4%) in 40-400 T;
 * the mean of those intervals is P. A carrier frame at a length within 3% of
 * the known P refines it; a different length must show in two carrier frames
 * in a row to replace it (everything learned is then dropped, the music
 * context with it). Between
 * carriers (rests, channels in unison, drums) the loop is assumed to run on,
 * but it is forgotten after 250 frames (5 s) without a carrier, or 25 frames
 * in a row that have sound but no carrier (other sound: the longest stretch
 * of channels in unison in the pack's music is 14 frames).
 *
 * The box sampler runs through every frame while a loop is known, carrying
 * its state from frame to frame: grid points every P/2 T-states; the
 * speaker-high time up to each; a box level (0-256) per grid point, the high
 * time over the last two hops (one loop). Box levels that hold for three grid
 * points in a row are plateaus, and go into a histogram of levels (halved
 * when it holds 65,000).
 *
 * The weights: the histogram's peaks are the levels the channel combinations
 * make (a peak needs 20 plateaus and 1/2000 of them all). Lowest first, a
 * level not within 14/256 of a sum of the weights found so far is the next
 * channel's weight (I/O contention lengthens a slot by up to 6 T, so a level
 * spreads; levels under 10/256 or over 230/256 are "all off" and "all on").
 * Learned every 8 frames, and at once when a frame fits less than 90% (a
 * channel heard for the first time) or there are no weights yet. A box level
 * decodes into the combination nearest to it (a table of 257 entries, built
 * when the weights change); within 12/256 it fits. A channel whose weight is
 * kept keeps its rises when the weights change.
 *
 * The channels: each channel's state is debounced over 3 grid points (a box
 * straddling a change reads in between for up to two); a change from 0 to 1
 * is a rise, timed at the first grid point of the new state. A rise that came
 * with every channel's (within two loops) is a full swing: all channels at
 * once, which another routine driving the whole speaker (a drum) makes.
 *
 * A channel's voice at a time: the run of rises around it that lie on one
 * period (each within 3/4 of a loop plus 1/48 of the period of the line
 * through the run's ends: an accumulator engine's rises fall on whole loops
 * and the rise times are only known to a grid point; I/O contention stretches
 * the loop while the screen is drawn), with a mean period of 1,400-116,666 T
 * (2.5 kHz to 30 Hz), up to 5 intervals. Comparing positions over the run,
 * not single intervals, keeps two notes a semitone apart from merging even
 * where a loop is more than a semitone of the period (above about 1 kHz).
 * The period is the run's span over its intervals, so it is measured, never
 * snapped. A run takes 3 intervals, not 2, beside a full swing (two
 * intervals beside a drum's or a pause's swing agree by chance) and above
 * about 350 Hz (see run_of). A channel that stopped has no next rise, or one
 * after a rest that fits nothing: no voice, nothing held.
 *
 * Drums: a full swing that no run of a voice explains is another routine
 * driving the speaker (Colony 8's drums stop the music loop for a third of a
 * frame and play a few long pulses); the channels' runs resume after it, the
 * interval across it fitting nothing.
 *
 * Work: one unit per edge of each pass, per grid point, per histogram bin
 * looked at and per table entry; every pass checks the cap. A frame whose
 * work runs out is not decoded (the sampler restarts at its end). */

#include "sound.h"

#define IV_MIN 40u           /* carrier intervals, T */
#define IV_MAX 400u
#define CARRIER_MIN 64u      /* rising edges at the loop interval in a carrier frame */
#define LOOP_HOLD 250u       /* frames without a carrier before the loop is forgotten (5 s) */
#define BARE_MAX 25u         /* frames in a row with sound but no carrier: other sound, the loop is forgotten */
#define DEBOUNCE 3u          /* grid points a channel state must hold */
#define PLATEAU 4u           /* box levels within this of the one before: a plateau */
#define HIST_MAX 65000u
#define PEAK_MIN 20u         /* plateaus at a level for a peak, and 1/PEAK_SHARE of them all */
#define PEAK_SHARE 2000u
#define FIT_LEARN 90u        /* a frame fitting less: learn the weights again at once */
#define LEVEL_LO 10u         /* box levels below are "all off", above LEVEL_HI "all on" */
#define LEVEL_HI 230u
#define SUM_TOL 14u          /* a level explained by a sum of weights (I/O contention widens a slot by up to 6 T) */
#define FIT_TOL 12u          /* a box level a combination explains */
#define SAME_W 8u            /* a weight kept across a re-learning */
#define LEARN_FRAMES 8u
#define PER_MIN 1400u        /* a voice's period, T: 2.5 kHz */
#define PER_MAX 116666u      /* 30 Hz */
#define RUN_MAX 5u           /* intervals averaged into a period */

static int spent(const snd_decoder *d)
{
	return d->work >= d->work_cap;
}

static uint32_t udiff(uint32_t a, uint32_t b)
{
	return a > b ? a - b : b - a;
}

static void reset_channels(snd_slots *s)
{
	uint32_t c;
	s->cur = 0;
	for (c = 0; c < SND_SLOTS; c++) {
		s->cand[c] = 0;
		s->run[c] = 0;
		s->nrise[c] = 0;
	}
}

void snd_slots_reset(snd_slots *s)
{
	uint32_t i;

	s->loop_q8 = 0;
	s->loop_frame = 0;
	s->other_q8 = 0;
	s->bare = 0;
	s->grid_q8 = 0;
	s->last_t = 0;
	s->cum = 0;
	s->h[0] = s->h[1] = 0;
	s->recip = 0;
	s->running = 0;
	s->level = 0;
	s->primed = 0;
	s->lv[0] = s->lv[1] = 0xffffu;
	for (i = 0; i < SND_LEVEL_BINS; i++)
		s->hist[i] = 0;
	s->hist_total = 0;
	s->learnt_frame = 0;
	s->k = 0;
	for (i = 0; i < SND_SLOTS; i++)
		s->w[i] = 0;
	for (i = 0; i < 257u; i++)
		s->lut[i] = 0;
	reset_channels(s);
	s->good = s->samples = 0;
	for (i = 0; i < SND_TONAL; i++)
		s->sref_q8[i] = s->pend_q8[i] = 0;
	s->sref_frame = s->schange_frame = s->pend_frame = 0;
	s->pend_n = 0;
	s->have_sref = s->have_schange = 0;
	s->sbroken = 0;
}

/* ---- the loop carrier --------------------------------------------------- */

/* The mean rising-edge interval within 4% of `p_q8` (T x 256) in the frame's
 * speaker edges, and in *count how many there are; *rising counts the rising
 * edges. */
static uint32_t near_mean(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t ear, uint32_t p_q8,
                          uint32_t *count, uint32_t *rising)
{
	uint32_t i, prev = 0, have = 0, sum = 0, cnt = 0, up = 0, tol = p_q8 / 25u;

	for (i = 0; i < n; i++) {
		uint8_t lv = (e[i].level >> 1) & 1u;
		if (lv == ear) continue;
		ear = lv;
		if (!lv) continue;
		up++;
		if (have) {
			uint32_t iv = e[i].tstate - prev;
			if (iv >= IV_MIN && iv < IV_MAX && udiff(iv << 8, p_q8) <= tol) {
				sum += iv;
				cnt++;
			}
		}
		prev = e[i].tstate;
		have = 1;
	}
	d->work += n;
	*count = cnt;
	*rising = up;
	return cnt ? (sum << 8) / cnt : 0;
}

/* The frame's carrier interval (T x 256), 0 if it has none. */
static uint32_t carrier(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level)
{
	snd_slots *s = &d->sl;
	uint8_t ear = (start_level >> 1) & 1u;
	uint32_t m, cnt, up, i, prev = 0, have = 0, best = 0;
	uint32_t *bins = d->pool;   /* scratch: 4 T bins of the intervals */
	const uint32_t nb = (IV_MAX - IV_MIN) / 4u;

	if (spent(d)) return 0;
	if (s->loop_q8) {
		m = near_mean(d, e, n, ear, s->loop_q8, &cnt, &up);
		if (cnt >= CARRIER_MIN && 2u * cnt >= up) return m;
	}
	/* the most common interval */
	for (i = 0; i < nb; i++)
		bins[i] = 0;
	for (i = 0; i < n; i++) {
		uint8_t lv = (e[i].level >> 1) & 1u;
		if (lv == ear) continue;
		ear = lv;
		if (!lv) continue;
		if (have) {
			uint32_t iv = e[i].tstate - prev;
			if (iv >= IV_MIN && iv < IV_MAX) bins[(iv - IV_MIN) / 4u]++;
		}
		prev = e[i].tstate;
		have = 1;
	}
	d->work += n + nb;
	for (i = 1; i < nb; i++)
		if (bins[i] > bins[best]) best = i;
	if (bins[best] < CARRIER_MIN / 2u || spent(d)) return 0;
	m = near_mean(d, e, n, (start_level >> 1) & 1u, (IV_MIN + best * 4u + 2u) << 8, &cnt, &up);
	if (cnt < CARRIER_MIN || 2u * cnt < up) return 0;
	return m;
}

/* ---- the weights --------------------------------------------------------- */

static uint32_t combo(const snd_slots *s, uint32_t m)
{
	uint32_t c, sum = 0;
	for (c = 0; c < s->k; c++)
		if (m >> c & 1u) sum += s->w[c];
	return sum;
}

static void build_lut(snd_decoder *d)
{
	snd_slots *s = &d->sl;
	uint32_t b, m, combos = 1u << s->k;

	for (b = 0; b <= 256u; b++) {
		uint32_t best = 0, best_d = 0xffffffffu;
		for (m = 0; m < combos; m++) {
			uint32_t dd = udiff(b, combo(s, m));
			if (dd < best_d) {
				best_d = dd;
				best = m;
			}
		}
		s->lut[b] = (uint8_t)(best | (best_d <= FIT_TOL ? 0x80u : 0u));
	}
	d->work += 257u * combos;
}

/* Learns the weights from the plateau histogram; keeps the rises of the
 * channels whose weight stays. */
static void learn(snd_decoder *d)
{
	snd_slots *s = &d->sl;
	uint32_t *h = d->pool;   /* scratch: the histogram, peaks cleared as found */
	uint32_t lev[8], nl = 0, i, j, w[SND_SLOTS], nw = 0;
	int changed;

	for (i = 0; i < SND_LEVEL_BINS; i++)
		h[i] = s->hist[i];
	while (nl < 8u && !spent(d)) {
		uint32_t best = 0, sum = 0, wsum = 0, lo, hi;
		for (i = 1; i < SND_LEVEL_BINS; i++)
			if (h[i] > h[best]) best = i;
		d->work += SND_LEVEL_BINS;
		if (h[best] < PEAK_MIN || h[best] * PEAK_SHARE < s->hist_total) break;
		lo = best >= 2u ? best - 2u : 0;
		hi = best + 2u < SND_LEVEL_BINS ? best + 2u : SND_LEVEL_BINS - 1u;
		for (i = lo; i <= hi; i++) {
			sum += s->hist[i];
			wsum += s->hist[i] * (2u * i + 1u);   /* bin i holds levels 2i and 2i + 1 */
		}
		lev[nl++] = (wsum + sum / 2u) / sum;
		lo = best >= 4u ? best - 4u : 0;
		hi = best + 4u < SND_LEVEL_BINS ? best + 4u : SND_LEVEL_BINS - 1u;
		for (i = lo; i <= hi; i++)
			h[i] = 0;
	}
	/* ascending */
	for (i = 1; i < nl; i++)
		for (j = i; j > 0 && lev[j - 1u] > lev[j]; j--) {
			uint32_t x = lev[j];
			lev[j] = lev[j - 1u];
			lev[j - 1u] = x;
		}
	for (i = 0; i < nl && nw < SND_SLOTS; i++) {
		uint32_t m, explained = 0;
		if (lev[i] < LEVEL_LO || lev[i] > LEVEL_HI) continue;
		for (m = 0; m < (1u << nw) && !explained; m++) {
			uint32_t c, sum = 0;
			for (c = 0; c < nw; c++)
				if (m >> c & 1u) sum += w[c];
			if (udiff(sum, lev[i]) <= SUM_TOL) explained = 1;
		}
		if (!explained) w[nw++] = lev[i];
	}
	changed = nw != s->k;
	for (i = 0; i < nw && !changed; i++)
		if (udiff(w[i], s->w[i]) > SAME_W) changed = 1;
	if (!changed) {
		/* the same channels: follow small drifts */
		for (i = 0; i < nw; i++)
			s->w[i] = (uint8_t)w[i];
		if (nw) build_lut(d);
		return;
	}
	{
		/* new channels: carry over the rises of those whose weight stays,
		 * moving each kept ring to its new channel by swaps, in place */
		uint32_t src[SND_SLOTS], kept_n[SND_SLOTS], x;
		uint8_t cur = 0;
		for (i = 0; i < SND_SLOTS; i++) {
			src[i] = SND_SLOTS;   /* none */
			if (i >= nw) continue;
			for (j = 0; j < s->k; j++)
				if (udiff(w[i], s->w[j]) <= SAME_W) break;
			if (j < s->k) src[i] = j;
		}
		for (i = 0; i < nw; i++) {
			uint32_t from = src[i], r;
			if (from == SND_SLOTS || from == i) continue;
			for (r = 0; r < SND_RISES; r++) {
				uint32_t t = s->rise[i][r];
				uint8_t f = s->swing[i][r];
				s->rise[i][r] = s->rise[from][r];
				s->swing[i][r] = s->swing[from][r];
				s->rise[from][r] = t;
				s->swing[from][r] = f;
			}
			x = s->nrise[i];
			s->nrise[i] = s->nrise[from];
			s->nrise[from] = x;
			if ((s->cur >> i ^ s->cur >> from) & 1u) s->cur ^= (uint8_t)((1u << i) | (1u << from));
			/* whatever was at i is now at `from` */
			for (j = i + 1u; j < nw; j++)
				if (src[j] == i) src[j] = from;
			d->work += SND_RISES;
		}
		for (i = 0; i < SND_SLOTS; i++) {
			kept_n[i] = src[i] == SND_SLOTS ? 0 : s->nrise[i];
			if (kept_n[i] && (s->cur >> i & 1u)) cur |= (uint8_t)(1u << i);
		}
		reset_channels(s);
		for (i = 0; i < nw; i++) {
			s->w[i] = (uint8_t)w[i];
			s->nrise[i] = kept_n[i];
			s->cand[i] = (uint8_t)(cur >> i & 1u);
		}
		s->cur = cur;
		s->k = (uint8_t)nw;
		if (nw) build_lut(d);
	}
}

/* ---- the sampler ------------------------------------------------------- */

static void record_rise(snd_slots *s, uint32_t c, uint32_t at, uint32_t hop)
{
	uint32_t i = s->nrise[c] % SND_RISES, o, all = 1;

	s->rise[c][i] = at;
	s->swing[c][i] = 0;
	s->nrise[c]++;
	if (s->k < 2u) return;
	/* a full swing: every channel rose within four hops (two loops: the box
	 * levels pass through other combinations on the way, so each channel's
	 * debounce ends a grid point or two apart) */
	for (o = 0; o < s->k && all; o++) {
		uint32_t j;
		if (o == c) continue;
		if (!s->nrise[o]) {
			all = 0;
			break;
		}
		j = (s->nrise[o] - 1u) % SND_RISES;
		if (udiff(s->rise[o][j], at) > 4u * hop) all = 0;
	}
	if (!all) return;
	for (o = 0; o < s->k; o++)
		s->swing[o][(s->nrise[o] - 1u) % SND_RISES] = 1;
}

/* One box level (0-256) whose box is centred on absolute time `at`. */
static void sample(snd_slots *s, uint32_t b, uint32_t at, uint32_t hop)
{
	uint32_t c, x;
	uint8_t state;

	if (s->lv[1] != 0xffffu && s->lv[0] != 0xffffu && udiff(b, s->lv[1]) < PLATEAU && udiff(s->lv[1], s->lv[0]) < PLATEAU) {
		s->hist[b >= 256u ? SND_LEVEL_BINS - 1u : b >> 1]++;
		if (++s->hist_total >= HIST_MAX) {
			s->hist_total = 0;
			for (c = 0; c < SND_LEVEL_BINS; c++) {
				s->hist[c] >>= 1;
				s->hist_total += s->hist[c];
			}
		}
	}
	s->lv[0] = s->lv[1];
	s->lv[1] = (uint16_t)b;
	if (!s->k) return;
	x = s->lut[b];
	state = (uint8_t)(x & 0x7fu);
	s->samples++;
	if (x & 0x80u) s->good++;
	for (c = 0; c < s->k; c++) {
		uint8_t bit = (uint8_t)(state >> c & 1u);
		if (bit == s->cand[c]) {
			if (s->run[c] < 255u) s->run[c]++;
		} else {
			s->cand[c] = bit;
			s->run[c] = 1;
		}
		if (s->cand[c] != (s->cur >> c & 1u) && s->run[c] >= DEBOUNCE) {
			s->cur ^= (uint8_t)(1u << c);
			if (bit) record_rise(s, c, at - (uint32_t)(s->run[c] - 1u) * hop, hop);
		}
	}
}

/* Evaluates the grid points up to (and including) relative time limit_q8 (T x 256). */
static void grid_to(snd_decoder *d, int32_t limit_q8, uint32_t base)
{
	snd_slots *s = &d->sl;
	int32_t hop_q8 = (int32_t)(s->loop_q8 >> 1);
	uint32_t hop = s->loop_q8 >> 9;

	while (s->grid_q8 <= limit_q8 && !spent(d)) {
		int32_t g = s->grid_q8 >> 8;
		uint32_t hg = s->cum + (s->level ? (uint32_t)(g - s->last_t) : 0u);
		if (s->primed == 2u) {
			uint32_t b = ((hg - s->h[0]) * s->recip) >> 16;
			if (b > 256u) b = 256u;
			sample(s, b, base + (uint32_t)g - hop, hop);
			s->h[0] = s->h[1];
			s->h[1] = hg;
		} else {
			s->h[s->primed++] = hg;
		}
		s->grid_q8 += hop_q8;
		d->work++;
	}
}

void snd_slots_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_slot *fs)
{
	snd_slots *s = &d->sl;
	uint32_t t = d->frames, base = t * ZX_FRAME_TSTATES, m, i, sounding = 0;
	uint8_t ear = (start_level >> 1) & 1u, end_ear = ear;
	const int32_t end = (int32_t)ZX_FRAME_TSTATES;

	fs->carrier = 0;
	fs->tdm = 0;
	fs->fit = 0;
	s->good = s->samples = 0;
	for (i = 0; i < n; i++) {
		uint8_t lv = (e[i].level >> 1) & 1u;
		if (lv != end_ear) sounding = 1;
		end_ear = lv;
	}
	d->work += n;

	m = carrier(d, e, n, start_level);
	if (m) {
		if (!s->loop_q8) {
			snd_slots_reset(s);
			s->loop_q8 = m;
		} else if (udiff(m, s->loop_q8) <= s->loop_q8 / 32u) {
			s->loop_q8 = (s->loop_q8 * 7u + m) / 8u;
			s->other_q8 = 0;
		} else if (s->other_q8 && udiff(m, s->other_q8) <= s->other_q8 / 32u) {
			snd_slots_reset(s);   /* another loop: learn it afresh */
			s->loop_q8 = m;
		} else {
			s->other_q8 = m;
		}
		if (s->loop_q8 == m || udiff(m, s->loop_q8) <= s->loop_q8 / 32u) {
			s->loop_frame = t;
			fs->carrier = 1;   /* the loop in use: a carrier at another length is other sound */
		}
	}
	if (m) s->bare = 0;
	else if (sounding) s->bare++;
	if (s->loop_q8 && (t - s->loop_frame > LOOP_HOLD || s->bare > BARE_MAX)) snd_slots_reset(s);
	if (!s->loop_q8) return;

	/* the box sampler, from where the last frame left it */
	{
		uint32_t box = ((s->loop_q8 >> 1) * 2u + 128u) >> 8;
		s->recip = (256u << 16) / box;
	}
	if (!s->running) {
		s->running = 1;
		s->level = ear;
		s->last_t = 0;
		s->cum = 0;
		s->grid_q8 = 0;
		s->primed = 0;
	}
	for (i = 0; i < n && !spent(d); i++) {
		uint8_t lv = (e[i].level >> 1) & 1u;
		int32_t at = (int32_t)e[i].tstate;
		if (lv == s->level) continue;
		grid_to(d, at * 256, base);
		if (s->level) s->cum += (uint32_t)(at - s->last_t);
		s->last_t = at;
		s->level = lv;
		d->work++;
	}
	grid_to(d, end * 256 - 1, base);
	if (spent(d)) {
		/* out of work: this frame is not decoded; the sampler restarts at its end */
		s->level = end_ear;
		s->last_t = end;
		s->cum = 0;
		s->primed = 0;
		s->grid_q8 = end * 256;
		s->good = s->samples = 0;
	}
	/* to the next frame's time base; an edge long ago moves up (the level held since) */
	s->grid_q8 -= end * 256;
	if (s->last_t < 0) {
		if (s->level) s->cum += (uint32_t)(0 - s->last_t);
		s->last_t = 0;
	}
	s->last_t -= end;

	if (!spent(d) && s->hist_total >= 200u &&
	    (s->k == 0 || t - s->learnt_frame >= LEARN_FRAMES || s->good * 100u < s->samples * FIT_LEARN)) {
		learn(d);
		s->learnt_frame = t;
	}
	if (s->k && s->samples && !spent(d)) {
		fs->tdm = 1;
		fs->fit = (uint8_t)(s->good * 100u / s->samples);
	}
}

/* ---- voices and drums ---------------------------------------------------- */

/* Rise i of channel c, oldest kept first. */
static uint32_t rise_at(const snd_slots *s, uint32_t c, uint32_t i)
{
	uint32_t kept = s->nrise[c] < SND_RISES ? s->nrise[c] : SND_RISES;
	return s->rise[c][(s->nrise[c] - kept + i) % SND_RISES];
}

static uint32_t kept(const snd_slots *s, uint32_t c)
{
	return s->nrise[c] < SND_RISES ? s->nrise[c] : SND_RISES;
}

/* The last rise of channel c at or before `at` (count if none). */
static uint32_t find(snd_decoder *d, uint32_t c, uint32_t at)
{
	const snd_slots *s = &d->sl;
	uint32_t lo = 0, hi = kept(s, c);   /* the answer is lo - 1 */

	while (lo < hi) {
		uint32_t mid = (lo + hi) / 2u;
		d->work++;
		if ((int32_t)(rise_at(s, c, mid) - at) <= 0) lo = mid + 1u;
		else hi = mid;
	}
	return lo ? lo - 1u : kept(s, c);
}

static int swing_at(const snd_slots *s, uint32_t c, uint32_t i)
{
	return s->swing[c][(s->nrise[c] - kept(s, c) + i) % SND_RISES];
}

/* Rises lo..hi lie on one period: each within D of the line through the
 * first and the last, D being 3/4 of a loop (an accumulator engine's rises
 * fall on whole loops, half a loop either side of the true time, and a grid
 * point is a quarter loop) plus 1/48 of the period (I/O contention makes the
 * loop up to 2.6% longer while the screen is drawn), and each interval
 * within 2D of the mean (one stretched by the sequencer's pause between
 * notes moves the line less than D in a short run); the mean period within
 * the voice range. Two notes a semitone apart differ by 5.6% of the period:
 * a run of k periods of one and m of the other bends away from its line by
 * about k * m / (k + m) of that, more than D once it holds enough of both
 * (2 of each at 1 kHz, 3 of each at 2 kHz on a 156 T loop); a run that stays
 * on its line holds mostly one note (see run_of for the shortest runs). */
static int on_line(snd_decoder *d, uint32_t c, uint32_t lo, uint32_t hi)
{
	const snd_slots *s = &d->sl;
	uint32_t r0 = rise_at(s, c, lo), span = rise_at(s, c, hi) - r0, n = hi - lo, p_q8, dev_q8, j, prev = r0;

	if (span < n * PER_MIN || span > n * PER_MAX) return 0;
	p_q8 = (span << 8) / n;
	dev_q8 = (s->loop_q8 * 3u) / 4u + p_q8 / 48u;
	for (j = lo + 1u; j <= hi; j++) {
		uint32_t r = rise_at(s, c, j), at_q8 = (r - r0) << 8, want_q8 = (j - lo) * p_q8;
		if (udiff(at_q8, want_q8) > dev_q8 || udiff((r - prev) << 8, p_q8) > 2u * dev_q8) return 0;
		prev = r;
	}
	d->work += n;
	return 1;
}

/* The run of rises on one period around interval a (from rise a to rise
 * a + 1), grown a rise at a time backward and forward while they stay on the
 * line (on_line), up to RUN_MAX intervals: returns its length (0 if it is no
 * run), puts its span in *sum and in *bare whether every rise of it is a full
 * swing. A run needs 2 intervals, or 3:
 *   - when a rise of it is a full swing (two intervals beside a swing agree by
 *     chance: the rises a drum or the sequencer's pause between notes makes
 *     when it leaves the speaker on);
 *   - when the period is under 64 loops (above about 350 Hz), where a run of
 *     one interval of each of two notes a semitone apart can stay within D of
 *     its line: with 3 intervals such a run is two of one note and one of the
 *     other, its mean within a third of a semitone of the first. */
static uint32_t run_of(snd_decoder *d, uint32_t c, uint32_t a, uint32_t *sum, int *bare)
{
	const snd_slots *s = &d->sl;
	uint32_t n = kept(s, c), lo = a, hi = a + 1u, j, cnt, need = 2u;
	int back = 1, fwd = 1, swingy = 0, all = 1;

	*sum = 0;
	*bare = 0;
	if (!on_line(d, c, lo, hi)) return 0;
	while (hi - lo < RUN_MAX && (back || fwd)) {
		if (back) {
			if (lo >= 1u && on_line(d, c, lo - 1u, hi)) lo--;
			else back = 0;
		}
		if (fwd && hi - lo < RUN_MAX) {
			if (hi + 1u < n && on_line(d, c, lo, hi + 1u)) hi++;
			else fwd = 0;
		}
	}
	for (j = lo; j <= hi; j++) {
		int sw = swing_at(s, c, j);
		swingy |= sw;
		all &= sw;
	}
	cnt = hi - lo;
	*sum = rise_at(s, c, hi) - rise_at(s, c, lo);
	*bare = all;
	if (swingy || *sum < cnt * 64u * (s->loop_q8 >> 8)) need = 3u;
	return cnt >= need ? cnt : 0;
}

/* The period (T x 256) of the run around interval a if it is a voice, else 0.
 * Given keep_q8, a bare run (the whole speaker toggling: a plain square wave,
 * not time slots) is a voice only if it continues one in keep_q8 (channels in
 * unison, in phase), so other sound left to the time-slot decoder plays no
 * notes. */
static uint32_t voice_of(snd_decoder *d, uint32_t c, uint32_t a, const uint32_t *keep_q8)
{
	uint32_t sum, cnt, p, k;
	int bare;

	cnt = run_of(d, c, a, &sum, &bare);
	if (!cnt) return 0;
	p = (sum << 8) / cnt;
	if (!bare || !keep_q8) return p;
	for (k = 0; k < SND_TONAL; k++)
		if (keep_q8[k] && udiff(p, keep_q8[k]) <= keep_q8[k] / 32u) return p;
	return 0;
}

uint32_t snd_slots_voice(snd_decoder *d, uint32_t c, uint32_t at, const uint32_t *keep_q8)
{
	const snd_slots *s = &d->sl;
	uint32_t n, a;

	if (c >= s->k || spent(d)) return 0;
	n = kept(s, c);
	if (n < 3u) return 0;
	a = find(d, c, at);
	if (a >= n || a + 1u >= n) return 0;
	return voice_of(d, c, a, keep_q8);
}

uint32_t snd_slots_explained(snd_decoder *d, uint32_t from, uint32_t to, const uint32_t *keep_q8, uint32_t *rises,
                             uint32_t *swings)
{
	const snd_slots *s = &d->sl;
	uint32_t c, explained = 0;

	*rises = 0;
	*swings = 0;
	for (c = 0; c < s->k; c++) {
		uint32_t n = kept(s, c), i = find(d, c, from);
		i = i >= n ? 0 : i + 1u;   /* the first rise after `from` */
		for (; i < n && (int32_t)(rise_at(s, c, i) - to) < 0 && !spent(d); i++) {
			(*rises)++;
			*swings += (uint32_t)swing_at(s, c, i);
			d->work++;
			if ((i >= 1u && voice_of(d, c, i - 1u, keep_q8)) || (i + 1u < n && voice_of(d, c, i, keep_q8)))
				explained++;
		}
	}
	return explained;
}

uint8_t snd_slots_level(const snd_slots *s, uint32_t c)
{
	/* louder for a wider slot: 96-255 */
	return (uint8_t)(96u + (s->w[c] * 159u) / 256u);
}

uint32_t snd_slots_drums(snd_decoder *d, uint32_t from, uint32_t to)
{
	const snd_slots *s = &d->sl;
	uint32_t n, i, drums = 0, sum;
	int bare;

	if (s->k < 2u || spent(d)) return 0;
	n = kept(s, 0);
	i = find(d, 0, from);
	i = i >= n ? 0 : i + 1u;   /* the first rise after `from` */
	for (; i < n && (int32_t)(rise_at(s, 0, i) - to) < 0 && !spent(d); i++) {
		d->work++;
		if (!swing_at(s, 0, i)) continue;
		/* explained if it is part of a run (a voice's, or a plain square wave's) */
		if (i >= 1u && run_of(d, 0, i - 1u, &sum, &bare)) continue;
		if (i + 1u < n && run_of(d, 0, i, &sum, &bare)) continue;
		drums++;
	}
	return drums;
}
