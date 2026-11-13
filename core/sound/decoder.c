/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The beeper music decoder: edge history, style selector, note timing,
 * confidence. Each frame t arrives with its speaker edges; the decision is
 * about frame F = t - 3, with frames F+1..F+3 as look-ahead (the mixer plays
 * all sound 3 frames late, so the decision is on time).
 *
 * History: the speaker (EAR) edges of frames F..t (SND_HIST), T-states within
 * their frame; windows over several frames put them on one time axis. A frame
 * with more than SND_EDGES_MAX edges is too busy for speaker flips and is not
 * decoded as such (time slots read every edge as it arrives, slots.c).
 *
 * Work cap: the decoder runs on every frame in every sound mode, so its work
 * is counted (flips.c: search steps, edges looked at, pool passes; slots.c:
 * edges of each pass, grid points, histogram bins, table entries) and capped
 * at SND_WORK_MAX units a frame, 2/3 of it for the new frame's analysis and
 * the rest for the decision. Real and generated music needs at most about
 * 27,000 (typically 2,000-9,000); dense or adversarial edges stop at the cap.
 * Out of work is the safe direction: a frame whose analysis runs out is
 * undecodable, like a busy one, and a decision that runs out is not
 * confident and ends the music context.
 *
 * Style selector: a steady loop carrier (slots.c: most rising edges repeating
 * at one loop length of 40-400 T, the loop held through rests for 5 s) with
 * its channels learned makes a frame time slots (SLOTS), decided by
 * decide_slots() below, and the speaker-flip analysis is skipped for it.
 * Otherwise speaker flips: a frame is FLIPS when it reports voices or is a
 * confident part of the music, NONE otherwise. Speaker-flip engines have no
 * carrier (their rising edges are whole periods apart, 700 T and more).
 *
 * Time slots (decide_slots): the voices are the channels with a run of
 * agreeing periods through F's middle, or else through a quarter frame
 * either side (a note beginning or ending in F); full swings that no voice
 * explains, one in F and another within a quarter frame, are a drum. Music context as for speaker flips, from the
 * time-slot voices' pitch changes, tracked at F+2 (one frame of look-ahead
 * left for the next rise), between notes up to 25 frames apart (slow bass
 * notes are known late); sound without the loop carrier that the channels do
 * not explain ends it. In a frame without the carrier whose rises all come
 * with every channel's (the whole speaker toggling: a plain square wave),
 * a voice must continue the last decision's (channels in unison, in phase)
 * and the voices' runs must explain 85% of F's rises. Confident: voices with 75% of F's box levels
 * explained, in music context; a rest (no voice) after a confident decision,
 * for up to 50 frames; a drum in music context next to voices. Each voice's
 * level follows its slot's width.
 *
 * Speaker flips, per frame, when it arrives:
 *   - its voices alone (a 20 ms window) and with the frame before (40 ms);
 *   - clean: the frame alone decodes into voices explaining >= 90% of its
 *     edges (at least 4);
 *   - a pitch change: a clean frame with a voice the reference lacks while
 *     the reference has a voice it lacks (by more than 1/32 of the period,
 *     about half a semitone), the reference being the last note and clean
 *     within 10 frames (200 ms). A clean frame sharing a voice with the
 *     reference (the same note, a voice more or less, or the next note
 *     beginning beside it) keeps the reference.
 *
 * Note timing adapts by itself. Engines play either long notes that change
 * anywhere in a frame (40 ms windows and a 40 ms minimum note suit them) or
 * one note per 50 Hz frame from an interrupt sequencer (20 ms frame-aligned
 * windows); neither timing works for both. Each pitch change votes: the two
 * clean frames are neighbours and explain all but 2 of their edges (the old
 * voices to the end of one frame, the new from the start of the next) +1;
 * unexplained edges with no rest between the two (the change fell inside a
 * frame) -1; otherwise no vote. The votes are summed within -3..3; the
 * decoder starts with long notes, switches to one note per frame at +2 and
 * back at -2.
 *
 * The decision about F. Candidates: the voices of F alone, of the 40 ms
 * windows F-1..F and F..F+1, and (to explain edges only) of the decision
 * before and of F+1 alone, for notes ending or starting in F. A candidate
 * that is a whole multiple of a shorter one is dropped (flips.c). Shortest
 * first, each takes its chains (3 or more edges, running on into F+1) through
 * F's edges not taken yet. Reported: up to 3 candidates whose chains take 3
 * or more of F's edges and span at least half of F's sounding time (or half a
 * frame if F sounds less), the longest spans first; with one note per frame
 * only F's own voices may be reported, with long notes only the 40 ms
 * windows' (a voice only F alone shows is a transient). The share of F's
 * edges taken by all the candidates is F's fit.
 *
 * Confidence (confidence over coverage: unsure means beeper):
 *   - music context: a pitch change in F or F+1, or in the 100 frames (2 s)
 *     before F with no undecodable frame decided since (other sound ends the
 *     music until the next pitch change; such a frame also drops the
 *     reference note). A tone that never changes pitch for 2 s is a test tone or an
 *     effect, not music; so the first note of a tune plays on the beeper
 *     until the second begins (a lone tone cannot be told from a test tone
 *     within 60 ms of look-ahead).
 *   - voices reported and fit >= 85%: confident in music context.
 *   - fewer than 4 edges (a rest), fewer than 8, or fit >= 85% with no voice
 *     reported (a note ending or starting in F): confident in music context
 *     if the decision before was, for up to 50 frames after the last
 *     confident voices; no voices, so nothing is held across a rest.
 *   - fit < 50% with 8 or more edges: a drum (a buzz waveform, a sweep or
 *     noise between notes), reported with no tonal voice and confident only
 *     in music context with confident voices in the 4 frames before and a
 *     clean frame in the 3 after; otherwise not confident.
 *   - anything else (a frame partly explained): not confident.
 * Voices keep their tone slot from one decision to the next, so the synth
 * hears a held note as one note. */

