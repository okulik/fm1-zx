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
 * kind-2 records, recorded scores, which the decoder checks whole (every size, count, bucket and
 * event) and the player below plays.
 *
 * A recorded score (docs/pack-format.md): an intro of intro_t T from its trigger edge (T0) and,
 * with loop_t not 0, a loop whose passes follow one another from T0 + intro_t, each loop_t long.
 * A section's (a pass's) times are T-states from its start; its buckets are frame-long spans from
 * its start, the whole ones alone stored and compared (the partial one at a section's end is
 * neither), and the fingerprint is the speaker edges (EAR level changes, from any OUT) of each,
 * capped at 255: a nibble stream, each section's first bucket an escape (nibble 8) and its count
 * (two nibbles, low first), each next the change from the bucket before (-7..+7) or an escape.
 * Events: a varint (64-T units since the section's event before, or its start), a byte (bits 0-1
 * the voice, bits 2-7 0 a note off, else a note on of level (b >> 2) x 4 + 3) and a note on's
 * index in the record's pitch table (each pitch a full period in T, 3 bytes). Method B's player,
 * once a frame after the decision (snd_b.scores), with frame j just fed:
 *  - following: each score followed (the candidates, then the score chosen) has each of its whole
 *    buckets that ends in frame j compared with its fingerprint, on its own sections' grids (the
 *    intro's from T0, each pass's from its start, the loop's buckets again for every pass): they
 *    agree when |live - recorded| <= max(2, recorded / 8). Three buckets in a row that disagree
 *    stop it; when none is left followed, the score stops: the frames from j - SND_DELAY_FRAMES
 *    on, not yet decided, are not its (the first of the three buckets began in frame
 *    j - SND_DELAY_FRAMES). A score with no loop ends its following with its intro; one with a
 *    loop is followed round it for as long as it agrees.
 *  - arming: while no score plays (from its trigger to its last frame decided), every score whose
 *    site makes a speaker edge in frame j, after at least `quiet` frames with none from it (a
 *    game's first frames count as quiet), and whose code (the FNV-1a 32 of the 32 bytes from
 *    site - 16) is the record's, is a candidate; T0 is the first candidate's site's first speaker
 *    edge in frame j, and another score joins only at that same edge (the same site).
 *    A score whose trigger holds while one plays does not interrupt it.
 *  - choosing: when frame f0's decision is due (frame j = f0 + SND_DELAY_FRAMES, its buckets
 *    compared so far), the candidates stopped by then are dropped; of the rest, the one with the
 *    fewest disagreeing buckets (the lowest index of those) plays and the others are dropped. It
 *    then stops by its own buckets alone (no other takes over). With none left, no score plays.
 *  - the decision for frame j - SND_DELAY_FRAMES, if the score plays it (from frame f0, T0's, to
 *    the one its intro ends in when it has no loop, or up to the frame before it stopped): its
 *    events whose times fall in the frame (each at its T-state in the frame); at each pass's start
 *    the voices sounding that the loop's first events (at the pass's start) do not restate
 *    released, the loop's events then from its first; with no loop, in the frame its intro ends,
 *    the voices still sounding released where it ends. Unless the decision has the frame
 *    confident (a table entry claims it: the table wins, its events stand), the frame becomes the
 *    score's: confident, its voices at the frame's start and its events (the first SND_EVENTS),
 *    engine n + 1 + its index (n the table's entries), its stored roles, B's voices the score's and
 *    nothing of a table entry's carried or held.
 * A frame after a score stops is decided as without it (the beeper's, unless the table claims
 * it): B's voices are silenced there and the mixer leaves FM.
 *
 * Built into flash, not SRAM, on the device (the reader runs only for frames a learned entry
 * claims; the player, a pass or two over a frame's edges per score followed, through
 * snd_b.scores). 32-bit arithmetic only (no 64-bit division); a score's times are kept from its
 * section's start and the frame's, so a loop followed for hours does not overflow them. */

#include "sound_internal.h"

#define PROG_DEPTH 8u
#define HEAD 4u       /* the list's header: count, 0, 0 (u16) */
#define ENTRY 36u     /* an entry's fixed part */
#define ROLES 16u     /* a kind-1 record (roles for a built-in entry), padded */
#define REGS 18u      /* register numbers 0-17 */
#define FLIPS 1u      /* an entry's flags byte: snd_engine.flips */
#define OBSERVED 2u   /* snd_engine.observed */
#define SCORE 32u     /* a kind-2 record's fixed part (a recorded score) */
#define ESC 8u        /* a score's fingerprint: the nibble before a count */
#define FRAME ((int32_t)ZX_FRAME_TSTATES)

_Static_assert(SND_SCORES_MAX <= 8u, "a bit per score in snd_b_score.live");

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

static uint32_t rd24(const uint8_t *p)
{
	return rd16(p) | (uint32_t)p[2] << 16;
}

/* A score's section: its length in T (0 the intro, 1 the loop), its whole buckets, its events. */
static uint32_t sec_len(const uint8_t *r, uint32_t sec)
{
	return rd32(r + 12u + 4u * sec);
}

static uint32_t sec_buckets(const uint8_t *r, uint32_t sec)
{
	return rd16(r + 20u + 2u * sec);
}

static uint32_t sec_events(const uint8_t *r, uint32_t sec)
{
	return rd16(r + 24u + 2u * sec);
}

/* Nibble i of the fingerprint at fp (the low nibble of a byte first). */
static uint32_t nibble(const uint8_t *fp, uint32_t i)
{
	return (uint32_t)(fp[i >> 1] >> ((i & 1u) * 4u)) & 15u;
}

/* The recorded count of the bucket at nibble *i of the fingerprint fp, the bucket before it
 * `prev`: an escape and the count, or the change from prev (-7..+7); *i moved past it. */
static uint32_t bucket_count(const uint8_t *fp, uint32_t *i, uint32_t prev)
{
	const uint32_t d = nibble(fp, (*i)++);
	uint32_t v;

	if (d != ESC) return prev + d - (d & 8u ? 16u : 0u);
	v = nibble(fp, *i) | nibble(fp, *i + 1u) << 4;
	*i += 2u;
	return v;
}

/* Is the fingerprint at fp (within `bytes` bytes) valid for ib intro and lb loop buckets? Each
 * section's first bucket an escape, every count 0-255, the stream within the bytes, a spare nibble
 * at its end 0; its bytes in *len. */
static int fp_ok(const uint8_t *fp, uint32_t bytes, uint32_t ib, uint32_t lb, uint32_t *len)
{
	uint32_t i = 0, s, k, d, v = 0;

	for (s = 0; s < 2u; s++)
		for (k = 0; k < (s ? lb : ib); k++) {
			if (i >= 2u * bytes) return 0;
			d = nibble(fp, i);
			if ((!k && d != ESC) || (d == ESC && i + 3u > 2u * bytes)) return 0;
			v = bucket_count(fp, &i, v);
			if (v > 255u) return 0;
		}
	if ((i & 1u) && nibble(fp, i)) return 0;
	*len = (i + 1u) / 2u;
	return 1;
}

/* Are a section's n events at e + *at (within size) valid? Each time a varint, inside the
 * section (below len T), its voice below `voices`, a note on's pitch index below np; *at moved
 * past them. */
static int events_ok(const uint8_t *e, uint32_t size, uint32_t *at, uint32_t n, uint32_t len, uint32_t voices, uint32_t np)
{
	const uint32_t units = (len + 63u) / 64u;   /* a time below len T: below this in 64-T units */
	uint32_t u = 0, dt, vb;

	for (; n > 0; n--) {
		if (!varint(e, size, at, &dt)) return 0;
		u += dt;   /* (dt < 2^28, u below 2^24 before it: no wrap) */
		if (u >= units || *at >= size) return 0;
		vb = e[(*at)++];
		if ((vb & 3u) >= voices) return 0;
		if ((vb >> 2) && (*at >= size || e[(*at)++] >= np)) return 0;
	}
	return 1;
}

/* Is the kind-2 record at e (size bytes, within the list) valid? 1-3 voices, an intro or a loop,
 * each section's whole buckets its length's (SND_SCORE_BUCKETS_MAX of both at most; a loop one at
 * least), 1-255 pitches of 64 T at least, the fingerprint (fp_ok) and every event (events_ok)
 * within the record. */
static int score_ok(const uint8_t *e, uint32_t size)
{
	const uint32_t voices = e[3], intro_t = sec_len(e, 0), loop_t = sec_len(e, 1), ib = sec_buckets(e, 0), lb = sec_buckets(e, 1);
	const uint32_t np = e[28];
	uint32_t k, at = SCORE + 3u * np, len;

	if (voices < 1u || voices > SND_TONAL || !np || at > size) return 0;
	if (ib != intro_t / ZX_FRAME_TSTATES || lb != loop_t / ZX_FRAME_TSTATES || ib + lb > SND_SCORE_BUCKETS_MAX) return 0;
	if ((!intro_t && !loop_t) || (loop_t && !lb)) return 0;
	for (k = 0; k < np; k++)
		if (rd24(e + SCORE + 3u * k) < 64u) return 0;
	if (!fp_ok(e + at, size - at, ib, lb, &len)) return 0;
	at += len;
	return events_ok(e, size, &at, sec_events(e, 0), intro_t, voices, np) && events_ok(e, size, &at, sec_events(e, 1), loop_t, voices, np);
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

/* The frame's speaker edges (EAR changes, from level lvl before the first) at T-states from a to
 * below b. */
static uint32_t span(const zx_edge *e, uint32_t n, uint32_t lvl, uint32_t a, uint32_t b)
{
	uint32_t i, c = 0;

	for (i = 0; i < n; lvl = e[i++].level)
		if (((e[i].level ^ lvl) & 2u) && e[i].tstate >= a && e[i].tstate < b) c++;
	return c;
}

/* The T-state of the frame's first speaker edge from site; NOWHERE if none. */
static uint32_t first_from(const zx_edge *e, uint32_t n, uint32_t lvl, uint32_t site)
{
	uint32_t i;

	for (i = 0; i < n; lvl = e[i++].level)
		if (((e[i].level ^ lvl) & 2u) && e[i].pc == site) return e[i].tstate;
	return NOWHERE;
}

/* The length (T) of f's current bucket: a whole bucket, or the partial one at its section's end
 * (0: none). */
static uint32_t seg(const uint8_t *r, const snd_b_follow *f)
{
	const uint32_t nb = sec_buckets(r, f->sec);

	return f->bucket < nb ? ZX_FRAME_TSTATES : sec_len(r, f->sec) - nb * ZX_FRAME_TSTATES;
}

/* f moved to its next bucket, its end that bucket's: the next of its section, else the loop's
 * first (its fingerprint's again for each pass); sec 2 past the intro of a score with no loop. */
static void advance(const uint8_t *r, snd_b_follow *f)
{
	uint32_t len;

	do {
		if (f->bucket < sec_buckets(r, f->sec)) {
			f->bucket++;
		} else if (!sec_len(r, 1)) {
			f->sec = 2;
			return;
		} else {
			if (!f->sec) f->loop_nib = f->nib;   /* the intro's buckets read: the loop's follow */
			f->sec = 1;
			f->nib = f->loop_nib;
			f->bucket = 0;
		}
		len = seg(r, f);
	} while (!len);
	f->end += len;
}

/* Score k followed from T0 (T-state t0 of the frame fed now). */
static void begin(snd_b *b, uint32_t k, uint32_t t0)
{
	snd_b_follow *f = &b->sc.fl[k];
	const uint8_t *r = b->score[k];

	f->sec = 0;
	f->bucket = 0;
	f->nib = 0;
	f->count = 0;
	f->prev = 0;
	f->run = 0;
	f->miss = 0;
	f->end = t0;
	if (seg(r, f)) f->end += seg(r, f);
	else advance(r, f);
}

/* Score k followed through the frame's edges from T-state `from` on: each of its whole buckets
 * that ends in the frame compared with its fingerprint; three in a row that disagree stop it. */
static void step(snd_b *b, uint32_t k, const zx_edge *e, uint32_t n, uint32_t lvl, uint32_t from)
{
	snd_b_follow *f = &b->sc.fl[k];
	const uint8_t *r = b->score[k];
	uint32_t live, rec, tol;

	while (f->sec < 2u && f->end <= ZX_FRAME_TSTATES) {
		if (f->bucket < sec_buckets(r, f->sec)) {   /* a whole bucket: compared */
			live = f->count + span(e, n, lvl, from, f->end);
			if (live > 255u) live = 255u;
			rec = bucket_count(r + SCORE + 3u * r[28], &f->nib, f->prev);
			f->prev = (uint8_t)rec;
			tol = rec / 8u > 2u ? rec / 8u : 2u;
			if (live + tol >= rec && live <= rec + tol) {
				f->run = 0;
			} else {
				if (f->miss < 255u) f->miss++;
				if (++f->run == 3u) {   /* it stops */
					b->sc.live &= (uint8_t)~(1u << k);
					return;
				}
			}
		}
		from = f->end;
		f->count = 0;
		advance(r, f);
	}
	if (f->sec < 2u) {
		live = f->count + span(e, n, lvl, from, ~0u);
		f->count = (uint16_t)(live > 255u ? 255u : live);
		f->end -= ZX_FRAME_TSTATES;
	}
}

/* The playing score's next event's time. */
static void next_time(snd_b_score *sc)
{
	uint32_t dt = 0;

	varint(sc->rec, rd16(sc->rec), &sc->at, &dt);
	sc->u += dt;
}

/* The playing score's section sec begins (its start already set): its events from offset at. */
static void section(snd_b_score *sc, uint32_t sec, uint32_t at)
{
	sc->sec = (uint8_t)sec;
	sc->at = at;
	sc->u = 0;
	sc->left = (uint16_t)sec_events(sc->rec, sec);
	if (sc->left) next_time(sc);
}

/* The voices a section's first events (n of them, from offset at) give a note on at its start. */
static uint32_t restated(const uint8_t *r, uint32_t at, uint32_t n)
{
	uint32_t mask = 0, dt, vb;

	for (; n > 0; n--) {
		if (!varint(r, rd16(r), &at, &dt) || dt) break;
		vb = r[at++];
		if (vb >> 2) {
			mask |= 1u << (vb & 3u);
			at++;
		}
	}
	return mask;
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

/* The playing score's sounding voices but those of mask released at T-state t of out's frame
 * (into out from event i if mine); the events out has then. */
static uint32_t release(snd_b_score *sc, snd_frame *out, uint32_t i, uint32_t t, uint32_t mask, uint32_t mine)
{
	uint32_t k;

	for (k = 0; k < SND_TONAL; k++) {
		if (!sc->v[k].period_q8 || (mask >> k & 1u)) continue;
		if (mine) i = event(out, i, t, k, 0, 0);
		sc->v[k].period_q8 = 0;
		sc->v[k].level = 0;
	}
	return i;
}

/* Frame f0 + rel, which the playing score plays: its events (into out unless the decision has the
 * frame confident), each pass of the loop that begins in it, then the frame the score's. */
static void score_frame(snd_b *b, snd_frame *out, uint32_t rel)
{
	snd_b_score *sc = &b->sc;
	const uint8_t *r = sc->rec;
	const uint32_t loop = sec_len(r, 1) != 0, mine = !out->confident;
	uint32_t i = 0, k;
	int32_t t;

	if (mine)
		for (k = 0; k < SND_TONAL; k++) {
			out->tone[k].period_q8 = sc->v[k].period_q8;
			out->tone[k].level = sc->v[k].level;
		}
	for (;;) {
		while (sc->left && (t = sc->start + (int32_t)(sc->u * 64u)) < FRAME) {
			const uint32_t vb = r[sc->at++];
			uint32_t p = 0, level = 0;
			if (vb >> 2) {
				p = rd24(r + SCORE + 3u * r[sc->at++]) << 8;
				level = (vb >> 2) * 4u + 3u;
			}
			k = vb & 3u;
			sc->v[k].period_q8 = p;
			sc->v[k].level = (uint8_t)level;
			if (mine) i = event(out, i, (uint32_t)t, k, p, level);
			if (--sc->left) next_time(sc);
		}
		t = sc->start + (int32_t)sec_len(r, sc->sec);   /* the section's end */
		if (!loop || t >= FRAME) break;
		/* a pass of the loop begins in this frame: the voices its first events do not restate
		 * released */
		if (!sc->sec) sc->loop_at = sc->at;   /* the intro's events read: the loop's follow */
		sc->start = t;
		section(sc, 1, sc->loop_at);
		i = release(sc, out, i, (uint32_t)t, restated(r, sc->loop_at, sc->left), mine);
	}
	if (!loop && rel + 1u == sc->frames)   /* the intro ends in this frame: the score's end */
		i = release(sc, out, i, (uint32_t)(sc->start + (int32_t)sec_len(r, 0)), 0, mine);
	sc->start -= FRAME;
	if (!mine) return;   /* the table's */
	out->n_ev = (uint8_t)i;
	out->drum = 0;
	out->style = SND_STYLE_ENGINE;
	out->confident = 1;
	out->engine = (uint8_t)(b->n + sc->index);
	silence(b);
	for (k = 0; k < SND_TONAL; k++) {
		out->role[k] = k < r[3] && r[29 + k] <= SND_ROLE_PAD ? r[29 + k] : SND_ROLE_AUTO;
		b->tone[k].period_q8 = sc->v[k].period_q8;
		b->tone[k].level = sc->v[k].level;
	}
	b->engine = 0;
	b->held = 0;
}

/* T0's frame decided: the candidate followed with the fewest disagreeing buckets (the lowest
 * index of those) becomes the score that plays, the others are dropped; 0 if none is left. */
static uint32_t choose(snd_b *b)
{
	snd_b_score *sc = &b->sc;
	uint32_t k, best = SND_SCORES_MAX, at, len = 0;
	const uint8_t *r;

	for (k = 0; k < b->n_scores; k++)
		if ((sc->live >> k & 1u) && (best == SND_SCORES_MAX || sc->fl[k].miss < sc->fl[best].miss)) best = k;
	if (best == SND_SCORES_MAX) {
		sc->rec = 0;
		return 0;
	}
	r = b->score[best];
	sc->rec = r;
	sc->index = (uint8_t)(best + 1u);
	sc->live = (uint8_t)(1u << best);
	sc->frames = sec_len(r, 1) ? ~0u : (sc->t0 + sec_len(r, 0) + ZX_FRAME_TSTATES - 1u) / ZX_FRAME_TSTATES;
	sc->start = (int32_t)sc->t0;
	at = SCORE + 3u * r[28];
	fp_ok(r + at, rd16(r) - at, sec_buckets(r, 0), sec_buckets(r, 1), &len);   /* (valid: its length) */
	section(sc, 0, at + len);
	for (k = 0; k < SND_TONAL; k++) {
		sc->v[k].period_q8 = 0;
		sc->v[k].level = 0;
	}
	return 1;
}

/* The player (snd_b.scores), after the decision for frame j - SND_DELAY_FRAMES, frame j just
 * fed: following, arming, the decided frame (the rules at the top). */
static void play(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t lvl, snd_frame *out)
{
	snd_b_score *sc = &b->sc;
	const uint32_t j = b->frames - 1u;
	uint32_t k, rel;

	if (sc->rec && sc->live) {   /* each score followed through frame j */
		for (k = 0; k < b->n_scores; k++)
			if (sc->live >> k & 1u) step(b, k, e, n, lvl, 0);
		/* none followed: the frames not yet decided are not the score's */
		if (!sc->live && sc->frames > j - SND_DELAY_FRAMES - sc->f0) sc->frames = j - SND_DELAY_FRAMES - sc->f0;
	}
	for (k = 0; k < b->n_scores; k++) {
		const uint8_t *r = b->score[k];
		const uint32_t t = first_from(e, n, lvl, rd16(r + 4));
		if (t == NOWHERE) {
			if (b->quiet[k] < 0xFFFFu) b->quiet[k]++;
			continue;
		}
		/* a candidate: while no score plays, or with those armed at the same edge */
		if (zm && b->quiet[k] >= rd16(r + 6) && (!sc->rec || (sc->f0 == j && sc->t0 == t)) && code_at(zm, rd16(r + 4)) == rd32(r + 8)) {
			if (!sc->rec) {
				sc->rec = r;
				sc->f0 = j;
				sc->t0 = t;
				sc->frames = ~0u;
				sc->live = 0;
			}
			sc->live |= (uint8_t)(1u << k);
			begin(b, k, t);
			step(b, k, e, n, lvl, t);
		}
		b->quiet[k] = 0;
	}
	if (!sc->rec || b->frames <= SND_DELAY_FRAMES) return;
	rel = j - SND_DELAY_FRAMES - sc->f0;   /* the frame decided, from f0 */
	if ((int32_t)rel < 0) return;          /* (before T0's) */
	if (!rel && !choose(b)) return;        /* T0's frame: one candidate plays */
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
