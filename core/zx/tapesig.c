/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The tape signal player (see tapesig.h). Host build only.
 *
 * Blocks are read from the data in place: insert checks the whole file once, so play can
 * trust every length. Play is a sequence of entries (a pulse, a pause, a stop), taken one at
 * a time as the machine's T-states pass. */

#include <stddef.h>
#include <string.h>

#include "tapesig.h"

#define TZX_HEADER 10u
#define MS_TSTATES 3500u
#define TAP_PAUSE_MS 1000u
#define STD_PILOT 2168u
#define STD_PILOT_HEADER 8063u
#define STD_PILOT_DATA 3223u
#define STD_SYNC1 667u
#define STD_SYNC2 735u
#define STD_ZERO 855u
#define STD_ONE 1710u
/* Block visits a whole play may make, loops unrolled: bounds the work of blocks that take no
 * time (texts, groups, empty loop bodies), so a hostile file cannot hang a port read. */
#define MAX_VISITS (1u << 24)

enum { PH_PILOT, PH_SYNC1, PH_SYNC2, PH_SEQ, PH_DATA, PH_PAUSE, PH_END };
enum { K_PLAY, K_SKIP, K_LOOP, K_LOOP_END };
enum { E_PULSE, E_PAUSE, E_STOP, E_END };

typedef struct {
	uint8_t kind;             /* K_* */
	uint32_t next;            /* offset of the block after this one */
	uint32_t count;           /* K_LOOP: passes */
	/* K_PLAY: as the fields of zx_tapesig */
	uint8_t stop;
	uint16_t pilot_len, sync1, sync2, zero, one;
	uint32_t pilot_n, seq_off, seq_n, data_off, data_bits, pause;
} block;

static uint32_t w16(const uint8_t *p)
{
	return (uint32_t)p[0] | ((uint32_t)p[1] << 8);
}

static uint32_t w24(const uint8_t *p)
{
	return w16(p) | ((uint32_t)p[2] << 16);
}

static uint32_t w32(const uint8_t *p)
{
	return w24(p) | ((uint32_t)p[3] << 24);
}

/* A standard-speed block of n bytes at off (flag, data, checksum). */
static void standard(block *b, const uint8_t *d, uint32_t off, uint32_t n, uint32_t pause_ms)
{
	b->kind = K_PLAY;
	b->pilot_len = STD_PILOT;
	b->pilot_n = n && d[off] < 128u ? STD_PILOT_HEADER : STD_PILOT_DATA;
	b->sync1 = STD_SYNC1;
	b->sync2 = STD_SYNC2;
	b->zero = STD_ZERO;
	b->one = STD_ONE;
	b->data_off = off;
	b->data_bits = n * 8u;
	b->pause = pause_ms * MS_TSTATES;
}

/* Data bits of a 0x11 or 0x14 block: n bytes, the last one's top `last` bits. Zero-length
 * bits and a `last` outside 1-8 are malformed (with n = 0 there are no bits to check). */
static int data_bits(block *b, uint32_t off, uint32_t n, uint32_t last, uint32_t zero, uint32_t one)
{
	if (n == 0) return ZX_OK;
	if (last == 0 || last > 8u || zero == 0 || one == 0) return ZX_ERR_FORMAT;
	b->zero = (uint16_t)zero;
	b->one = (uint16_t)one;
	b->data_off = off;
	b->data_bits = (n - 1u) * 8u + last;
	return ZX_OK;
}