#include "sound.h"

#define FIT_CLEAN 90u       /* % of a frame's edges its own voices explain: clean */
#define FIT_TONAL 85u       /* % explained for a confident tonal frame */
#define FIT_DRUM 50u        /* % explained below which an unexplained busy frame is a drum */
#define MUSIC_FRAMES 100u   /* a pitch change within this many frames: music */
#define REST_FRAMES 50u     /* a rest keeps FM this long after the last confident voices */
#define CHANGE_GAP 10u      /* most frames between two clean frames for a pitch change (200 ms) */
#define WORK_ANALYSIS (SND_WORK_MAX * 2u / 3u)   /* of the frame's work, the most its own analysis may use */
#define TIMING_MAX 3
#define TIMING_SWITCH 2
#define VOICE_LEVEL 160u
#define DRUM_LEVEL 200u
#define FIT_SLOTS 75u       /* % of a time-slot frame's box levels a combination explains */
#define SLOTS_GAP 25u       /* most frames between two time-slot notes for a pitch change (500 ms) */

static uint32_t ring(uint32_t frame)
{
	return frame % SND_HIST;
}

void snd_decoder_init(snd_decoder *d)
{
	uint32_t i, k;

	d->frames = 0;
	for (i = 0; i < SND_HIST; i++) {
		for (k = 0; k < SND_TONAL; k++) {
			d->slot[i].hp_q8[k] = 0;
			d->slot[i].pair_q8[k] = 0;
		}
		d->slot[i].n = 0;
		d->slot[i].expl = 0;
		d->slot[i].busy = 0;
		d->slot[i].clean = 0;
		d->slot[i].change = 0;
	}
	d->timing = 0;
	d->frame_notes = 0;
	d->have_clean = 0;
	d->have_change = 0;
	d->broken = 0;
	d->work = 0;
	d->work_cap = 0xffffffffu;
	d->clean_frame = 0;
	d->change_frame = 0;
	for (k = 0; k < SND_TONAL; k++) {
		d->clean_q8[k] = 0;
		d->out_q8[k] = 0;
	}
	d->since_tonal = REST_FRAMES;
	d->prev_conf = 0;
	snd_slots_reset(&d->sl);
	for (k = 0; k < SND_TONAL; k++)
		d->sref_q8[k] = 0;
	d->sref_frame = 0;
	d->schange_frame = 0;
	d->have_sref = 0;
	d->have_schange = 0;
	d->sbroken = 0;
}

