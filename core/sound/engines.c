/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B: known music engines, recognised by the code around the OUT that drives the speaker,
 * their notes read from memory and registers and checked against the beeper's edges.
 *
 * Per frame (snd_b_frame), as it arrives:
 *  1. Every OUT site of the frame is looked up: a remembered match (its signature re-checked once
 *     a frame it is used) or, failing that, every entry's signature with the anchor that puts one
 *     of its sites at that OUT (not again for an address that matched nothing while the code
 *     around it stays the same). Every edge must come from a site of one matched engine but one
 *     (other code's single edge, left out), else the frame is not B's (other code drives the
 *     speaker; an engine's hold ends).
 *  2. The engine's reader gives the voices at the frame's end (tonal sites), how long they have
 *     held (since the note began, or a slide's last step) and, if the engine moved on to its next
 *     note after its last edge, that note (then the frame's edges are the voices last seen); drum
 *     sites make drum events.
 *  3. The check (below): the voices must agree with the edges they made, segment by segment in
 *     the frame (the voices before the change, a slide's step, the voices read). A frame that
 *     fails is not B's. The edges are used only for this and for where events go: B never takes
 *     a pitch from them.
 * Then, SND_DELAY_FRAMES later, the decision (decide): the frame's voice changes as events at
 * the first edge with the new voices (the note it moved on to after its last edge only if the
 * next frame is the engine's and passes), the drum on at its first edge and off after its last, an
 * engine that stops (no edge of its own for longer than twice its longest period) releasing its
 * voices half a period after its last edge, and up to SND_B_HOLD silent frames after an engine's
 * last edge claimed as its silence (the voices ring out on their envelopes; a tune's rests stay
 * the tune's, and other code's edges within them do not end the hold).
 *
 * An entry whose notes are calls of a routine (min_len: ROM BEEP) is checked its own way
 * (observe_calls); one whose channels each flip the speaker (flips: the two-count tune, the
 * unison melody) has its flips checked (flip).
 *
 * The built-in entries: Castaway's title tune and Escape from Colony 8's music (the pack's
 * games), the 48K ROM's BEEPER, and the two-count tune, the burst tune and the unison melody
 * (games outside the repository: their digests, offsets and formulas only), in engines_builtin.c;
 * engines_test.c adds
 * the test fixtures' engines for host tests. */

#include "sound_internal.h"

#define FRAME_T ZX_FRAME_TSTATES
#define DRUM_GAP 4000u        /* T without a drum edge that ends a drum */
#define DRUM_TAIL 1000u       /* the drum is released this long after its last edge */
#define EPS 24u               /* T a channel's OUT may come early or late (contention, branches) */
#define NEVER (INT32_MAX / 2) /* snd_b_track.set: a level not yet seen to change */
#define OLDEST (-(1 << 24))   /* times carried from frames before are kept no older (240 frames) */

/* ---- the table --------------------------------------------------------- */

const char *snd_engine_name(const snd_engine *table, uint32_t index)
{
	return table[index].name;
}

/* ---- matching ---------------------------------------------------------- */

uint64_t snd_engine_digest(const zx_machine *m, const snd_engine *en, uint16_t anchor)
{
	uint64_t h = 0xcbf29ce484222325ull;
	uint32_t i, w = 0, r = 0;

	for (i = 0; i < en->len; i++) {
		uint32_t byte;
		while (w < en->n_wild && en->wild[w] < i) w++;
		if (w < en->n_wild && en->wild[w] == i) continue;
		while (r < en->n_reloc && en->reloc[r] + 1u < i) r++;
		if (r < en->n_reloc && (en->reloc[r] == i || en->reloc[r] + 1u == i)) {
			uint32_t lo = en->reloc[r];
			uint32_t word = zx_peek(m, (uint16_t)(anchor + lo)) | (uint32_t)zx_peek(m, (uint16_t)(anchor + lo + 1u)) << 8;
			word = (word - anchor) & 0xFFFFu;
			byte = i == lo ? (word & 0xFFu) : word >> 8;
		} else {
			byte = zx_peek(m, (uint16_t)(anchor + i));
		}
		h = (h ^ byte) * 0x00000100000001b3ull;
	}
	return h;
}

/* The hash (FNV-1a 32) of the bytes the table's signatures cover with a site at pc: from b->back
 * before it to b->ahead after; 0 when they all lie in the ROM (they cannot change). */
static uint32_t window_sum(const snd_b *b, const zx_machine *zm, uint16_t pc)
{
	uint32_t h = 0x811c9dc5u, i, n = (uint32_t)b->back + b->ahead;
	const uint16_t from = (uint16_t)(pc - b->back);

	if (pc >= b->back && (uint32_t)pc + b->ahead <= ZX_ROM_SIZE) return 0;
	for (i = 0; i < n; i++)
		h = (h ^ zx_peek(zm, (uint16_t)(from + i))) * 0x01000193u;
	return h;
}

/* The match whose engine has a site at pc, re-checked this frame (NULL if none); *site is that
 * site's index. Tries the table where no remembered match has a site there, unless pc is a
 * remembered unknown address (it matched nothing) whose window (window_sum) is unchanged: a
 * signature depends on nothing else, so the table would fail again. A changed window (code
 * loaded, moved or modified there) is tried at once; the window is hashed every frame pc is
 * looked up (a change that kept its 32-bit hash would go unseen until the next change). */
static snd_b_match *find(snd_b *b, const zx_machine *zm, uint16_t pc, uint32_t *site)
{
	snd_b_match *free_m = &b->match[0];
	snd_b_unknown *u = 0;
	uint32_t i, k, s, sum, had = 0;

	for (i = 0; i < SND_B_MATCHES; i++) {
		snd_b_match *mt = &b->match[i];
		const snd_engine *en;
		if (!mt->entry) {
			free_m = mt;
			continue;
		}
		if (free_m->entry && mt->used < free_m->used) free_m = mt;
		en = &b->table[mt->entry - 1u];
		for (s = 0; s < en->n_sites; s++) {
			if ((uint16_t)(mt->anchor + en->sites[s].off) != pc) continue;
			if (mt->frame != b->frames) {
				mt->frame = b->frames;
				mt->ok = snd_engine_digest(zm, en, mt->anchor) == en->digest;
			}
			if (!mt->ok) {   /* the code changed: forget it */
				mt->entry = 0;
				free_m = mt;
				break;
			}
			mt->used = b->frames;
			*site = s;
			return mt;
		}
	}
	sum = window_sum(b, zm, pc);
	for (i = 0; i < SND_B_UNKNOWN; i++) {   /* pc's entry; else a free one, else the oldest */
		snd_b_unknown *x = &b->unknown[i];
		if (x->valid && x->pc == pc) {
			if (x->sum == sum) {
				x->used = b->frames;
				return 0;
			}
			u = x;
			had = 1;
			break;
		}
		if (!u || (u->valid && (!x->valid || x->used < u->used))) u = x;
	}
	for (k = 0; k < b->n; k++) {
		const snd_engine *en = &b->table[k];
		for (s = 0; s < en->n_sites; s++) {
			uint16_t anchor = (uint16_t)(pc - en->sites[s].off);
			if (snd_engine_digest(zm, en, anchor) != en->digest) continue;
			if (had) u->valid = 0;
			free_m->anchor = anchor;
			free_m->entry = (uint8_t)(k + 1u);
			free_m->ok = 1;
			free_m->frame = b->frames;
			free_m->used = b->frames;
			*site = s;
			return free_m;
		}
	}
	u->pc = pc;
	u->valid = 1;
	u->sum = sum;
	u->used = b->frames;
	return 0;
}

/* ---- classifying edges ------------------------------------------------- */

#define EXTRA_PCS 4u   /* unlisted OUT addresses remembered per frame */

typedef struct {          /* which site of the engine an edge came from */
	snd_b *b;
	const zx_machine *zm;
	snd_b_match *mine;
	const zx_out_site *sites;
	uint32_t ns;
	uint32_t site_of[ZX_OUT_SITES];    /* per listed site: the engine's site index or NOWHERE */
	uint16_t extra_pc[EXTRA_PCS];      /* OUTs past the listed sites, and their site index */
	uint32_t extra_site[EXTRA_PCS];
	uint32_t n_extra;
	int32_t bad_at;                    /* check: the edge before the first that contradicted the voices, or NEVER */
} classifier;

/* The engine's site index of an edge from pc, or NOWHERE (other code; also any OUT past the
 * listed sites and EXTRA_PCS more: a frame with that many is no engine's music). */
static uint32_t classify(classifier *c, uint16_t pc)
{
	uint32_t k, s = NOWHERE;
	snd_b_match *mt;

	for (k = 0; k < c->ns; k++)
		if (c->sites[k].pc == pc) return c->site_of[k];
	for (k = 0; k < c->n_extra; k++)
		if (c->extra_pc[k] == pc) return c->extra_site[k];
	if (c->n_extra == EXTRA_PCS) return NOWHERE;
	mt = find(c->b, c->zm, pc, &s);
	if (mt != c->mine) s = NOWHERE;
	c->extra_pc[c->n_extra] = pc;
	c->extra_site[c->n_extra++] = s;
	return s;
}

/* The role of an edge for the check: a tonal or stop site's role, or 0xFF. */
static uint32_t tonal_role(classifier *c, const snd_engine *en, uint16_t pc, uint32_t *chan)
{
	uint32_t s = classify(c, pc);

	if (s == NOWHERE || en->sites[s].role == SND_SITE_DRUM) return 0xFF;
	*chan = en->sites[s].chan;
	return en->sites[s].role;
}

/* ---- the check ---------------------------------------------------------
 *
 * In a loop where each channel's OUT sets the speaker to that channel's level, in a fixed order
 * once a pass, the edges tell each channel's level at certain times, soundly:
 *  - an edge from channel j's OUT: j's level is the edge's, at that time;
 *  - the level before it was set by the OUT before j's (its predecessor in the order);
 *  - no edge from j's edge until the next edge (from k): j and every channel whose OUT comes
 *    between j's and k's in the order (all of them if k is j) kept the level; so did any other
 *    channel, k itself included, if the next edge comes more than one and a half passes after its
 *    first OUT after j's (its OUTs before were silent; a longer path through the loop delays an
 *    OUT by less than one pass).
 * A rise is counted when a channel known low is next known high within one and a half passes
 * (its time pinned to a pass). A frame contradicts its voices when a channel's consecutive rises
 * do not lie a whole number of its read periods apart (one period, for a strict entry: a learned
 * one), within a sixteenth of a period plus a pass (more than one pair in eight), when a channel
 * stays known at one level for longer than its period (plus that tolerance), when a silent
 * channel is known high, or when a segment's edges leave a channel's level unknown (too few edges
 * to tell the voices) or a new note's edges all fall too near its start to be checked. A
 * channel's state is carried into the next frame while its voice holds, so a low note gets its
 * evidence over frames. One-channel engines (pass 0): every change of the level is an edge of
 * the channel. */

#define MARGIN 400u   /* T either side of where the voices changed that the check leaves out */

typedef struct {
	snd_b_track tr[SND_TONAL];
	uint32_t pairs[SND_TONAL], bad[SND_TONAL], rises[SND_TONAL];   /* the segment's */
	uint8_t held[SND_TONAL];     /* known at one level for longer than its period */
	uint32_t period[SND_TONAL], tol[SND_TONAL], pass;
	uint8_t blind;               /* the voices are not known: follow the levels only */
	uint8_t strict;              /* rises one period apart only (snd_engine.strict) */
	int32_t bad_at;              /* the first contradicting edge's time, or NEVER */
} check_state;

/* Channel ch's level is `level` from time t (its first OUT with it) to `until`. */
static void known(check_state *c, uint32_t ch, uint32_t level, int32_t t, int32_t until)
{
	snd_b_track *tr = &c->tr[ch];

	if (level && !c->period[ch] && !c->blind) c->rises[ch]++;   /* a silent channel known high: it sounds */
	if (tr->bit == level) {
		if (until > tr->obs) tr->obs = until;
		if (c->period[ch] && tr->obs - tr->set > (int32_t)(c->period[ch] + c->tol[ch])) c->held[ch] = 1;
		return;
	}
	/* (one channel: its every change is an edge of its own) */
	if (tr->bit == 0 && level && !c->blind && (!c->pass || t - tr->obs <= (int32_t)(c->pass + c->pass / 2u + EPS))) {
		if (tr->has_rise && c->period[ch]) {
			uint32_t d = (uint32_t)(t - tr->rise), p = c->period[ch];
			uint32_t k = (d + p / 2u) / p;
			uint32_t off = k ? (d > k * p ? d - k * p : k * p - d) : d;
			c->pairs[ch]++;
			if (!k || off > c->tol[ch] || (k > 1u && c->strict)) {
				c->bad[ch]++;
				if (c->bad_at == NEVER) c->bad_at = t;
			}
		}
		c->rises[ch]++;
		tr->has_rise = 1;
		tr->rise = t;
	}
	/* a level known from a change pinned to a pass starts its time held there; one first known
	 * here, from its next change */
	tr->set = tr->bit != 2 && t - tr->obs <= (int32_t)(c->pass + c->pass / 2u + EPS) ? t : NEVER;
	tr->bit = (uint8_t)level;
	tr->obs = until;
}

/* The voices become v (NULL: not known); a channel whose period changes counts its rises
 * afresh. */
static void voices(check_state *c, const snd_voice *v)
{
	uint32_t ch;

	c->blind = !v;
	for (ch = 0; ch < SND_TONAL; ch++) {
		uint32_t p = v ? v[ch].period_q8 >> 8 : 0;
		if (p != c->period[ch]) c->tr[ch].has_rise = 0;
		c->period[ch] = p;
		c->tol[ch] = p / 16u + c->pass + EPS;
	}
}

/* Starts following every channel afresh, with the voices v. */
static void restart(check_state *c, const snd_voice *v)
{
	uint32_t ch;

	for (ch = 0; ch < SND_TONAL; ch++) {
		c->tr[ch].bit = 2;
		c->tr[ch].set = c->tr[ch].obs = c->tr[ch].rise = 0;
		c->tr[ch].has_rise = c->tr[ch].paired = 0;
	}
	voices(c, v);
}

/* A time of the frame before, as one of this frame (clamped at OLDEST, NEVER kept); 1 if
 * clamped. */
static int age(int32_t *t)
{
	if (*t >= NEVER) return 0;
	*t -= (int32_t)FRAME_T;
	if (*t >= OLDEST) return 0;
	*t = OLDEST;
	return 1;
}

/* Does a voice of v sound? */
static int sounds(const snd_voice *v, const snd_engine *en)
{
	uint32_t ch;

	for (ch = 0; ch < en->channels; ch++)
		if (v[ch].period_q8) return 1;
	return 0;
}

/* A channel's level not known (in the segment so far; a flip only guessed is not known). */
static int unknown(const check_state *c, const snd_engine *en)
{
	uint32_t ch;

	for (ch = 0; ch < en->channels; ch++)
		if (c->tr[ch].bit >= 2) return 1;
	return 0;
}

/* T-states from channel a's OUT to channel b's next one. */
static uint32_t gap(const snd_engine *en, uint32_t a, uint32_t b)
{
	return en->pos[b] > en->pos[a] ? en->pos[b] - en->pos[a] : en->pos[b] + en->pass - en->pos[a];
}

/* The inference from channel j's edge (level `level`, time t) up to the next tonal edge, from
 * channel k at time tn: the OUTs between them were silent. */
static void silent(check_state *c, const snd_engine *en, uint32_t j, uint32_t level, int32_t t, uint32_t k, int32_t tn)
{
	uint32_t ch;

	if (!en->pass) return;
	for (ch = 0; ch < en->channels; ch++) {
		int32_t d, dk;
		if (ch == j) {   /* its own OUTs until the next edge */
			d = (int32_t)(k == j ? en->pass : gap(en, j, k));
			if (tn - t > d) known(c, j, level, t, tn - d);
			continue;
		}
		d = (int32_t)gap(en, j, ch);
		if (ch == k) {
			if (tn - t > d + (int32_t)(en->pass + en->pass / 2u)) known(c, ch, level, t + d, tn - (int32_t)en->pass);
			continue;
		}
		dk = (int32_t)(k == j ? en->pass : gap(en, j, k));
		/* between j's OUT and k's in the order; or after k's, if the quiet outlasts its OUT */
		if (d < dk || tn - t > d + (int32_t)(en->pass + en->pass / 2u)) known(c, ch, level, t + d, tn - (int32_t)gap(en, ch, k));
	}
}

/* Entries with flips (the speaker is the XOR of the channels' levels, each edge one channel's
 * flip): every flip of a channel must come half its period after its last one, within half the
 * tolerance (more than one in eight off contradicts the voices, as for rises). An edge from an
 * OUT all channels share (SND_CHAN_ANY) is the flip of the channel it fits (after a whole number
 * of half periods: flips that fall in one pass cancel), else of one not yet seen, else of the
 * nearest. Two flips in one pass cancel: a channel's flip that comes a few half periods after
 * its last one is not a contradiction if each flip it missed is explained by another sounding
 * channel due to flip then (cancelled); a channel not yet seen may be guessed to flip there,
 * but is known only once a later flip of its own (hidden or not) fits that guess. A new note's
 * first two flips, taken by order, may be each other's (the channel second in order flipping
 * first): an edge that fits no channel while two have been seen but have no pair counted
 * exchanges their last flips if one of them then fits (exchange). Edges one half period apart
 * of a channel just seen, with nothing between, are the flips of a channel an octave above it
 * not yet seen, if one sounds: that channel's flips between would show, unless the edges are
 * its own and the lower channel's flips cancel every other one. An in-phase f + 2f sounds
 * exactly like a single square at f, so a silent channel read as sounding exactly an octave
 * above another is accepted by design. */

/* How far t is from a whole number (*k, at least 1) of half periods after channel ch's last
 * flip; NOWHERE for a silent channel. */
static uint32_t due(const check_state *c, uint32_t ch, int32_t t, uint32_t *k)
{
	const uint32_t d = (uint32_t)(t - c->tr[ch].obs), h = c->period[ch] / 2u;

	*k = 1;
	if (!h) return NOWHERE;
	*k = (d + h / 2u) / h;
	if (!*k) *k = 1;
	return d > *k * h ? d - *k * h : *k * h - d;
}

/* Counts a pair of channel ch's flips, the later at t; bad: it contradicts the voices. */
static void pair(check_state *c, uint32_t ch, int bad, int32_t t)
{
	c->pairs[ch]++;
	c->tr[ch].paired = 1;
	if (bad) {
		c->bad[ch]++;
		if (c->bad_at == NEVER) c->bad_at = t;
	}
}

/* Channel ch flips at t, k (2 to 8) half periods after its last flip: is each flip it missed
 * (its last plus j half periods) explained by another sounding channel flipping in the same
 * pass (within half that channel's tolerance of a whole number of its half periods after its
 * last flip)? Each such flip is that channel's, its pair counted against its own period. A
 * channel not yet seen is guessed to flip there (bit 3: no pair, still not known). Nothing
 * changes unless every missed flip is explained. */
static int cancelled(check_state *c, const snd_engine *en, uint32_t ch, int32_t t, uint32_t k)
{
	snd_b_track keep[SND_TONAL];
	uint8_t who[8];   /* per missed flip: the channel, plus 4 if checked, plus 8 if bad */
	uint32_t j, o, ko;

	if (k > 8u) return 0;
	for (o = 0; o < en->channels; o++)
		keep[o] = c->tr[o];
	for (j = 1; j < k; j++) {
		const int32_t m = c->tr[ch].obs + (int32_t)(j * (c->period[ch] / 2u));
		for (o = 0; o < en->channels; o++) {
			if (o == ch || !c->period[o]) continue;
			if (c->tr[o].bit == 2) {
				c->tr[o].bit = 3;
				who[j] = (uint8_t)o;
				break;
			}
			if (c->tr[o].obs < m && due(c, o, m, &ko) <= c->tol[o] / 2u) {
				c->tr[o].bit = 1;
				who[j] = (uint8_t)(o | 4u | (ko != 1u ? 8u : 0u));
				break;
			}
		}
		if (o == en->channels) {
			for (o = 0; o < en->channels; o++)
				c->tr[o] = keep[o];
			return 0;
		}
		c->tr[o].obs = m;
	}
	for (j = 1; j < k; j++)
		if (who[j] & 4u) pair(c, who[j] & 3u, who[j] & 8u, t);
	return 1;
}

/* An edge at t that fits no channel: two sounding channels seen but with no pair counted have
 * their last flips exchanged if one of them then fits, one half period after its last flip.
 * The channel that fits, else ch (nothing exchanged). */
static uint32_t exchange(check_state *c, const snd_engine *en, uint32_t ch, int32_t t)
{
	uint32_t a, b, x, k;

	for (a = 0; a < en->channels; a++) {
		for (b = a + 1u; b < en->channels; b++) {
			snd_b_track *ta = &c->tr[a], *tb = &c->tr[b];
			const int32_t o = ta->obs;
			if (ta->bit != 1 || tb->bit != 1 || ta->paired || tb->paired || !c->period[a] || !c->period[b]) continue;
			ta->obs = tb->obs;
			tb->obs = o;
			for (x = a; x <= b; x += b - a)
				if (due(c, x, t, &k) <= c->tol[x] / 2u && k == 1u) return x;
			tb->obs = ta->obs;
			ta->obs = o;
		}
	}
	return ch;
}

static void flip(check_state *c, const snd_engine *en, uint32_t ch, int32_t t)
{
	uint32_t i, k, off, best = NOWHERE;
	const int any = ch == SND_CHAN_ANY;   /* (only flips from one OUT can cancel) */

	if (any) {
		for (i = ch = 0; i < en->channels; i++) {
			off = due(c, i, t, &k);
			if (c->tr[i].bit == 2) off = 1u << 30;
			else if (off > k * (c->tol[i] / 2u)) off = (1u << 31) | (off >> 2);
			else if (k > 1) off |= 1u << 29;
			if (off < best) {
				best = off;
				ch = i;
			}
		}
		if (best >> 31) ch = exchange(c, en, ch, t);   /* (none fits, all seen) */
		/* one half period after the last flip of a channel just seen: the edges are those of a
		 * channel an octave above it not yet seen, if one sounds */
		if (c->tr[ch].bit == 1 && !c->tr[ch].paired && due(c, ch, t, &k) <= c->tol[ch] / 2u && k == 1u) {
			for (i = 0; i < en->channels; i++) {
				const uint32_t p2 = 2u * c->period[i];
				if (c->tr[i].bit != 2 || !p2 || (p2 > c->period[ch] ? p2 - c->period[ch] : c->period[ch] - p2) > c->tol[ch]) continue;
				c->tr[i].obs = c->tr[ch].obs;
				c->tr[i].bit = 1;
				c->tr[ch].bit = 2;
				ch = i;
				break;
			}
		}
	}
	if (!c->blind) {
		if (!c->period[ch]) {
			c->rises[ch]++;   /* a silent channel sounds */
		} else if (c->tr[ch].bit != 2) {
			off = due(c, ch, t, &k);
			pair(c, ch, off > c->tol[ch] / 2u || (k != 1u && (!any || !cancelled(c, en, ch, t, k))), t);
		}
	}
	c->tr[ch].bit = 1;   /* (seen: its level is not followed) */
	c->tr[ch].obs = t;
}

#define SEGS 3u

typedef struct {          /* the voices of a frame in time: segment s from at[s] with voices v[s] */
	uint32_t n;
	uint32_t at[SEGS];        /* at[0] = 0 */
	const snd_voice *v[SEGS]; /* NULL: not known, not checked */
	uint8_t pause[SEGS];      /* the engine paused before the segment (a new note) */
} segments;

/* Do the voices agree with the engine's tonal edges, segment by segment? After a pause the
 * channels are followed afresh, the edges within MARGIN of it left out; at a step (a slide)
 * they are followed on. in: the check as the frame before left it with the first segment's
 * voices (NULL: not known); out (may be NULL): as this frame leaves it, with the last's. */
static int check(classifier *cl, const snd_engine *en, const zx_edge *e, uint32_t n, const segments *sg,
                 const snd_b_carry *in, snd_b_carry *out)
{
	check_state c;
	uint32_t i, ch, s, chan = 0, seg = 0;
	uint32_t first[SEGS], rises[SEGS][SND_TONAL], pairs[SEGS][SND_TONAL], bad[SEGS][SND_TONAL];
	uint8_t held[SEGS][SND_TONAL], blank[SEGS];   /* blank: a channel's level never known in it */
	uint8_t any[SEGS] = { 0, 0, 0 };              /* tonal edges from its start on, checked or not */
	int32_t pt = 0, bad_at;  /* the edge before: time, channel, level */
	uint32_t pj = 0, pl = 0;
	int have = 0;

	c.pass = en->pass;
	c.strict = en->strict;
	c.bad_at = NEVER;
	for (ch = 0; ch < SND_TONAL; ch++)
		c.period[ch] = 0;
	restart(&c, sg->v[0]);
	for (s = 0; s < SEGS; s++) {
		first[s] = NOWHERE;
		for (ch = 0; ch < SND_TONAL; ch++) {
			rises[s][ch] = pairs[s][ch] = bad[s][ch] = 0;
			held[s][ch] = 0;
		}
	}
	for (ch = 0; ch < SND_TONAL; ch++) {
		c.rises[ch] = c.pairs[ch] = c.bad[ch] = 0;
		c.held[ch] = 0;
		if (in && sg->v[0]) {
			c.tr[ch] = in->tr[ch];
			age(&c.tr[ch].set);
			age(&c.tr[ch].obs);
			if (age(&c.tr[ch].rise)) c.tr[ch].has_rise = 0;
		}
	}
	if (in && sg->v[0])
		voices(&c, sg->v[0]);   /* (keeps the rises carried in) */
	if (in && in->has && sg->v[0]) {
		have = 1;
		pt = in->t - (int32_t)FRAME_T;
		pj = in->chan;
		pl = in->level;
	}
	for (i = 0; i <= n; i++) {
		int32_t t = i < n ? (int32_t)e[i].tstate : (int32_t)FRAME_T;
		uint32_t role, level;
		/* the segment the edge is in; a new one starts afresh */
		while (seg + 1u < sg->n && (uint32_t)t >= sg->at[seg + 1u]) {
			for (ch = 0; ch < SND_TONAL; ch++) {
				rises[seg][ch] = c.rises[ch];
				pairs[seg][ch] = c.pairs[ch];
				bad[seg][ch] = c.bad[ch];
				held[seg][ch] = c.held[ch];
				c.rises[ch] = c.pairs[ch] = c.bad[ch] = 0;
				c.held[ch] = 0;
			}
			blank[seg] = unknown(&c, en);
			seg++;
			if (sg->pause[seg]) {
				restart(&c, sg->v[seg]);
				have = 0;
			} else {
				voices(&c, sg->v[seg]);
			}
		}
		if (i == n) break;
		role = tonal_role(cl, en, e[i].pc, &chan);
		level = (e[i].level >> 1) & 1u;
		if (role == 0xFF) continue;
		if (role == SND_SITE_STOP) {   /* the loop ended: the next note starts afresh */
			have = 0;
			continue;
		}
		any[seg] = 1;
		/* one channel: no edge for longer than twice its period (decide's stop) and a quarter
		 * of a frame (not the voices read alone: a wrong period must not end its own check) is
		 * a pause, not a contradiction (a routine that plays a held note in short bursts); it
		 * is followed afresh from here */
		if (have && !en->pass && sg->v[seg] && sg->v[seg][0].period_q8 && (uint32_t)(t - pt) > FRAME_T / 4u &&
		    (uint32_t)(t - pt) > 2u * (sg->v[seg][0].period_q8 >> 8) + 2000u) {
			restart(&c, sg->v[seg]);
			have = 0;
		}
		if ((seg && sg->pause[seg] && (uint32_t)t < sg->at[seg] + MARGIN) ||
		    (seg + 1u < sg->n && sg->pause[seg + 1u] && (uint32_t)t + MARGIN >= sg->at[seg + 1u])) {
			have = 0;   /* near a pause */
			continue;
		}
		if (sg->v[seg] && first[seg] == NOWHERE) first[seg] = (uint32_t)t;
		bad_at = c.bad_at;
		if (en->flips) {
			flip(&c, en, chan, t);
		} else {
			if (have) {
				silent(&c, en, pj, pl, pt, chan, t);
				if (en->pass) {   /* the level before: the predecessor's */
					uint32_t p = (chan + en->channels - 1u) % en->channels;
					int32_t d = (int32_t)gap(en, p, chan);
					known(&c, p, pl, t - d, t - d);
				}
			}
			known(&c, chan, level, t, t);
		}
		if (bad_at == NEVER && c.bad_at != NEVER && have) c.bad_at = pt;   /* (from the edge before it: a new note's first) */
		have = 1;
		pt = t;
		pj = chan;
		pl = level;
	}
	for (ch = 0; ch < SND_TONAL; ch++) {
		rises[seg][ch] = c.rises[ch];
		pairs[seg][ch] = c.pairs[ch];
		bad[seg][ch] = c.bad[ch];
		held[seg][ch] = c.held[ch];
	}
	blank[seg] = unknown(&c, en);
	if (seg + 1u < sg->n) {   /* the last voices began after the last edge */
		restart(&c, sg->v[sg->n - 1u]);
		have = 0;
	}
	cl->bad_at = c.bad_at;
	if (out) {
		for (ch = 0; ch < SND_TONAL; ch++)
			out->tr[ch] = c.tr[ch];
		out->has = (uint8_t)have;
		out->t = pt;
		out->chan = (uint8_t)pj;
		out->level = (uint8_t)pl;
	}
	for (s = 0; s < sg->n; s++) {
		if (sg->v[s] && sg->pause[s] && any[s] && first[s] == NOWHERE && sounds(sg->v[s], en)) return 0;   /* a new note's edges, none checked */
		if (!sg->v[s] || first[s] == NOWHERE) continue;
		if (blank[s]) return 0;   /* too few edges to tell the voices */
		for (ch = 0; ch < en->channels; ch++) {
			if (!sg->v[s][ch].period_q8) {
				if (rises[s][ch]) return 0;   /* a silent channel sounds */
				continue;
			}
			if (held[s][ch] || bad[s][ch] * 8u > pairs[s][ch]) return 0;
		}
	}
	return 1;
}

/* ---- engines whose notes are calls (min_len: ROM BEEP) ------------------
 *
 * One tonal site; each note is one call of the routine, a square of equal halves at exactly the
 * half-period read, so its edges are checked one by one, each within EPS of a half-period after
 * the one before (no share of the period: a misread by a semitone fails). In order, the frame's
 * edges must be the note the frame before ended with, carried on (the first a half-period after
 * that frame's last edge), then the note read (back from the last edge): anything else in the
 * frame (a call between them) is not B's. A new call is told by its edges: the routine's setup
 * lies between two calls, so their edges break from the half-period (or the pitch changes; a
 * call that follows one of the same pitch with no break is the same note to the ear). A note is
 * B's only if its whole length, from its first edge to half a period after its last
 * (rd->left of it to come after the frame), is at least min_len. A note whose first frame has a
 * single edge (nothing to check it by) is B's if the next frame carries it on and passes (ok 2,
 * decide). */

/* Is d within EPS of h? */
static int halfway(uint32_t d, uint32_t h)
{
	return d - h + EPS <= 2u * EPS;
}

static void observe_calls(const snd_b *b, const snd_engine *en, const zx_edge *e, uint32_t n, const snd_b_read *rd, snd_b_rec *r)
{
	const snd_b_rec *p = &b->rec[(b->frames + SND_HIST - 1u) % SND_HIST];
	const uint32_t h = rd->v[0].period_q8 >> 9, hp = p->v[0].period_q8 >> 9;
	uint32_t i = 0, j = n - 1u;

	r->v[0] = rd->v[0];
	r->read = 1;
	r->cont = (uint8_t)(p->whole && p->entry == r->entry && halfway(e[0].tstate + FRAME_T - p->t_last, hp));
	if (r->cont)
		for (i = 1; i < n && halfway(e[i].tstate - e[i - 1u].tstate, hp); i++) {}
	if (i == n) {   /* the note goes on (at the pitch read, or the edges are not the voice's) */
		j = 0;
		r->whole = 1;
		r->ok = h == hp;
	} else {
		while (j > i && halfway(e[j].tstate - e[j - 1u].tstate, h)) j--;
		if (i) {   /* the note before ends half a period after its last edge: a step to silence */
			r->t_step = e[i - 1u].tstate + hp;
			r->step = r->t_step < e[j].tstate;
		}
		r->whole = e[n - 1u].tstate - e[j].tstate + rd->left >= en->min_len;
		r->ok = j != i || !r->whole ? 0 : j + 1u < n ? 1 : 2;
	}
	r->t_row = e[j].tstate;
}

/* ---- per frame, as it arrives ------------------------------------------ */

static void observe(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, snd_b_rec *r)
{
	const snd_engine *en;
	classifier cl;
	uint32_t i, k, best = NOWHERE, best_t = 0, from = 0, row = 0, chan, stop = 0;
	uint32_t last_of[ZX_OUT_SITES];
	int foreign = 0, tonal_site = 0;
	snd_b_read rd;

	r->kind = SND_B_NONE;
	r->entry = 0;
	r->ok = 0;
	r->tonal = 0;
	r->drum = 0;
	r->last_mine = 0;
	r->read = 0;
	r->carry = 0;
	r->next = 0;
	r->step = 0;
	r->t_row = r->t_first = r->d_first = r->t_step = NOWHERE;
	r->t_last = r->d_last = 0;
	r->whole = r->cont = 0;
	for (k = 0; k < SND_TONAL; k++) {
		r->v[k].period_q8 = r->bv[k].period_q8 = 0;
		r->v[k].level = r->bv[k].level = 0;
	}
	if (!n) return;
	r->kind = SND_B_FOREIGN;
	if (!zm) return;

	cl.b = b;
	cl.zm = zm;
	cl.mine = 0;
	cl.n_extra = 0;
	cl.sites = zx_out_sites(zm, &cl.ns);
	if (cl.ns > ZX_OUT_SITES) cl.ns = ZX_OUT_SITES;
	for (k = 0; k < cl.ns; k++) {
		snd_b_match *mt = find(b, zm, cl.sites[k].pc, &cl.site_of[k]);
		last_of[k] = NOWHERE;
		if (!mt || (cl.mine && mt != cl.mine)) {   /* other code, or a second engine */
			cl.site_of[k] = NOWHERE;
			continue;
		}
		cl.mine = mt;
	}
	if (!cl.mine) return;
	en = &b->table[cl.mine->entry - 1u];
	r->entry = cl.mine->entry;

	/* every edge: its site's role, or other code's. One edge of other code's in the frame (the
	 * engine's caller setting the speaker low after it returns, a stray click) is left out: the
	 * check passes over it and FM does not play it. A second ends it: the frame is not B's (other
	 * code's sound). An entry whose notes are calls has its every edge checked: none is left out */
	r->last_mine = classify(&cl, e[n - 1u].pc) != NOWHERE;
	for (i = 0; i < n; i++) {
		uint32_t s = classify(&cl, e[i].pc);
		if (s == NOWHERE) {
			if (++foreign > 1 || en->min_len) break;
			continue;
		}
		for (k = 0; k < cl.ns; k++)
			if (cl.sites[k].pc == e[i].pc) last_of[k] = e[i].tstate;
		if (en->sites[s].role == SND_SITE_DRUM) {
			if (r->d_first == NOWHERE) r->d_first = e[i].tstate;
			r->d_last = e[i].tstate;
			r->drum = 1;
		} else {
			if (r->t_first == NOWHERE) r->t_first = e[i].tstate;
			r->t_last = e[i].tstate;
			r->tonal = 1;
			if (en->sites[s].role == SND_SITE_STOP) stop = e[i].tstate + 1u;
			else tonal_site = 1;
		}
	}
	if (foreign > 1 || (foreign && en->min_len)) return;
	r->kind = SND_B_MINE;
	r->ok = 1;

	/* the voices, from the tonal site whose last edge came last */
	for (k = 0; k < cl.ns; k++) {
		if (cl.site_of[k] == NOWHERE || last_of[k] == NOWHERE || en->sites[cl.site_of[k]].role != SND_SITE_TONAL) continue;
		if (best == NOWHERE || last_of[k] >= best_t) {
			best = k;
			best_t = last_of[k];
		}
	}
	if (best == NOWHERE) {   /* stop or drum edges only: the voices are as they were */
		if (b->last_entry == r->entry) {
			for (k = 0; k < SND_TONAL; k++)
				r->v[k] = b->lastv[k];
			r->read = 1;
		}
		/* tonal edges all from sites past the listed ones (no registers, not read): unchecked */
		if (tonal_site) r->ok = 0;
		return;
	}
	for (k = 0; k < SND_TONAL; k++) {
		rd.v[k].period_q8 = rd.back[k].period_q8 = 0;
		rd.v[k].level = rd.back[k].level = 0;
	}
	rd.since = rd.row = SND_B_SINCE_ALL;
	rd.step = 0;
	rd.stale = 0;
	rd.left = 0;
	en->read(zm, en, cl.mine->anchor, cl.site_of[best], &cl.sites[best].regs, &rd);
	if (en->min_len) {
		observe_calls(b, en, e, n, &rd, r);
		return;
	}
	{
		const snd_b_rec *p = &b->rec[(b->frames + SND_HIST - 1u) % SND_HIST];
		const int had = b->last_entry == r->entry;
		const snd_voice *pv = b->lastv;   /* the voices the engine was last seen with */
		int retry = 0;
		const snd_b_carry *in = b->frames && p->read && p->entry == r->entry && p->carry && !p->next ? &p->cy : 0;
		int same = 1;

		segments sg;

		sg.n = 1;
		sg.at[0] = 0;
		sg.pause[0] = 0;
		/* a learned entry that does not know its notes' timing, and read a new note its edges do
		 * not fit: the engine may have moved on to it after its last edge (a note changed on the
		 * frame's interrupt), or within the frame, the edges before being the voices of the
		 * frame before */
		if (!rd.stale && en->strict && had && rd.since == SND_B_SINCE_ALL) {
			for (k = 0; k < SND_TONAL; k++)
				if (pv[k].period_q8 != rd.v[k].period_q8) retry = 1;
			sg.v[0] = rd.v;
			if (retry && !check(&cl, en, e, n, &sg, 0, 0)) rd.stale = 1;
			else retry = 0;
		}
		if (rd.stale) {   /* the edges are the voices of the frame before, if it read them */
			if (had) {   /* (from the frame's first edge, if the frame before did not play them) */
				for (k = 0; k < SND_TONAL; k++)
					r->v[k] = pv[k];
				r->read = 1;
				r->t_row = r->t_first;
				sg.v[0] = pv;
				/* (a retried new note may have begun within the frame: then the voices before
				 * up to the edge before the first they do not fit, the note read from there,
				 * unchecked here) */
				while (!(r->ok = (uint8_t)check(&cl, en, e, n, &sg, in, &r->cy)) && retry-- > 0 && cl.bad_at > 0 && cl.bad_at < NEVER) {
					sg.n = 2;
					sg.at[1] = (uint32_t)cl.bad_at;
					sg.v[1] = 0;
					sg.pause[1] = 1;
				}
				r->carry = 1;
			} else {
				r->ok = 0;
			}
			if (rd.stale == 1) {   /* and the engine moved on to the voices read, after its last edge */
				r->next = 1;
				r->carry = 0;
				r->t_next = sg.n > 1 ? sg.at[1] : rd.since < FRAME_T - r->t_last ? FRAME_T - rd.since : r->t_last + 1u;
				for (k = 0; k < SND_TONAL; k++)
					r->nv[k] = k < en->channels ? rd.v[k] : (snd_voice){ 0, 0 };
			}
			return;
		}
		if (rd.since != SND_B_SINCE_ALL && rd.since < best_t) from = best_t - rd.since;
		if (rd.row != SND_B_SINCE_ALL && rd.row < best_t) row = best_t - rd.row;
		if (row && row == from && row < stop) row = from = stop;   /* a note begins after the stop before it */
		for (i = 0; i < n; i++) {
			if (e[i].tstate >= from && tonal_role(&cl, en, e[i].pc, &chan) != 0xFF) {
				r->t_row = e[i].tstate;
				break;
			}
		}
		for (k = 0; k < SND_TONAL && k < en->channels; k++)
			r->v[k] = rd.v[k];
		r->read = 1;
		r->carry = 1;
		/* in time: the voices the frame before ended with, up to the first change in the frame
		 * (a new note, or a slide's step: at most two fit in a frame), then the voices one step
		 * back (sliding), then those read; a new note begins after a pause */
		if (from) {
			const uint32_t back = rd.step && from > rd.step ? from - rd.step : 0;
			int known_back = 1;
			for (k = 0; k < SND_TONAL; k++)
				if (rd.v[k].period_q8 && !rd.back[k].period_q8) known_back = 0;
			sg.v[0] = had ? pv : 0;
			if (back) {
				sg.at[sg.n] = back;
				sg.v[sg.n] = known_back ? rd.back : 0;
				sg.pause[sg.n++] = back == row;
				for (i = 0; known_back && i < n; i++) {   /* the step's voices from its first edge */
					if (e[i].tstate >= back && tonal_role(&cl, en, e[i].pc, &chan) != 0xFF) {
						r->t_step = e[i].tstate;
						for (k = 0; k < SND_TONAL; k++)
							r->bv[k] = rd.back[k];
						break;
					}
				}
				r->step = r->t_step < r->t_row;
			}
			sg.at[sg.n] = from;
			sg.v[sg.n] = r->v;
			sg.pause[sg.n++] = from == row;
			r->ok = (uint8_t)check(&cl, en, e, n, &sg, had ? in : 0, &r->cy);
		} else {
			for (k = 0; k < SND_TONAL; k++)
				if (!had || pv[k].period_q8 != r->v[k].period_q8) same = 0;
			sg.v[0] = r->v;
			r->ok = (uint8_t)check(&cl, en, e, n, &sg, same ? in : 0, &r->cy);
		}
	}
}

/* ---- the decision, SND_DELAY_FRAMES later ------------------------------- */

typedef struct {
	snd_event ev[2u * SND_EVENTS];
	uint32_t n;
} ev_list;

static void add(ev_list *l, uint32_t t, uint8_t voice, uint32_t period_q8, uint8_t level)
{
	uint32_t i;

	if (l->n >= sizeof l->ev / sizeof l->ev[0]) return;
	if (t >= FRAME_T) t = FRAME_T - 1u;
	i = l->n++;
	while (i && l->ev[i - 1u].t > t) {   /* in time order; same time: as added */
		l->ev[i] = l->ev[i - 1u];
		i--;
	}
	l->ev[i].t = t;
	l->ev[i].voice = voice;
	l->ev[i].period_q8 = period_q8;
	l->ev[i].level = level;
}

/* An off at T-state t of this frame, or carried into the next one. */
static void off_at(snd_b *b, ev_list *l, uint32_t t, uint8_t voice)
{
	if (t < FRAME_T) {
		add(l, t, voice, 0, 0);
	} else {
		t -= FRAME_T;
		b->carry[voice] = t < FRAME_T ? t : FRAME_T - 1u;
	}
}

/* The next tonal (drum) edge after frame f's, as T-states from frame f's start; NOWHERE if none
 * in the frames already seen. */
static uint32_t next_edge(const snd_b *b, uint32_t f, int drum)
{
	uint32_t k;

	for (k = 1; k <= SND_DELAY_FRAMES; k++) {
		const snd_b_rec *r = &b->rec[(f + k) % SND_HIST];
		uint32_t t = drum ? r->d_first : r->t_first;
		if (r->kind != SND_B_NONE && t != NOWHERE) return k * FRAME_T + t;
	}
	return NOWHERE;
}

static void decide(snd_b *b, snd_frame *out)
{
	const uint32_t f = b->frames - 1u - SND_DELAY_FRAMES;   /* b->frames counts the frame just fed */
	const snd_b_rec *r = &b->rec[f % SND_HIST], *nx = &b->rec[(f + 1u) % SND_HIST];
	const snd_engine *en = r->entry ? &b->table[r->entry - 1u] : 0;
	snd_voice cur[SND_TONAL];
	ev_list l;
	uint32_t k, i;
	int conf;

	if (r->kind == SND_B_MINE)   /* (ok 2: if the next frame carries the note on and passes) */
		conf = r->ok == 1 || (r->ok == 2 && nx->cont && nx->ok == 1);
	else if (r->kind == SND_B_NONE)
		conf = b->engine && b->held < SND_B_HOLD;
	else
		conf = 0;

	for (k = 0; k < SND_TONAL; k++)
		out->tone[k] = b->tone[k];
	out->drum = b->drum;
	out->n_ev = 0;
	if (!conf) {
		silence(b);
		/* a hold may follow the engine's own edges after the other code's; other code's edges
		 * within the engine's hold (a stray edge in a rest) leave the hold running: the mixer
		 * passes over a lone unconfident frame, so the rest stays FM */
		if (r->kind == SND_B_FOREIGN && r->last_mine) {
			b->engine = r->entry;
			b->held = 0;
		} else if (r->kind == SND_B_FOREIGN && b->engine && b->held < SND_B_HOLD) {
			b->held++;
		} else {
			b->engine = 0;
			b->held = 0;
		}
		for (k = 0; k < SND_TONAL; k++)
			out->tone[k] = r->v[k];
		out->drum = 0;
		out->style = SND_STYLE_NONE;
		out->confident = 0;
		out->engine = 0;
		return;
	}

	l.n = 0;
	for (k = 0; k <= SND_TONAL; k++) {
		if (b->carry[k] == NOWHERE) continue;
		add(&l, b->carry[k], (uint8_t)k, 0, 0);
		b->carry[k] = NOWHERE;
	}
	for (k = 0; k < SND_TONAL; k++)
		cur[k] = b->tone[k];
	if (r->kind == SND_B_MINE) {
		if (r->step) {   /* a slide's step before the voices read (or a call's end: bv silent) */
			for (k = 0; k < SND_TONAL; k++) {
				if (r->bv[k].period_q8 == cur[k].period_q8 && r->bv[k].level == cur[k].level) continue;
				add(&l, r->t_step, (uint8_t)k, r->bv[k].period_q8, r->bv[k].level);
				cur[k] = r->bv[k];
			}
		}
		if (r->t_row != NOWHERE) {   /* the voices were read */
			for (k = 0; k < SND_TONAL; k++) {
				if (r->v[k].period_q8 == cur[k].period_q8 && r->v[k].level == cur[k].level) continue;
				add(&l, r->t_row, (uint8_t)k, r->v[k].period_q8, r->v[k].level);
				cur[k] = r->v[k];
			}
		}
		if (r->next && nx->kind == SND_B_MINE && nx->ok == 1) {   /* it moved on after its last edge (if the next frame passes) */
			for (k = 0; k < SND_TONAL; k++) {
				if (r->nv[k].period_q8 == cur[k].period_q8 && r->nv[k].level == cur[k].level) continue;
				add(&l, r->t_next, (uint8_t)k, r->nv[k].period_q8, r->nv[k].level);
				cur[k] = r->nv[k];
			}
		}
		if (r->tonal) {   /* does the engine stop after its last edge? */
			uint32_t pmax = 0, next = next_edge(b, f, 0);
			for (k = 0; k < SND_TONAL; k++)
				if (cur[k].period_q8 && cur[k].level && (cur[k].period_q8 >> 8) > pmax) pmax = cur[k].period_q8 >> 8;
			if (pmax && (next == NOWHERE || next - r->t_last > 2u * pmax + 2000u)) {
				for (k = 0; k < SND_TONAL; k++)
					if (cur[k].period_q8 && cur[k].level) off_at(b, &l, r->t_last + (cur[k].period_q8 >> 9), (uint8_t)k);
			}
		}
		if (r->drum) {
			uint32_t next = next_edge(b, f, 1);
			if (!b->drum && en && en->drum) add(&l, r->d_first, SND_TONAL, 0, en->drum);
			if (next == NOWHERE || next - r->d_last > DRUM_GAP) off_at(b, &l, r->d_last + DRUM_TAIL, SND_TONAL);
		}
		b->engine = r->entry;
		b->held = 0;
	} else {
		b->held++;
	}

	/* the events, in time order; those past SND_EVENTS are dropped (the state follows the
	 * events played) */
	for (i = 0; i < l.n && i < SND_EVENTS; i++) {
		const snd_event *ev = &l.ev[i];
		out->ev[i] = *ev;
		if (ev->voice < SND_TONAL) {
			b->tone[ev->voice].period_q8 = ev->period_q8;
			b->tone[ev->voice].level = ev->level;
		} else {
			b->drum = ev->level;
		}
	}
	out->n_ev = (uint8_t)i;
	out->style = SND_STYLE_ENGINE;
	out->confident = 1;
	out->engine = b->engine;
	/* the claiming entry's stored roles (SND_ROLE_AUTO past its channels): the live rule fills
	 * the voices left AUTO */
	for (k = 0; k < SND_TONAL; k++)
		out->role[k] = b->table[b->engine - 1u].role[k];
}

void snd_b_frame(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out)
{
	uint32_t k;

	(void)start_level;
	if (n > ZX_MAX_EDGES) n = ZX_MAX_EDGES;
	{
		snd_b_rec *r = &b->rec[b->frames % SND_HIST];
		observe(b, zm, e, n, r);
		if (r->read) {   /* the voices the engine was seen with last */
			for (k = 0; k < SND_TONAL; k++)
				b->lastv[k] = r->next ? r->nv[k] : r->v[k];
			b->last_entry = r->entry;
		}
	}
	b->frames++;
	for (k = 0; k < SND_TONAL; k++)
		out->role[k] = SND_ROLE_AUTO;
	if (b->frames > SND_DELAY_FRAMES) {
		decide(b, out);
		return;
	}
	for (k = 0; k < SND_TONAL; k++) {
		out->tone[k].period_q8 = 0;
		out->tone[k].level = 0;
	}
	out->drum = 0;
	out->style = SND_STYLE_NONE;
	out->confident = 0;
	out->engine = 0;
	out->n_ev = 0;
}