/* Reads and checks the TZX block at off (< len) into b. */
static int tzx_block(const uint8_t *d, uint32_t len, uint32_t off, block *b)
{
	uint32_t p = off + 1u, avail = len - p, size, n;

	memset(b, 0, sizeof *b);
	b->kind = K_PLAY;
#define NEED(k) do { if (avail < (uint32_t)(k)) return ZX_ERR_FORMAT; } while (0)
	switch (d[off]) {
	case 0x10:
		NEED(4);
		n = w16(d + p + 2u);
		size = 4u + n;
		NEED(size);
		standard(b, d, p + 4u, n, w16(d + p));
		break;
	case 0x11:
		NEED(0x12);
		n = w24(d + p + 0x0Fu);
		NEED(0x12u + n);
		size = 0x12u + n;
		b->pilot_len = (uint16_t)w16(d + p);
		b->sync1 = (uint16_t)w16(d + p + 2u);
		b->sync2 = (uint16_t)w16(d + p + 4u);
		b->pilot_n = w16(d + p + 0x0Au);
		b->pause = w16(d + p + 0x0Du) * MS_TSTATES;
		if ((b->pilot_n && !b->pilot_len) || !b->sync1 || !b->sync2) return ZX_ERR_FORMAT;
		if (data_bits(b, p + 0x12u, n, d[p + 0x0Cu], w16(d + p + 6u), w16(d + p + 8u))) return ZX_ERR_FORMAT;
		break;
	case 0x12:
		NEED(4);
		size = 4u;
		b->pilot_len = (uint16_t)w16(d + p);
		b->pilot_n = w16(d + p + 2u);
		if (!b->pilot_len || !b->pilot_n) return ZX_ERR_FORMAT;
		break;
	case 0x13:
		NEED(1);
		n = d[p];
		size = 1u + 2u * n;
		if (n == 0) return ZX_ERR_FORMAT;
		NEED(size);
		for (b->seq_n = 0; b->seq_n < n; b->seq_n++)
			if (!w16(d + p + 1u + 2u * b->seq_n)) return ZX_ERR_FORMAT;
		b->seq_off = p + 1u;
		break;
	case 0x14:
		NEED(10);
		n = w24(d + p + 7u);
		NEED(10u + n);
		size = 10u + n;
		b->pause = w16(d + p + 5u) * MS_TSTATES;
		if (data_bits(b, p + 10u, n, d[p + 4u], w16(d + p), w16(d + p + 2u))) return ZX_ERR_FORMAT;
		break;
	case 0x20:
		NEED(2);
		size = 2u;
		b->pause = w16(d + p) * MS_TSTATES;
		b->stop = b->pause == 0;
		break;
	case 0x24:
		NEED(2);
		size = 2u;
		b->kind = K_LOOP;
		b->count = w16(d + p);
		if (b->count == 0) return ZX_ERR_FORMAT;
		break;
	case 0x25:
		size = 0;
		b->kind = K_LOOP_END;
		break;
	/* No signal: skipped by their lengths. */
	case 0x21: case 0x30:   /* group start, text: a length byte and the text */
		NEED(1);
		size = 1u + d[p];
		b->kind = K_SKIP;
		break;
	case 0x22: case 0x27:   /* group end, return from sequence */
		size = 0;
		b->kind = K_SKIP;
		break;
	case 0x23:              /* jump: a word */
		size = 2u;
		b->kind = K_SKIP;
		break;
	case 0x26:              /* call sequence: a count, then that many words */
		NEED(2);
		size = 2u + 2u * w16(d + p);
		b->kind = K_SKIP;
		break;
	case 0x28: case 0x32:   /* select block, archive info: a length word and the content */
		NEED(2);
		size = 2u + w16(d + p);
		b->kind = K_SKIP;
		break;
	case 0x2A:              /* stop the tape in 48K mode: a 4-byte length (0) */
		size = 4u;
		b->kind = K_SKIP;
		break;
	case 0x2B:              /* signal level: a 4-byte length and the level */
		size = 5u;
		b->kind = K_SKIP;
		break;
	case 0x31:              /* message: a time byte, a length byte and the text */
		NEED(2);
		size = 2u + d[p + 1u];
		b->kind = K_SKIP;
		break;
	case 0x33:              /* hardware type: a count, then 3 bytes each */
		NEED(1);
		size = 1u + 3u * d[p];
		b->kind = K_SKIP;
		break;
	case 0x34:              /* emulation info */
		size = 8u;
		b->kind = K_SKIP;
		break;
	case 0x35:              /* custom info: a 16-byte name and a 4-byte length */
		NEED(0x14);
		n = w32(d + p + 0x10u);
		if (n > avail - 0x14u) return ZX_ERR_FORMAT;
		size = 0x14u + n;
		b->kind = K_SKIP;
		break;
	case 0x5A:              /* glue */
		size = 9u;
		b->kind = K_SKIP;
		break;
	default:                /* 0x15 direct recording, 0x18 CSW, 0x19 generalized data, unknown */
		return ZX_ERR_UNSUPPORTED;
	}
	NEED(size);
#undef NEED
	b->next = p + size;
	return ZX_OK;
}

/* Reads the .tap block at off (< len) into b. */
static int tap_block(const uint8_t *d, uint32_t len, uint32_t off, block *b)
{
	uint32_t n;

	memset(b, 0, sizeof *b);
	if (len - off < 2u) return ZX_ERR_FORMAT;
	n = w16(d + off);
	if (len - off - 2u < n) return ZX_ERR_FORMAT;
	standard(b, d, off + 2u, n, TAP_PAUSE_MS);
	b->next = off + 2u + n;
	return ZX_OK;
}