/* Same voice: periods within 1/32 (about half a semitone). */
static int same(uint32_t a, uint32_t b)
{
	uint32_t hi = a > b ? a : b, diff = a > b ? a - b : b - a;
	return diff <= hi / 32u;
}

static int has_voice(const uint32_t *v, uint32_t p)
{
	uint32_t k;
	for (k = 0; k < SND_TONAL; k++)
		if (v[k] && same(v[k], p)) return 1;
	return 0;
}

/* Some voice of a is missing from b. */
static int lacks(const uint32_t *a, const uint32_t *b)
{
	uint32_t k;
	for (k = 0; k < SND_TONAL; k++)
		if (a[k] && !has_voice(b, a[k])) return 1;
	return 0;
}

/* Joins frames f..f+count-1 of the history into d->pool on one time axis;
 * returns the edge count and the first frame's in *first_n. */
static uint32_t join(snd_decoder *d, uint32_t f, uint32_t count, uint32_t *first_n)
{
	uint32_t n = 0, k, i;

	for (k = 0; k < count; k++) {
		const snd_slot *s = &d->slot[ring(f + k)];
		const uint32_t *t = d->t[ring(f + k)];
		for (i = 0; i < s->n; i++)
			d->pool[n++] = t[i] + k * ZX_FRAME_TSTATES;
		if (k == 0 && first_n) *first_n = n;
	}
	return n;
}

/* Stores frame t's speaker edges and analyses the frame on its own and with
 * the one before. */
static void take_frame(snd_decoder *d, uint32_t t, const zx_edge *e, uint32_t n, uint8_t start_level)
{
	snd_slot *s = &d->slot[ring(t)];
	uint32_t *dst = d->t[ring(t)];
	uint32_t i, k = 0, m, explained, voices = 0;
	uint8_t ear = (start_level >> 1) & 1u;

	/* time slots first: they read every edge, however many */
	d->work_cap = WORK_ANALYSIS;
	snd_slots_frame(d, e, n, start_level, s);

	s->busy = 0;
	for (i = 0; i < n; i++) {
		uint8_t lv = (e[i].level >> 1) & 1u;
		if (lv == ear) continue;   /* MIC only: not the speaker */
		ear = lv;
		if (k == SND_EDGES_MAX) {
			s->busy = 1;
			break;
		}
		dst[k++] = e[i].tstate;
	}
	s->n = (uint16_t)k;
	s->expl = 0;
	s->clean = 0;
	s->change = 0;
	for (i = 0; i < SND_TONAL; i++) {
		s->hp_q8[i] = 0;
		s->pair_q8[i] = 0;
	}
	if (s->busy || s->tdm) {
		/* too busy for speaker flips: some other sound, which ends the music
		 * this decoder was following; or time slots, decided as such */
		d->have_clean = 0;
		return;
	}

	m = join(d, t, 1, 0);
	explained = snd_flips_decode(d, d->pool, m, s->hp_q8);
	s->expl = (uint16_t)explained;
	for (i = 0; i < SND_TONAL; i++)
		if (s->hp_q8[i]) voices++;
	s->clean = voices && k >= 4u && explained * 100u >= k * FIT_CLEAN;
	if (t > 0 && !d->slot[ring(t - 1u)].busy) {
		m = join(d, t - 1u, 2, 0);
		snd_flips_decode(d, d->pool, m, s->pair_q8);
	}
	if (d->work >= d->work_cap) {
		/* out of work: undecodable, like a frame with too many edges */
		s->busy = 1;
		s->clean = 0;
		s->expl = 0;
		for (i = 0; i < SND_TONAL; i++) {
			s->hp_q8[i] = 0;
			s->pair_q8[i] = 0;
		}
		d->have_clean = 0;
	}
}

