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
 * in a row to replace it (everything learned is then dropped). Between
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
 * A channel's voice at a time: the rises just before and after it, whose
 * interval agrees with one of the intervals beside it within 1/32 plus one
 * loop (the rise times are only known to a grid point, and an accumulator
 * engine's rises fall on whole loops; 1/32 is half a semitone, so a glide
 * breaks into its steps), within 1,400-116,666 T (2.5 kHz to 30 Hz). The
 * period is the mean of up to 5 agreeing intervals around it, so it is
 * measured, never snapped. A run of two intervals touching a full swing is
 * not enough (two intervals beside a drum's or a pause's swing agree by
 * chance): it takes three. A channel that stopped has no next rise, or one
 * after a rest that agrees with nothing: no voice, nothing held.
 *
 * Drums: a full swing that no run of a voice explains is another routine
 * driving the speaker (Colony 8's drums stop the music loop for a third of a
 * frame and play a few long pulses); the channels' runs resume after it, the
 * interval across it agreeing with nothing.
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
		/* new channels: carry over the rises of those whose weight stays */
		uint32_t (*keep)[SND_RISES] = (uint32_t (*)[SND_RISES])(void *)(d->pool + SND_LEVEL_BINS);
		uint32_t keep_n[SND_SLOTS];
		uint8_t keep_cur = 0;
		for (i = 0; i < nw; i++) {
			keep_n[i] = 0;
			for (j = 0; j < s->k; j++)
				if (udiff(w[i], s->w[j]) <= SAME_W) break;
			if (j < s->k && i * SND_RISES + SND_RISES <= 2u * SND_EDGES_MAX - SND_LEVEL_BINS) {
				uint32_t r;
				for (r = 0; r < SND_RISES; r++)
					keep[i][r] = s->rise[j][r];
				for (r = 0; r < SND_RISES; r++)
					d->used[i * SND_RISES + r] = s->swing[j][r];
				keep_n[i] = s->nrise[j];
				if (s->cur >> j & 1u) keep_cur |= (uint8_t)(1u << i);
				d->work += SND_RISES;
			}
		}
		reset_channels(s);
		for (i = 0; i < nw; i++) {
			uint32_t r;
			s->w[i] = (uint8_t)w[i];
			if (!keep_n[i]) continue;
			for (r = 0; r < SND_RISES; r++) {
				s->rise[i][r] = keep[i][r];
				s->swing[i][r] = d->used[i * SND_RISES + r];
			}
			s->nrise[i] = keep_n[i];
		}
		s->cur = keep_cur;
		for (i = 0; i < nw; i++)
			s->cand[i] = (uint8_t)(keep_cur >> i & 1u);
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
		fs->carrier = 1;
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
		if (s->loop_q8 == m || udiff(m, s->loop_q8) <= s->loop_q8 / 32u) s->loop_frame = t;
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

/* Intervals agree: within 1/32 plus one loop. */
static int agree(const snd_slots *s, uint32_t a, uint32_t b)
{
	return udiff(a, b) <= a / 32u + (s->loop_q8 >> 8);
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

/* The run of intervals agreeing with interval a (from rise a to rise a + 1)
 * around it, at most RUN_MAX: returns its length (0 if it is no run), puts
 * their sum in *sum and in *bare whether every rise of it is a full swing.
 * A run has 2 intervals, or 3 when one of them touches a full swing (two
 * intervals beside a swing agree by chance: the rises a drum or the
 * sequencer's pause between notes makes when it leaves the speaker on). */
static uint32_t run_of(snd_decoder *d, uint32_t c, uint32_t a, uint32_t *sum, int *bare)
{
	const snd_slots *s = &d->sl;
	uint32_t n = kept(s, c), iv, cnt = 1, j;
	int swingy, all;

	iv = rise_at(s, c, a + 1u) - rise_at(s, c, a);
	*sum = iv;
	*bare = 0;
	if (iv < PER_MIN || iv > PER_MAX) return 0;
	swingy = swing_at(s, c, a) || swing_at(s, c, a + 1u);
	all = swing_at(s, c, a) && swing_at(s, c, a + 1u);
	for (j = a; j >= 1u && cnt < RUN_MAX; j--) {
		uint32_t x = rise_at(s, c, j) - rise_at(s, c, j - 1u);
		if (!agree(s, iv, x)) break;
		*sum += x;
		cnt++;
		swingy |= swing_at(s, c, j - 1u);
		all &= swing_at(s, c, j - 1u);
	}
	for (j = a + 1u; j + 1u < n && cnt < RUN_MAX; j++) {
		uint32_t x = rise_at(s, c, j + 1u) - rise_at(s, c, j);
		if (!agree(s, iv, x)) break;
		*sum += x;
		cnt++;
		swingy |= swing_at(s, c, j + 1u);
		all &= swing_at(s, c, j + 1u);
	}
	d->work += cnt;
	*bare = all;
	return cnt >= (swingy ? 3u : 2u) ? cnt : 0;
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