static int read_block(const uint8_t *d, uint32_t len, int tzx, uint32_t off, block *b)
{
	return tzx ? tzx_block(d, len, off, b) : tap_block(d, len, off, b);
}

/* Checks the whole file: every block, loops neither nested nor left open nor stray, and the
 * unrolled block visits bounded. */
static int check(const uint8_t *d, uint32_t len, int tzx, uint8_t *bad_id)
{
	uint32_t off = tzx ? TZX_HEADER : 0, loop_count = 0, body = 0, loop_id_off = 0;
	uint64_t visits = 0;
	int in_loop = 0;
	block b;

	while (off < len) {
		int r = read_block(d, len, tzx, off, &b);

		if (tzx) *bad_id = d[off];
		if (r != ZX_OK) return r;
		if (b.kind == K_LOOP) {
			if (in_loop) return ZX_ERR_FORMAT;
			in_loop = 1;
			loop_count = b.count;
			loop_id_off = off;
			body = 0;
		} else if (b.kind == K_LOOP_END) {
			if (!in_loop) return ZX_ERR_FORMAT;
			in_loop = 0;
			visits += (uint64_t)loop_count * (body + 1u);
		} else if (in_loop) {
			body++;
		} else {
			visits++;
		}
		if (visits + body > MAX_VISITS) return ZX_ERR_FORMAT;
		off = b.next;
	}
	if (in_loop) {
		*bad_id = d[loop_id_off];
		return ZX_ERR_FORMAT;
	}
	*bad_id = 0;
	return ZX_OK;
}

/* Moves to the next block that plays, through skips and loops; 0 at the end of the data. */
static int next_block(zx_tapesig *s)
{
	block b;

	while (s->next < s->len) {
		(void)read_block(s->data, s->len, s->tzx, s->next, &b);   /* checked at insert */
		switch (b.kind) {
		case K_LOOP:
			s->loop_start = b.next;
			s->loop_left = (uint16_t)b.count;
			s->next = b.next;
			break;
		case K_LOOP_END:
			if (s->loop_left > 1u) {
				s->loop_left--;
				s->next = s->loop_start;
			} else {
				s->next = b.next;
			}
			break;
		case K_SKIP:
			s->next = b.next;
			break;
		default:
			s->next = b.next;
			s->stop = b.stop;
			s->pilot_len = b.pilot_len;
			s->pilot_n = b.pilot_n;
			s->sync1 = b.sync1;
			s->sync2 = b.sync2;
			s->seq_off = b.seq_off;
			s->seq_n = b.seq_n;
			s->zero = b.zero;
			s->one = b.one;
			s->data_off = b.data_off;
			s->data_bits = b.data_bits;
			s->pause = b.pause;
			s->phase = PH_PILOT;
			s->index = 0;
			return 1;
		}
	}
	return 0;
}

/* The next entry of play and its length in T-states (E_PULSE, E_PAUSE). */
static int next_entry(zx_tapesig *s, uint32_t *t)
{
	for (;;) {
		switch (s->phase) {
		case PH_PILOT:
			if (s->index < s->pilot_n) {
				s->index++;
				*t = s->pilot_len;
				return E_PULSE;
			}
			s->phase = PH_SYNC1;
			break;
		case PH_SYNC1:
			s->phase = PH_SYNC2;
			if (s->sync1) {
				*t = s->sync1;
				return E_PULSE;
			}
			break;
		case PH_SYNC2:
			s->phase = PH_SEQ;
			s->index = 0;
			if (s->sync2) {
				*t = s->sync2;
				return E_PULSE;
			}
			break;
		case PH_SEQ:
			if (s->index < s->seq_n) {
				*t = w16(s->data + s->seq_off + 2u * s->index);
				s->index++;
				return E_PULSE;
			}
			s->phase = PH_DATA;
			s->index = 0;
			break;
		case PH_DATA:
			if (s->index < 2u * s->data_bits) {
				uint32_t bit = s->index >> 1;
				uint8_t byte = s->data[s->data_off + (bit >> 3)];

				s->index++;
				*t = (byte & (0x80u >> (bit & 7u))) ? s->one : s->zero;
				return E_PULSE;
			}
			s->phase = PH_PAUSE;
			break;
		case PH_PAUSE:
			s->phase = PH_END;
			if (s->stop) return E_STOP;
			if (s->pause) {
				*t = s->pause;
				return E_PAUSE;
			}
			break;
		default:
			if (!next_block(s)) return E_END;
			break;
		}
	}
}