/* Pitch changes and the note-timing votes, with frame t just analysed. */
static void track_changes(snd_decoder *d, uint32_t t)
{
	snd_slot *s = &d->slot[ring(t)];
	uint32_t k;
	int vote = 0;

	if (!s->clean) return;
	if (!d->have_clean || t - d->clean_frame > CHANGE_GAP) {
		/* a new reference after silence or other sound: not a change */
	} else if (lacks(s->hp_q8, d->clean_q8) && lacks(d->clean_q8, s->hp_q8)) {
		uint32_t f, rest = 0, stray = 0;
		s->change = 1;
		if (t - d->clean_frame < SND_HIST) {   /* the frames between are still in the history */
			for (f = d->clean_frame; f <= t; f++) {
				const snd_slot *x = &d->slot[ring(f)];
				if (x->n < 4u) rest = 1;
				stray += x->n - x->expl;
			}
			if (t - d->clean_frame == 1u && stray <= 2u)
				vote = 1;    /* the old voices to the end of a frame, the new ones from the start of the next */
			else if (!rest && stray > 1u)
				vote = -1;   /* edges neither voice set explains: the change fell inside a frame */
		}
	} else {
		/* the same note (or a voice more or less, or a frame where the next note
		 * begins beside it): the reference stays */
		d->clean_frame = t;
		return;
	}
	d->have_clean = 1;
	d->clean_frame = t;
	for (k = 0; k < SND_TONAL; k++)
		d->clean_q8[k] = s->hp_q8[k];
	d->timing = (int8_t)(d->timing + vote);
	if (d->timing > TIMING_MAX) d->timing = TIMING_MAX;
	if (d->timing < -TIMING_MAX) d->timing = -TIMING_MAX;
	if (!d->frame_notes && d->timing >= TIMING_SWITCH) d->frame_notes = 1;
	if (d->frame_notes && d->timing <= -TIMING_SWITCH) d->frame_notes = 0;
}

typedef struct {
	uint32_t hp_q8;
	uint32_t edges;     /* F's edges it explains */
	uint32_t extent;    /* the time its chains span in F */
	uint8_t eligible;   /* may be reported under the timing in use */
} cand_t;

static void add_cand(cand_t *c, uint32_t *nc, uint32_t hp_q8, uint8_t eligible)
{
	uint32_t i, hp = hp_q8 >> 8, tol = hp >> 8 > 28u ? hp >> 8 : 28u;

	if (!hp_q8) return;
	for (i = 0; i < *nc; i++) {
		uint32_t o = c[i].hp_q8 >> 8;
		if ((o > hp ? o - hp : hp - o) <= tol) {
			c[i].eligible |= eligible;
			return;
		}
	}
	/* ascending */
	for (i = *nc; i > 0 && c[i - 1u].hp_q8 > hp_q8; i--)
		c[i] = c[i - 1u];
	c[i].hp_q8 = hp_q8;
	c[i].edges = 0;
	c[i].extent = 0;
	c[i].eligible = eligible;
	(*nc)++;
}

/* Fills *out with the decision: voices rep[0..nrep) (half-periods, T x 256)
 * at levels lvl, the drum, the style and the confidence. A voice that
 * continues keeps its tone slot, so the synth hears a held note as one note. */
static void emit(snd_decoder *d, const uint32_t *rep, const uint8_t *lvl, uint32_t nrep, uint8_t drum, uint8_t style,
                 int confident, snd_frame *out)
{
	uint32_t slot_q8[SND_TONAL], i, k;
	uint8_t taken[SND_TONAL], slot_lvl[SND_TONAL];

	for (k = 0; k < SND_TONAL; k++) {
		slot_q8[k] = 0;
		slot_lvl[k] = 0;
		taken[k] = 0;
	}
	for (k = 0; k < SND_TONAL; k++) {
		if (!d->out_q8[k]) continue;
		for (i = 0; i < nrep; i++)
			if (!taken[i] && same(rep[i], d->out_q8[k])) {
				slot_q8[k] = rep[i];
				slot_lvl[k] = lvl[i];
				taken[i] = 1;
				break;
			}
	}
	for (i = 0; i < nrep; i++) {
		if (taken[i]) continue;
		k = 0;
		while (k < SND_TONAL && slot_q8[k]) k++;
		if (k < SND_TONAL) {
			slot_q8[k] = rep[i];
			slot_lvl[k] = lvl[i];
		}
	}
	for (k = 0; k < SND_TONAL; k++) {
		d->out_q8[k] = slot_q8[k];
		out->tone[k].period_q8 = slot_q8[k] * 2u;
		out->tone[k].level = slot_q8[k] ? slot_lvl[k] : 0;
	}
	out->drum = drum;
	out->style = style;
	out->confident = (uint8_t)confident;

	d->prev_conf = (uint8_t)confident;
	if (confident && nrep) d->since_tonal = 0;
	else if (d->since_tonal < MUSIC_FRAMES) d->since_tonal++;
}

