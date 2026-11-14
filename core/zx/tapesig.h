/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The tape signal player (tapesig.c): plays a TZX or .tap as the pulses a cassette gives,
 * into bit 6 of port FEh reads, so that loaders of their own (turbo and custom ones) load.
 * Host build only: the firmware leaves tapesig.c out, and its m->tape_ear stays NULL.
 *
 * Played: TZX blocks 0x10 (standard speed), 0x11 (turbo), 0x12 (pure tone), 0x13 (pulse
 * sequence), 0x14 (pure data), 0x20 (pause; 0 stops the tape) and the loops 0x24/0x25. The
 * blocks that carry no signal (groups, texts, archive info, jumps, calls...) are skipped by
 * their lengths. Any other block makes the tape not loadable. A .tap block plays as a 0x10
 * block with a 1000 ms pause. Timings are TZX 1.20's: standard pilot 2168 T, 8063 pulses
 * for a header (flag < 128) and 3223 for data, syncs 667 and 735 T, bits 855 and 1710 T,
 * two pulses per bit, most significant bit first; 1 ms is 3500 T.
 *
 * The level starts low at insert and flips at the end of every pulse. A pause holds it low
 * for its length, after 1 ms at the old level if that was high (as TZX 1.20 asks, so the last
 * edge is finished: a loader times each edge from the one before it). A data block's pause
 * of 0 is no pause (the next block follows at once); a 0x20 pause of 0 is a stop: the level
 * falls in the same way and is held low until zx_tapesig_play. */

#ifndef ZX_TAPESIG_H
#define ZX_TAPESIG_H

#include <stdint.h>

#include "zx.h"

/* The player's state. The caller allocates it (zx_tapesig_size bytes) and keeps it, and the
 * data, alive while inserted; zx_tapesig_insert fills it in. */
typedef struct zx_tapesig {
	const uint8_t *data;
	uint32_t len;
	uint32_t next;            /* offset of the block after the current one */
	uint32_t loop_start;      /* offset of the first block of the open loop */
	uint16_t loop_left;       /* passes of the open loop still to play, this one included */
	uint8_t tzx;              /* data is a TZX (else a .tap) */

	/* The current block, as the phases it plays: pilot, syncs, pulse sequence, data, pause. */
	uint8_t stop;             /* ends with a stop (0x20 pause 0) */
	uint16_t pilot_len, sync1, sync2, zero, one;   /* a sync of 0 is none */
	uint32_t pilot_n;         /* pilot pulses */
	uint32_t seq_off, seq_n;  /* 0x13: offset of the pulse lengths, how many */
	uint32_t data_off, data_bits;
	uint32_t pause;           /* T-states of silence after the block, 0 = none */

	/* Where play has got to. */
	uint8_t phase;            /* tapesig.c's PH_* */
	uint32_t index;           /* pulses (pilot, sequence) or half-bits (data) done in the phase */
	uint32_t remaining;       /* T-states left of the current pulse or pause */
	uint8_t in_pulse;         /* the current entry is a pulse (it flips the level at its end) */
	uint32_t pause_rest;      /* T-states of the pause still to come after its first 1 ms */
	uint8_t stop_hold;        /* the 1 ms before a stop: the tape stops at its end */
	uint8_t level;
	uint8_t done;             /* past the last pulse and pause of the last block */
	uint8_t stopped;          /* held by a stop */
	uint64_t pos;             /* absolute T-state reached: frame_count * 69888 + frame T-state */
} zx_tapesig;

uint32_t zx_tapesig_size(void);

/* data: a TZX (it starts "ZXTape!\x1A") or a .tap, checked whole before anything changes:
 * every length and count must fit the data and be non-zero where it counts pulses or gives
 * their length. Returns ZX_OK, ZX_ERR_FORMAT (malformed or cut short) or ZX_ERR_UNSUPPORTED
 * (a block that is not loadable). On an error *bad_id is the id of the TZX block at fault, 0
 * for the TZX header or a .tap, and the machine is untouched. On success the signal starts at
 * the machine's current T-state with the level low: it sets m->tape_ear and m->tape_sig.
 *
 * The rest of the core knows only those two fields (the firmware's port read tests the
 * first): zx_init, and so zx_tape_autostart, clears them, and a snapshot load leaves them.
 * zx-sys keeps a signal across an autostart (it puts both back) and ejects it on a snapshot
 * load. Should the machine's clock go back (a reset), play goes on from where it was. */
int zx_tapesig_insert(zx_machine *m, zx_tapesig *s, const uint8_t *data, uint32_t len, uint8_t *bad_id);
void zx_tapesig_eject(zx_machine *m);
/* Call after every zx_run_frame while a signal is inserted: advances it to the frame end. */
void zx_tapesig_frame(zx_machine *m);
/* 1 once past the last pulse (and pause) of the last block; 0 with no signal inserted. */
int zx_tapesig_done(const zx_machine *m);
/* 1 while held by a stop (a 0x20 pause 0); 0 with no signal inserted. */
int zx_tapesig_stopped(const zx_machine *m);
/* Starts the tape again after a stop: the next block starts now. */
void zx_tapesig_play(zx_machine *m);

#ifdef ZX_TESTING
/* Host tests only: plays on from where s is, writing the next entries to out (at most max):
 * a pulse's length, a pause's length as one entry, and 0 for a stop (play goes on past it).
 * Returns the number written; fewer than max means the tape ended. */
uint32_t zx_tapesig_test_pulses(zx_tapesig *s, uint32_t *out, uint32_t max);
#endif

#endif