/* Plays up to the absolute T-state target. Every entry lasts at least 1 T, so the work is
 * bounded by the T-states played (and the visits check bounds the skips). */
static void advance(zx_tapesig *s, uint64_t target)
{
	if (target < s->pos) {   /* the machine's clock restarted (a reset): play on from here */
		s->pos = target;
		return;
	}
	while (s->pos < target) {
		uint64_t step;

		if (s->remaining == 0) {
			uint32_t t = 0;

			if (s->done || s->stopped) break;
			switch (next_entry(s, &t)) {
			case E_PULSE:
				s->remaining = t;
				s->in_pulse = 1;
				break;
			case E_PAUSE:
				/* a high level stays for 1 ms first, so the loader sees the last edge */
				s->in_pulse = 0;
				s->remaining = s->level && t > MS_TSTATES ? MS_TSTATES : t;
				s->pause_rest = t - s->remaining;
				break;
			case E_STOP:
				if (s->level) {
					s->in_pulse = 0;
					s->remaining = MS_TSTATES;
					s->stop_hold = 1;
				} else {
					s->stopped = 1;
				}
				break;
			default:
				s->done = 1;
				break;
			}
			continue;
		}
		step = target - s->pos;
		if (step < s->remaining) {
			s->remaining -= (uint32_t)step;
			s->pos = target;
		} else {
			s->pos += s->remaining;
			s->remaining = 0;
			if (s->in_pulse) {
				s->level ^= 1u;
			} else {   /* the end of a pause's or stop's 1 ms at the old level, or of a pause */
				s->level = 0;
				s->remaining = s->pause_rest;
				s->pause_rest = 0;
				s->stopped = s->stop_hold;
				s->stop_hold = 0;
			}
		}
	}
	s->pos = target;
}

static uint64_t now(const zx_machine *m, uint32_t frame_t)
{
	return (uint64_t)m->frame_count * ZX_FRAME_TSTATES + frame_t;
}

static uint8_t tapesig_ear(zx_machine *m, uint32_t frame_t)
{
	zx_tapesig *s = m->tape_sig;

	advance(s, now(m, frame_t));
	return s->level;
}

uint32_t zx_tapesig_size(void)
{
	return (uint32_t)sizeof(zx_tapesig);
}

int zx_tapesig_insert(zx_machine *m, zx_tapesig *s, const uint8_t *data, uint32_t len, uint8_t *bad_id)
{
	static const uint8_t sig[8] = { 'Z', 'X', 'T', 'a', 'p', 'e', '!', 0x1A };
	uint8_t id = 0;
	int tzx = len >= sizeof sig && memcmp(data, sig, sizeof sig) == 0;
	int r;

	if (len == 0 || (tzx && len < TZX_HEADER)) {
		r = ZX_ERR_FORMAT;
	} else {
		r = check(data, len, tzx, &id);
	}
	if (bad_id) *bad_id = id;
	if (r != ZX_OK) return r;

	memset(s, 0, sizeof *s);
	s->data = data;
	s->len = len;
	s->tzx = (uint8_t)tzx;
	s->next = tzx ? TZX_HEADER : 0;
	s->phase = PH_END;
	s->pos = now(m, m->frame_tstate);
	m->tape_sig = s;
	m->tape_ear = tapesig_ear;
	return ZX_OK;
}

void zx_tapesig_eject(zx_machine *m)
{
	m->tape_ear = NULL;
	m->tape_sig = NULL;
}

void zx_tapesig_frame(zx_machine *m)
{
	if (m->tape_sig) advance(m->tape_sig, now(m, m->frame_tstate));
}

int zx_tapesig_done(const zx_machine *m)
{
	const zx_tapesig *s = m->tape_sig;

	return s ? s->done : 0;
}

int zx_tapesig_stopped(const zx_machine *m)
{
	const zx_tapesig *s = m->tape_sig;

	return s ? s->stopped : 0;
}

void zx_tapesig_play(zx_machine *m)
{
	zx_tapesig *s = m->tape_sig;

	if (s) s->stopped = 0;
}

#ifdef ZX_TESTING
uint32_t zx_tapesig_test_pulses(zx_tapesig *s, uint32_t *out, uint32_t max)
{
	uint32_t n = 0, t = 0;

	while (n < max) {
		int e = next_entry(s, &t);

		if (e == E_END) break;
		out[n++] = e == E_STOP ? 0 : t;
	}
	return n;
}
#endif