/* The voices of the last decision as periods (T x 256): the voices a run of
 * whole-speaker swings may continue (slots.c). */
static void keep_of(const snd_decoder *d, uint32_t *keep_q8)
{
	uint32_t k;
	for (k = 0; k < SND_TONAL; k++)
		keep_q8[k] = d->out_q8[k] * 2u;
}

/* Frame f is a plain tone to the time-slot decoder: no loop carrier, and
 * every channel rise in it came with all the others' (the whole speaker
 * toggling). Then a run of such swings is a voice only if it continues one
 * of keep_q8 (channels in unison, in phase), and the voices' runs must
 * explain FIT_TONAL% of its rises; otherwise it is other sound. Returns 1 if
 * frame f passes (not plain, or plain and explained). */
static int plain_ok(snd_decoder *d, uint32_t f, const uint32_t *keep_q8, int *plain)
{
	uint32_t rises, swings, explained, base = f * ZX_FRAME_TSTATES;

	*plain = 0;
	if (d->slot[ring(f)].carrier) return 1;
	explained = snd_slots_explained(d, base, base + ZX_FRAME_TSTATES, keep_q8, &rises, &swings);
	if (!rises || swings < rises) return 1;
	*plain = 1;
	return explained * 100u >= rises * FIT_TONAL;
}

/* The time-slot voices at the middle of frame f: half-periods (T x 256) and
 * levels; returns how many. */
static uint32_t slots_voices(snd_decoder *d, uint32_t f, const uint32_t *keep_q8, uint32_t *rep, uint8_t *lvl)
{
	uint32_t c, nrep = 0, mid = f * ZX_FRAME_TSTATES + ZX_FRAME_TSTATES / 2u;

	for (c = 0; c < d->sl.k && nrep < SND_TONAL; c++) {
		/* sounding through the middle of the frame, or else through a quarter
		 * frame either side (a note beginning or ending in the frame) */
		uint32_t p = snd_slots_voice(d, c, mid, keep_q8);
		if (!p) p = snd_slots_voice(d, c, mid + ZX_FRAME_TSTATES / 4u, keep_q8);
		if (!p) p = snd_slots_voice(d, c, mid - ZX_FRAME_TSTATES / 4u, keep_q8);
		if (!p) continue;
		rep[nrep] = p / 2u;
		if (lvl) lvl[nrep] = snd_slots_level(&d->sl, c);
		nrep++;
	}
	return nrep;
}

/* The time slots' music context, at the newest frame whose voices are known
 * (x = t - 1: one frame of look-ahead for the next rise): the reference note
 * and pitch changes, as for speaker flips, with notes up to SLOTS_GAP frames
 * apart (slow bass notes are known late). Unexplained sound without the loop
 * carrier drops the reference. */
static void track_slots(snd_decoder *d, uint32_t x)
{
	const snd_slot *s = &d->slot[ring(x)];
	uint32_t v[SND_TONAL], keep[SND_TONAL], n, k;
	int plain;

	if (!s->tdm || s->fit < FIT_SLOTS) {
		/* other sound (no loop carrier) drops the reference; the loop still
		 * running but fitting badly is a channel not learned yet */
		if (!s->tdm || !s->carrier) d->have_sref = 0;
		return;
	}
	for (k = 0; k < SND_TONAL; k++)
		v[k] = 0;
	keep_of(d, keep);
	if (!plain_ok(d, x, keep, &plain)) return;   /* a plain tone the voices do not explain */
	n = slots_voices(d, x, plain ? keep : 0, v, 0);
	if (!n) return;   /* a rest: the reference stays (for SLOTS_GAP frames) */
	if (!d->have_sref || x - d->sref_frame > SLOTS_GAP) {
		/* a new reference after silence or other sound: not a change */
	} else if (lacks(v, d->sref_q8) && lacks(d->sref_q8, v)) {
		d->schange_frame = x;
		d->have_schange = 1;
		d->sbroken = 0;
	} else {
		d->sref_frame = x;   /* the same note, or a voice more or less */
		return;
	}
	d->have_sref = 1;
	d->sref_frame = x;
	for (k = 0; k < SND_TONAL; k++)
		d->sref_q8[k] = v[k];
}

