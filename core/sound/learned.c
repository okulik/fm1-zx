/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Learned entries: method B's entries for a game's own music routines, which the personal-pack
 * builder learned and stored in the pack as data (docs/pack-format.md). Here: the interpreter of
 * their programs, the decoder of a game's entry list into snd_engine records, and the generic
 * reader those records share.
 *
 * A program is bytecode for a stack machine of uint32_t values, at most PROG_DEPTH deep and
 * SND_PROG_MAX bytes long, with little-endian operands:
 *   01 k8, 02 k16, 03 k32     push k
 *   10 o16, 11 o16            push the byte, the word at anchor + o
 *   12 a16, 13 a16            push the byte, the word at a
 *   14 r8 d8                  push the byte at reg16(r) + d (d signed; r BC, DE, HL, IX or IY)
 *   20 r8                     push register r
 *   30 add, 31 sub, 32 mul    (wrapping), 33 div (unsigned; a zero divisor: silent), 34 and
 *   35 s8, 36 s8              shift right, left by s (0-31)
 *   40                        pop x; x = 0: silent
 * Registers: 0 A, 1 F, 2 B, 3 C, 4 D, 5 E, 6 H, 7 L, 8 BC, 9 DE, 10 HL, 11 IX, 12 IY, 13 SP,
 * 14 A', 15 BC', 16 DE', 17 HL'. A program ends at its length with one value on the stack, its
 * result; anything else (an unknown op, an operand past the end, a stack underflow or overflow,
 * another depth at the end) is invalid. Memory is read through zx_peek, so any address is safe.
 *
 * The list comes from the pack: the decoder trusts nothing in it. It checks every count, size and
 * offset against the list and the entry, bounds the matching's work (SND_LEARNED_LEN_MAX bytes a
 * signature, SND_LEARNED_WORK_MAX sites x signature bytes over the list: engines.c's find()
 * hashes each site's signature for an OUT no entry is known at), and dry-runs every program
 * (the interpreter with no machine: every op and operand checked, every read 0, nothing silent)
 * before an entry is used. An entry's flags byte is 0, 1 (flips: snd_engine.flips) or 2 (observed:
 * snd_engine.observed, every tonal site channel 0); any other value refuses the list.
 *
 * Besides learned entries (kind 0), a list may hold kind-1 records: stored roles for a built-in
 * entry, by its digest, which snd_learned_roles copies into the firmware's assembled table; and
 * kind-2 records, recorded scores, which the decoder checks whole (every size, count and event)
 * and the player below plays.
 *
 * A recorded score: its times are T-states from its trigger edge (T0); bucket b is the frame-long
 * span [b x ZX_FRAME_TSTATES, (b + 1) x ZX_FRAME_TSTATES) from T0, and the fingerprint the speaker
 * edges (EAR level changes, from any OUT) of each bucket, capped at 255. Method B's player, once
 * a frame after the decision (snd_b.scores), with frame j just fed:
 *  - following: the playing score's bucket that ends in frame j (from T0's T-state in frame j - 1
 *    to T0's in frame j) is compared with the fingerprint: they agree when |live - recorded| <=
 *    max(2, recorded / 8). Three buckets in a row that disagree stop the score: the frames from
 *    j - SND_DELAY_FRAMES on, not yet decided, are not its (the first of the three buckets began
 *    in frame j - SND_DELAY_FRAMES). Its last bucket compared ends the following.
 *  - arming: while no score plays (from its trigger to its last frame decided), a score whose site
 *    makes a speaker edge in frame j, after at least `quiet` frames with none from it (a game's
 *    first frames count as quiet), and whose code (the FNV-1a 32 of the 32 bytes from site - 16)
 *    is the record's, starts: T0 is its site's first speaker edge in frame j.
 *  - the decision for frame j - SND_DELAY_FRAMES, if the score plays it (from frame f0, T0's, to
 *    the one its fingerprint ends in, or up to the frame before it stopped): its events whose times
 *    fall in the frame (each at its T-state in the frame) and, in the frame its fingerprint ends,
 *    the voices still sounding released where it ends. Unless the decision has the frame
 *    confident (a table entry claims it: the table wins, its events stand), the frame becomes the
 *    score's: confident, its voices at the frame's start and its events (the first SND_EVENTS),
 *    engine n + 1 + its index (n the table's entries), its stored roles, B's voices the score's and
 *    nothing of a table entry's carried or held.
 * A frame after a score stops is decided as without it (the beeper's, unless the table claims
 * it): B's voices are silenced there and the mixer leaves FM.
 *
 * Built into flash, not SRAM, on the device (the reader runs only for frames a learned entry
 * claims; the player, a pass or two over a frame's edges, through snd_b.scores). uint32_t
 * arithmetic only (no 64-bit division). */

#include "sound_internal.h"

#define PROG_DEPTH 8u
#define HEAD 4u       /* the list's header: count, 0, 0 (u16) */
#define ENTRY 36u     /* an entry's fixed part */
#define ROLES 16u     /* a kind-1 record (roles for a built-in entry), padded */
#define REGS 18u      /* register numbers 0-17 */
#define FLIPS 1u      /* an entry's flags byte: snd_engine.flips */
#define OBSERVED 2u   /* snd_engine.observed */
#define SCORE 20u     /* a kind-2 record's fixed part (a recorded score) */
#define UNITS 1092u   /* a bucket (one frame) in the 64-T units of a score's times */

_Static_assert(ZX_FRAME_TSTATES == UNITS * 64u, "a bucket is a whole number of 64-T units");

static uint32_t rd16(const uint8_t *p)
{
	return p[0] | (uint32_t)p[1] << 8;
}

static uint32_t rd32(const uint8_t *p)
{
	return rd16(p) | rd16(p + 2) << 16;
}

static uint32_t peek16(const zx_machine *m, uint32_t a)
{
	return zx_peek(m, (uint16_t)a) | (uint32_t)zx_peek(m, (uint16_t)(a + 1u)) << 8;
}

/* Register n (< REGS) of r. */
static uint32_t reg(const zx_regs *r, uint32_t n)
{
	switch (n) {
	case 0: return r->af >> 8;
	case 1: return r->af & 0xFFu;
	case 2: return r->bc >> 8;
	case 3: return r->bc & 0xFFu;
	case 4: return r->de >> 8;
	case 5: return r->de & 0xFFu;
	case 6: return r->hl >> 8;
	case 7: return r->hl & 0xFFu;
	case 8: return r->bc;
	case 9: return r->de;
	case 10: return r->hl;
	case 11: return r->ix;
	case 12: return r->iy;
	case 13: return r->sp;
	case 14: return r->af_ >> 8;
	case 15: return r->bc_;
	case 16: return r->de_;
	default: return r->hl_;
	}
}

/* The interpreter; m NULL: the dry run (no reads: each gives 0; nothing silent). */
static int run(const uint8_t *p, uint32_t len, const zx_machine *m, uint16_t anchor, const zx_regs *ctx, uint32_t *out)
{
	uint32_t s[PROG_DEPTH], d = 0, i = 0;

	if (len > SND_PROG_MAX) return -1;
	while (i < len) {
		const uint32_t op = p[i++], left = len - i;
		uint32_t x = 0, y, a;
		if (op >= 0x30 && op <= 0x34) {   /* binary */
			if (d < 2) return -1;
			y = s[--d];
			x = s[d - 1u];
			if (op == 0x30) x += y;
			else if (op == 0x31) x -= y;
			else if (op == 0x32) x *= y;
			else if (op == 0x34) x &= y;
			else if (y) x /= y;
			else if (m) return 1;   /* a zero divisor */
			s[d - 1u] = x;
			continue;
		}
		switch (op) {
		case 0x35:
		case 0x36:
			if (left < 1 || p[i] > 31u || d < 1) return -1;
			s[d - 1u] = op == 0x35 ? s[d - 1u] >> p[i] : s[d - 1u] << p[i];
			i++;
			continue;
		case 0x40:
			if (d < 1) return -1;
			if (!s[--d] && m) return 1;
			continue;
		case 0x01:
			if (left < 1) return -1;
			x = p[i++];
			break;
		case 0x02:
			if (left < 2) return -1;
			x = rd16(p + i);
			i += 2;
			break;
		case 0x03:
			if (left < 4) return -1;
			x = rd16(p + i) | rd16(p + i + 2) << 16;
			i += 4;
			break;
		case 0x10:
		case 0x11:
		case 0x12:
		case 0x13:
			if (left < 2) return -1;
			a = rd16(p + i) + (op < 0x12 ? anchor : 0u);
			i += 2;
			if (m) x = op & 1u ? peek16(m, a) : zx_peek(m, (uint16_t)a);
			break;
		case 0x14:
			if (left < 2 || p[i] < 8u || p[i] > 12u) return -1;
			if (m) x = zx_peek(m, (uint16_t)(reg(ctx, p[i]) + (uint32_t)(int32_t)(int8_t)p[i + 1]));
			i += 2;
			break;
		case 0x20:
			if (left < 1 || p[i] >= REGS) return -1;
			if (m) x = reg(ctx, p[i]);
			i++;
			break;
		default:
			return -1;
		}
		if (d == PROG_DEPTH) return -1;
		s[d++] = x;
	}
	if (d != 1) return -1;
	*out = s[0];
	return 0;
}

int snd_prog_eval(const uint8_t *p, uint32_t len, const zx_machine *m, uint16_t anchor, const zx_regs *ctx, uint32_t *out)
{
	return m ? run(p, len, m, anchor, ctx, out) : -1;
}

static int valid(const uint8_t *p, uint32_t len)
{
	uint32_t v;

	return len && run(p, len, 0, 0, 0, &v) == 0;
}

/* The generic reader: each channel's period from its program with the site's registers r; the
 * stale rule on the frame-end pc; the timing program with r (the site's last edge) and with the
 * frame-end registers (now) gives since and row, or stale 1 if the note began after the edge. */
static void snd_learned_read(const zx_machine *m, const snd_engine *en, uint16_t anchor, uint32_t site, const zx_regs *r,
                             snd_b_read *out)
{
	const snd_learned_ext *x = en->data;
	uint32_t c, v, e_edge, e_now;
	zx_regs now;

	(void)site;
	for (c = 0; c < en->channels && c < SND_TONAL; c++) {
		if (snd_prog_eval(x->prog[c], x->prog_len[c], m, anchor, r, &v) == 0 && v) {
			out->v[c].period_q8 = v;
			out->v[c].level = x->level[c];
		}
	}
	zx_get_regs(m, &now);
	if (x->stale_len && (uint16_t)(now.pc - anchor - x->stale_from) < x->stale_len) {
		out->stale = 2;
	} else if (x->timing && snd_prog_eval(x->timing, x->timing_len, m, anchor, r, &e_edge) == 0 &&
	           snd_prog_eval(x->timing, x->timing_len, m, anchor, &now, &e_now) == 0) {
		if (e_now < e_edge) {
			out->stale = 1;
			out->since = out->row = e_now;
		} else {
			out->since = out->row = e_edge;
		}
	}
}

/* Strictly ascending offsets, each at most `below` - 1, `gap` apart at least. */
static int ascending(const uint8_t *p, uint32_t n, uint32_t gap, uint32_t below)
{
	uint32_t k, prev = 0;

	for (k = 0; k < n; k++) {
		const uint32_t o = rd16(p + 2u * k);
		if (o >= below || (k && o < prev + gap)) return 0;
		prev = o;
	}
	return 1;
}

/* One entry at e (size bytes, within the list) into *en and *x; 0 if any of it is not valid. */
static int entry(const uint8_t *e, uint32_t size, snd_engine *en, snd_learned_ext *x)
{
	const uint32_t channels = e[3], len = rd16(e + 12), n_wild = e[14], n_reloc = e[15], n_sites = e[16];
	const uint32_t flags = e[17], timing = e[19], timing_len = e[35];
	uint32_t c, k, at = ENTRY, progs = 0;
	uint64_t digest = 0;

	/* flags: 0, FLIPS or OBSERVED (not both, no other bit) */
	if (e[2] != 0 || channels < 1 || channels > SND_TONAL || flags > OBSERVED || timing > 1u) return 0;
	if (len < 1 || len > SND_LEARNED_LEN_MAX || n_sites > SND_LEARNED_SITES_MAX) return 0;
	if (timing ? !timing_len || timing_len > SND_PROG_MAX : timing_len != 0) return 0;
	for (c = 0; c < SND_TONAL; c++) {
		const uint32_t pl = e[32 + c];
		if (c < channels ? pl < 1 || pl > SND_PROG_MAX : pl != 0) return 0;
		progs += pl;
	}
	/* the arrays and the programs, within the entry */
	if (2u * n_wild + 2u * n_reloc + 4u * n_sites + 2u * channels + progs + timing_len > size - ENTRY) return 0;
	if (!ascending(e + at, n_wild, 1, len)) return 0;
	en->wild = (const uint16_t *)(const void *)(e + at);
	at += 2u * n_wild;
	if (!ascending(e + at, n_reloc, 2, len)) return 0;
	en->reloc = (const uint16_t *)(const void *)(e + at);
	at += 2u * n_reloc;
	for (k = 0; k < n_sites; k++) {
		const uint8_t *s = e + at + 4u * k;
		const uint32_t role = s[2], chan = s[3];
		if (rd16(s) >= len || role > SND_SITE_STOP) return 0;
		if (role == SND_SITE_TONAL && (flags == OBSERVED ? chan != 0 : chan >= channels && !(chan == SND_CHAN_ANY && flags))) return 0;
	}
	en->sites = (const snd_engine_site *)(const void *)(e + at);
	at += 4u * n_sites;
	en->pos = (const uint16_t *)(const void *)(e + at);
	at += 2u * channels;
	for (c = 0; c < SND_TONAL; c++) {
		x->prog[c] = c < channels ? e + at : 0;
		x->prog_len[c] = e[32 + c];
		x->level[c] = c < channels ? e[26 + c] : 0;
		en->role[c] = c < channels && e[29 + c] <= SND_ROLE_PAD ? e[29 + c] : SND_ROLE_AUTO;
		if (c < channels && !valid(e + at, e[32 + c])) return 0;
		at += e[32 + c];
	}
	x->timing = timing ? e + at : 0;
	x->timing_len = (uint8_t)timing_len;
	if (timing && !valid(e + at, timing_len)) return 0;
	x->stale_from = (uint16_t)rd16(e + 22);
	x->stale_len = (uint16_t)rd16(e + 24);
	for (k = 8; k-- > 0;)
		digest = digest << 8 | e[4 + k];
	en->name = "learned";
	en->digest = digest;
	en->len = (uint16_t)len;
	en->n_wild = (uint8_t)n_wild;
	en->n_reloc = (uint8_t)n_reloc;
	en->n_sites = (uint8_t)n_sites;
	en->channels = (uint8_t)channels;
	en->pass = (uint16_t)rd16(e + 20);
	en->read = snd_learned_read;
	en->drum = e[18];
	en->flips = (uint8_t)(flags & FLIPS);
	en->min_len = 0;
	en->data = x;
	en->strict = 1;
	en->observed = flags == OBSERVED;
	return 1;
}

/* A score's varint at e + *at (LEB128, at most 4 bytes, within size) into *v; 0 if not valid. */
static int varint(const uint8_t *e, uint32_t size, uint32_t *at, uint32_t *v)
{
	uint32_t i, x = 0;

	for (i = 0; i < 4u && *at < size; i++) {
		const uint32_t c = e[(*at)++];
		x |= (c & 0x7Fu) << (7u * i);
		if (!(c & 0x80u)) {
			*v = x;
			return 1;
		}
	}
	return 0;
}

/* Is the kind-2 record at e (size bytes, within the list) valid? 1-3 voices, 1 to
 * SND_SCORE_BUCKETS_MAX buckets, the fingerprint and every event within the record, each event's
 * time a varint and before the fingerprint's end, its voice below `voices`, no other flag bit,
 * and a note on's period at least 64 T and its level not 0. */
static int score_ok(const uint8_t *e, uint32_t size)
{
	const uint32_t voices = e[3], buckets = rd16(e + 12);
	uint32_t k, at = SCORE + buckets, u = 0, dt, vf;

	if (voices < 1u || voices > SND_TONAL || buckets < 1u || buckets > SND_SCORE_BUCKETS_MAX || at > size) return 0;
	for (k = rd16(e + 14); k > 0; k--) {
		if (!varint(e, size, &at, &dt)) return 0;
		u += dt;   /* (dt < 2^28, u below 2^24 before it: no wrap) */
		if (u >= buckets * UNITS || at >= size) return 0;
		vf = e[at++];
		if ((vf & 0x7Cu) || (vf & 3u) >= voices) return 0;
		if (!(vf & 0x80u)) {
			if (size - at < 4u || (rd16(e + at) | (uint32_t)e[at + 2] << 16) < 64u || !e[at + 3]) return 0;
			at += 4u;
		}
	}
	return 1;
}

/* The size of the list's entry at list + off, checked (within the list, a multiple of 4, at least
 * its kind's fixed part: ENTRY for kind 0, ROLES for kind 1, SCORE for kind 2), its kind in *kind;
 * 0 if not valid. */
static uint32_t entry_size(const uint8_t *list, uint32_t avail, uint32_t off, uint32_t *kind)
{
	uint32_t size;

	if (avail - off < 4u) return 0;
	size = rd16(list + off);
	*kind = list[off + 2];
	if (*kind > 2u || size < (*kind == 0u ? ENTRY : *kind == 1u ? ROLES : SCORE) || size % 4u || size > avail - off) return 0;
	return size;
}

int snd_learned_decode(const uint8_t *list, uint32_t avail, snd_engine *en, snd_learned_ext *ext, uint32_t max, uint32_t *n)
{
	uint32_t count, k, off = HEAD, work = 0, kind, size, m = 0, entries = 0, scores = 0;

	*n = 0;
	if (avail < HEAD || ((uintptr_t)list & 3u)) return ZX_ERR_FORMAT;
	count = list[0];
	for (k = 0; k < count; k++) {
		size = entry_size(list, avail, off, &kind);
		if (!size) return ZX_ERR_FORMAT;
		if (kind == 2u) {   /* a recorded score: checked whole */
			if (++scores > SND_SCORES_MAX || !score_ok(list + off, size)) return ZX_ERR_FORMAT;
			off += size;
			continue;
		}
		if (++entries > SND_LEARNED_MAX || entries > max) return ZX_ERR_FORMAT;
		if (kind) {   /* roles for a built-in entry: no engine, no work */
			if (list[off + 3] < 1u || list[off + 3] > SND_TONAL) return ZX_ERR_FORMAT;
		} else {
			if (!entry(list + off, size, &en[m], &ext[m])) return ZX_ERR_FORMAT;
			work += (uint32_t)en[m].n_sites * en[m].len;   /* at most 16 x 512 an entry */
			if (work > SND_LEARNED_WORK_MAX) return ZX_ERR_FORMAT;
			m++;
		}
		off += size;
	}
	*n = m;
	return ZX_OK;
}

void snd_learned_roles(const uint8_t *list, uint32_t avail, snd_engine *table, uint32_t n)
{
	uint32_t count, k, j, c, off = HEAD, kind, size;

	if (avail < HEAD) return;
	count = list[0];
	for (k = 0; k < count; k++) {
		size = entry_size(list, avail, off, &kind);
		if (!size) return;
		if (kind == 1u) {
			const uint8_t *e = list + off;
			uint64_t digest = 0;
			for (c = 8; c-- > 0;)
				digest = digest << 8 | e[4 + c];
			for (j = 0; j < n; j++) {
				snd_engine *en = &table[j];
				if (en->read == snd_learned_read || en->digest != digest) continue;
				for (c = 0; c < SND_TONAL; c++)
					en->role[c] = c < e[3] && c < en->channels && e[12 + c] <= SND_ROLE_PAD ? e[12 + c] : SND_ROLE_AUTO;
			}
		}
		off += size;
	}
}

/* ---- recorded scores --------------------------------------------------- */

/* FNV-1a 32 of the 32 bytes from site - 16: the code a score's site must sit in. */
static uint32_t code_at(const zx_machine *m, uint32_t site)
{
	uint32_t h = 0x811c9dc5u, i;

	for (i = 0; i < 32u; i++)
		h = (h ^ zx_peek(m, (uint16_t)(site - 16u + i))) * 0x01000193u;
	return h;
}

/* The frame's speaker edges (EAR changes, from level lvl before the first) at T-state t or after;
 * those before it in *before. */
static uint32_t ear_edges(const zx_edge *e, uint32_t n, uint32_t lvl, uint32_t t, uint32_t *before)
{
	uint32_t i, after = 0;

	*before = 0;
	for (i = 0; i < n; lvl = e[i++].level) {
		if (!((e[i].level ^ lvl) & 2u)) continue;
		if (e[i].tstate < t) ++*before;
		else after++;
	}
	return after;
}

/* The T-state of the frame's first speaker edge from site; NOWHERE if none. */
static uint32_t first_from(const zx_edge *e, uint32_t n, uint32_t lvl, uint32_t site)
{
	uint32_t i;

	for (i = 0; i < n; lvl = e[i++].level)
		if (((e[i].level ^ lvl) & 2u) && e[i].pc == site) return e[i].tstate;
	return NOWHERE;
}

/* The playing score's next event's time. */
static void next_time(snd_b_score *sc)
{
	uint32_t dt = 0;

	varint(sc->rec, rd16(sc->rec), &sc->at, &dt);
	sc->u += dt;
}

/* Event i of out (if i < SND_EVENTS); the events out has then. */
static uint32_t event(snd_frame *out, uint32_t i, uint32_t t, uint32_t voice, uint32_t period_q8, uint32_t level)
{
	if (i >= SND_EVENTS) return i;
	out->ev[i].t = t < ZX_FRAME_TSTATES ? t : ZX_FRAME_TSTATES - 1u;
	out->ev[i].voice = (uint8_t)voice;
	out->ev[i].period_q8 = period_q8;
	out->ev[i].level = (uint8_t)level;
	return i + 1u;
}

/* Frame f0 + rel, which the playing score plays: its events (into out unless the decision has the
 * frame confident), then the frame the score's. */
static void score_frame(snd_b *b, snd_frame *out, uint32_t rel)
{
	snd_b_score *sc = &b->sc;
	const uint8_t *r = sc->rec;
	const uint32_t d = rel * ZX_FRAME_TSTATES, buckets = rd16(r + 12), mine = !out->confident;
	uint32_t i = 0, k;

	if (mine)
		for (k = 0; k < SND_TONAL; k++) {
			out->tone[k].period_q8 = sc->v[k].period_q8;
			out->tone[k].level = sc->v[k].level;
		}
	while (sc->left && sc->u * 64u + sc->t0 < d + ZX_FRAME_TSTATES) {
		const uint32_t t = sc->u * 64u + sc->t0 - d, vf = r[sc->at++];
		uint32_t p = 0, level = 0;
		if (!(vf & 0x80u)) {
			p = (rd16(r + sc->at) | (uint32_t)r[sc->at + 2] << 16) << 8;
			level = r[sc->at + 3];
			sc->at += 4u;
		}
		k = vf & 3u;
		sc->v[k].period_q8 = p;
		sc->v[k].level = (uint8_t)level;
		if (mine) i = event(out, i, t, k, p, level);
		if (--sc->left) next_time(sc);
	}
	if (rel + 1u == buckets + (sc->t0 != 0))   /* the fingerprint ends in this frame */
		for (k = 0; k < SND_TONAL; k++) {
			if (!sc->v[k].period_q8) continue;
			if (mine) i = event(out, i, buckets * ZX_FRAME_TSTATES + sc->t0 - d, k, 0, 0);
			sc->v[k].period_q8 = 0;
			sc->v[k].level = 0;
		}
	if (!mine) return;   /* the table's */
	out->n_ev = (uint8_t)i;
	out->drum = 0;
	out->style = SND_STYLE_ENGINE;
	out->confident = 1;
	out->engine = (uint8_t)(b->n + sc->index);
	silence(b);
	for (k = 0; k < SND_TONAL; k++) {
		out->role[k] = k < r[3] && r[16 + k] <= SND_ROLE_PAD ? r[16 + k] : SND_ROLE_AUTO;
		b->tone[k].period_q8 = sc->v[k].period_q8;
		b->tone[k].level = sc->v[k].level;
	}
	b->engine = 0;
	b->held = 0;
}

/* The player (snd_b.scores), after the decision for frame j - SND_DELAY_FRAMES, frame j just
 * fed: following, arming, the decided frame (the rules at the top). */
static void play(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t lvl, snd_frame *out)
{
	snd_b_score *sc = &b->sc;
	const uint32_t j = b->frames - 1u;
	uint32_t k, v, lo, rel;

	if (sc->rec && sc->bucket < rd16(sc->rec + 12)) {   /* the bucket that ends in frame j */
		const uint32_t rec = sc->rec[SCORE + sc->bucket], tol = rec / 8u > 2u ? rec / 8u : 2u;
		const uint32_t hi = ear_edges(e, n, lvl, sc->t0, &lo);
		lo += sc->count;
		if (lo > 255u) lo = 255u;
		sc->bad = (uint8_t)(lo + tol < rec || lo > rec + tol ? sc->bad + 1u : 0u);
		sc->count = (uint16_t)hi;
		sc->bucket++;
		if (sc->bad == 3u) {   /* it stops: the frames not yet decided are not its */
			sc->bucket = (uint16_t)rd16(sc->rec + 12);
			if (sc->frames > j - SND_DELAY_FRAMES - sc->f0) sc->frames = j - SND_DELAY_FRAMES - sc->f0;
		}
	}
	for (k = 0; k < b->n_scores; k++) {
		const uint8_t *r = b->score[k];
		const uint32_t t = first_from(e, n, lvl, rd16(r + 4));
		if (t == NOWHERE) {
			if (b->quiet[k] < 0xFFFFu) b->quiet[k]++;
			continue;
		}
		if (!sc->rec && zm && b->quiet[k] >= rd16(r + 6) && code_at(zm, rd16(r + 4)) == rd32(r + 8)) {
			sc->rec = r;
			sc->index = (uint8_t)(k + 1u);
			sc->f0 = j;
			sc->t0 = t;
			sc->frames = rd16(r + 12) + (t != 0);
			sc->bucket = 0;
			sc->bad = 0;
			sc->count = (uint16_t)ear_edges(e, n, lvl, t, &lo);
			sc->left = (uint16_t)rd16(r + 14);
			sc->at = SCORE + rd16(r + 12);
			sc->u = 0;
			if (sc->left) next_time(sc);
			for (v = 0; v < SND_TONAL; v++) {
				sc->v[v].period_q8 = 0;
				sc->v[v].level = 0;
			}
		}
		b->quiet[k] = 0;
	}
	if (!sc->rec || b->frames <= SND_DELAY_FRAMES) return;
	rel = j - SND_DELAY_FRAMES - sc->f0;   /* the frame decided, from f0 */
	if ((int32_t)rel < 0) return;          /* (before T0's) */
	if (rel < sc->frames) score_frame(b, out, rel);
	if (rel + 1u >= sc->frames) sc->rec = 0;   /* its last frame decided (or none left) */
}

void snd_b_scores(snd_b *b, const uint8_t *list, uint32_t avail)
{
	uint32_t count, k, off = HEAD, kind, size;

	if (avail < HEAD) return;
	count = list[0];
	for (k = 0; k < count && b->n_scores < SND_SCORES_MAX; k++) {
		size = entry_size(list, avail, off, &kind);
		if (!size) return;
		if (kind == 2u) {
			b->quiet[b->n_scores] = 0xFFFFu;   /* (a game's first frames count as quiet) */
			b->score[b->n_scores++] = list + off;
			b->scores = play;
		}
		off += size;
	}
}