/* The decision about frame f when it was decoded as time slots. */
static void decide_slots(snd_decoder *d, uint32_t f, snd_frame *out)
{
	const snd_slot *s = &d->slot[ring(f)];
	uint32_t rep[SND_TONAL], keep[SND_TONAL], k, nrep, drums, base = f * ZX_FRAME_TSTATES;
	uint8_t lvl[SND_TONAL], drum = 0;
	int context, confident = 0, fit_ok = s->fit >= FIT_SLOTS, over = 0, plain, explained_ok;

	d->work_cap = SND_WORK_MAX;
	track_slots(d, f + SND_DELAY_FRAMES - 1u);
	for (k = 0; k < SND_TONAL; k++) {
		rep[k] = 0;
		lvl[k] = 0;
	}
	keep_of(d, keep);
	explained_ok = plain_ok(d, f, keep, &plain);
	nrep = slots_voices(d, f, plain ? keep : 0, rep, lvl);
	/* a drum: unexplained full swings in F, at least two of them within a
	 * quarter frame around it (a drum plays a few long pulses; the
	 * sequencer's pause between notes leaves one swing) */
	drums = snd_slots_drums(d, base, base + ZX_FRAME_TSTATES);
	if (drums == 1u &&
	    snd_slots_drums(d, base - ZX_FRAME_TSTATES / 4u, base + ZX_FRAME_TSTATES + ZX_FRAME_TSTATES / 4u) < 2u)
		drums = 0;
	context = d->have_schange && !d->sbroken && (int32_t)(f - d->schange_frame) <= (int32_t)MUSIC_FRAMES;

	if (d->work >= d->work_cap) {
		/* out of work: not decoded, so not music */
		over = 1;
		nrep = 0;
		fit_ok = 0;
		drums = 0;
	}
	if (drums && context && (nrep || (d->prev_conf && d->since_tonal < REST_FRAMES))) {
		/* another routine drove the whole speaker between the notes: a drum */
		drum = DRUM_LEVEL;
		confident = 1;
	} else if (!fit_ok || !explained_ok) {
		/* not decoded, or a plain tone the voices do not explain: the beeper */
		nrep = 0;
	} else if (nrep) {
		confident = context;
	} else {
		/* a rest, or no voice through F's middle: music goes on, holding no note */
		confident = context && d->prev_conf && d->since_tonal < REST_FRAMES;
	}
	if (over || (!fit_ok && !drum && !s->carrier && (int32_t)(d->schange_frame - f) <= 0)) {
		/* other sound: the music context stays off until the next pitch change */
		d->sbroken = 1;
	}
	emit(d, rep, lvl, nrep, drum, SND_STYLE_SLOTS, confident, out);
}

/* The decision about frame f (the look-ahead f+1..f+3 is in the history). */
static void decide(snd_decoder *d, uint32_t f, snd_frame *out)
{
	const snd_slot *s = &d->slot[ring(f)], *s1 = &d->slot[ring(f + 1u)];
	cand_t c[5u * SND_TONAL];
	uint32_t rep[SND_TONAL], nc = 0, nrep = 0, i, k, m, nf, fit = 0, sounding;
	int context, confident = 0, over = 0;
	uint8_t drum = 0;

	if (s->tdm) {
		decide_slots(d, f, out);
		return;
	}
	d->work_cap = SND_WORK_MAX;

	context = s->change || s1->change ||
	          (d->have_change && f - d->change_frame <= MUSIC_FRAMES &&
	           !d->broken);
	for (k = 0; k < SND_TONAL; k++)
		rep[k] = 0;

	if (!s->busy && s->n >= 4u) {
		/* candidates: F alone, F-1..F, F..F+1 */
		for (k = 0; k < SND_TONAL; k++) {
			add_cand(c, &nc, s->hp_q8[k], d->frame_notes);
			add_cand(c, &nc, s->pair_q8[k], !d->frame_notes);
			add_cand(c, &nc, s1->pair_q8[k], !d->frame_notes);
			/* fit only: the voices of the frames before and after, for a note ending or starting in F */
			add_cand(c, &nc, d->out_q8[k], 0);
			add_cand(c, &nc, s1->hp_q8[k], 0);
		}
		m = join(d, f, s1->busy ? 1u : 2u, &nf);
		for (i = 0; i < m; i++)
			d->used[i] = 0;
		sounding = d->pool[nf - 1u] - d->pool[0];
		if (sounding < ZX_FRAME_TSTATES / 2u) sounding = ZX_FRAME_TSTATES / 2u;
		for (i = 0; i < nc; i++) {
			for (k = 0; k < i; k++)
				if (snd_flips_multiple(c[i].hp_q8, c[k].hp_q8)) break;
			if (k < i) {   /* a multiple of a shorter candidate: not a voice */
				c[i].eligible = 0;
				continue;
			}
			c[i].edges = snd_flips_cover(d, d->pool, m, nf, c[i].hp_q8, &c[i].extent);
			fit += c[i].edges;
		}
		/* report: eligible voices covering half the sounding time, most covering first */
		for (;;) {
			uint32_t best = nc, best_e = 0;
			for (i = 0; i < nc; i++) {
				if (!c[i].eligible || c[i].edges < 3u || c[i].extent <= best_e || 2u * c[i].extent < sounding)
					continue;
				best = i;
				best_e = c[i].extent;
			}
			if (best == nc || nrep == SND_TONAL) break;
			rep[nrep++] = c[best].hp_q8;
			c[best].eligible = 0;
		}
		fit = fit * 100u / nf;
		if (d->work >= d->work_cap) {
			/* out of work: not decoded, so not music */
			over = 1;
			nrep = 0;
			for (k = 0; k < SND_TONAL; k++)
				rep[k] = 0;
		} else if (nrep && fit >= FIT_TONAL) {
			confident = context;
		} else if (fit >= FIT_TONAL || nf < 8u) {
			/* no voice sounds for half of F, and F is explained or nearly silent:
			 * the end of a note or the start of the next, around a rest */
			nrep = 0;
			for (k = 0; k < SND_TONAL; k++)
				rep[k] = 0;
			confident = context && d->prev_conf && d->since_tonal < REST_FRAMES;
		} else if (fit < FIT_DRUM) {
			/* mostly unexplained: a drum, whatever stray chains it holds */
			int ahead = 0;
			for (k = 0; k < SND_TONAL; k++)
				rep[k] = 0;
			nrep = 0;
			for (k = 1; k <= SND_DELAY_FRAMES; k++)
				if (d->slot[ring(f + k)].clean) ahead = 1;
			if (context && ahead && d->since_tonal < SND_DELAY_FRAMES + 1u) {
				confident = 1;
				drum = DRUM_LEVEL;
			}
		}
	} else if (!s->busy) {
		/* a rest (or nearly): music goes on through it, holding no note */
		confident = context && d->prev_conf && d->since_tonal < REST_FRAMES;
	}

	{
		uint8_t lvl[SND_TONAL];
		for (k = 0; k < SND_TONAL; k++)
			lvl[k] = VOICE_LEVEL;
		emit(d, rep, lvl, nrep, drum, (confident || nrep) ? SND_STYLE_FLIPS : SND_STYLE_NONE, confident, out);
	}
	if (s->busy || over) d->broken = 1;   /* other sound: the music context stays off until the next change */
	if (s->change) {
		d->have_change = 1;
		d->change_frame = f;
		d->broken = 0;
	}
}

void snd_decoder_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out)
{
	uint32_t t = d->frames, k;

	d->work = 0;
	take_frame(d, t, e, n, start_level);
	track_changes(d, t);
	if (t >= SND_DELAY_FRAMES) {
		decide(d, t - SND_DELAY_FRAMES, out);
	} else {
		for (k = 0; k < SND_TONAL; k++) {
			out->tone[k].period_q8 = 0;
			out->tone[k].level = 0;
		}
		out->drum = 0;
		out->style = SND_STYLE_NONE;
		out->confident = 0;
	}
	d->frames++;
}
