/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Public API of the ZX Spectrum 48K machine core. */

#ifndef ZX_H
#define ZX_H

#include <stdint.h>

#include <Z80.h>
#include <Z80InsnClock.h>

#define ZX_ROM_SIZE 16384u
#define ZX_RAM_SIZE 49152u
#define ZX_FRAME_TSTATES 69888u
#define ZX_LINE_TSTATES 224u
#define ZX_FRAME_LINES 312u
#define ZX_INT_TSTATES 32u
#define ZX_FIRST_CONTENDED 14335u
#define ZX_PAPER_FIRST_LINE 64u
#define ZX_SCREEN_WIDTH 256u
#define ZX_SCREEN_LINES 192u
#define ZX_MAX_EDGES 4096u

typedef struct {
	uint16_t af, bc, de, hl, af_, bc_, de_, hl_, ix, iy, sp, pc;
	uint8_t i, r, iff1, iff2, im;
} zx_regs;

typedef struct {
	uint32_t tstate;   /* frame T-state of the OUT's I/O cycle */
	uint8_t level;     /* (OUT 0xFE value >> 3) & 3: bit0 MIC, bit1 EAR */
} zx_edge;

typedef struct zx_machine {
	Z80 cpu;
	const uint8_t *rom;                       /* ZX_ROM_SIZE bytes, owned by the caller */
	uint32_t ram_words[ZX_RAM_SIZE / 4u];     /* 0x4000-0xFFFF, word-aligned storage */
	uint32_t frame_tstate;                    /* T-states already run into the next frame */
	uint32_t frame_count;
	uint8_t dirty[ZX_SCREEN_LINES / 8u];      /* bit (y & 7) of byte y >> 3: line y changed (screen.c) */

	/* Timing state (timing.c). All T-states are frame coordinates: 0 is INT. */
	Z80InsnClock clock;                       /* M-cycle positions within the instruction */
	uint32_t run_base;                        /* frame T-state at which the current z80_run began */
	uint32_t insn_t0;                         /* frame T-state at which the current instruction began */
	uint32_t wait;                            /* contention delay accumulated in the current instruction */
	uint8_t insn_open;                        /* a prefix has been fetched; the instruction continues */
	uint32_t io_t;                            /* frame T-state at which the current I/O cycle starts */

	/* Tape state (tape.c). */
	const uint8_t *tape;                      /* inserted .tap, owned by the caller; NULL when none */
	uint32_t tape_len, tape_pos;              /* tape_pos: offset of the next block */
	uint8_t auto_step;                        /* autostart progress, 0 = idle (tape.c) */
	uint8_t auto_timer;                       /* frames left in the current autostart step */

	/* ULA state (ula.c). */
	uint8_t keys[8];                          /* row r = A(8+r); bit set = pressed */
	uint8_t kempston;
	uint8_t ear_in;                           /* bit 4 of the last OUT 0xFE */
	uint8_t level;                            /* beeper level now, edges dropped or not */
	uint8_t border_col;                       /* colour applying from line border_filled on */
	uint16_t border_filled;                   /* border_lines[0..border_filled) are final */
	uint8_t border_lines[ZX_FRAME_LINES];
	uint16_t edge_count;
	uint8_t pend_edge_count, pend_border_count;   /* changes by the frame-overshooting instruction */
	zx_edge pend_edges[4];
	zx_edge pend_borders[4];                  /* .level holds the colour */
	zx_edge edges[ZX_MAX_EDGES];
} zx_machine;

/* rom: ZX_ROM_SIZE bytes that outlive m. Leaves m in the power-on state. */
void zx_init(zx_machine *m, const uint8_t *rom);

/* Runs the CPU until the frame's T-states reach ZX_FRAME_TSTATES; the
 * overshoot of the last instruction is carried into the next frame. */
void zx_run_frame(zx_machine *m);

void zx_get_regs(const zx_machine *m, zx_regs *r);
void zx_set_regs(zx_machine *m, const zx_regs *r);

uint8_t zx_peek(const zx_machine *m, uint16_t addr);
void zx_poke(zx_machine *m, uint16_t addr, uint8_t value);   /* ignores ROM */

uint32_t zx_frame_tstate(const zx_machine *m);
uint32_t zx_frame_count(const zx_machine *m);

void zx_set_keys(zx_machine *m, const uint8_t rows[8]);   /* row r = address line A(8+r); bit set = pressed */
void zx_set_kempston(zx_machine *m, uint8_t bits);        /* bit0 right, 1 left, 2 down, 3 up, 4 fire */

/* Beeper level changes of the frame run last, in T-state order. */
const zx_edge *zx_edges(const zx_machine *m, uint32_t *count);

/* Border colour (0-7) on frame line 0-311 of the frame run last. */
uint8_t zx_border_line(const zx_machine *m, unsigned frame_line);

/* Nonzero when screen line y (0-191) changed since the start of the frame run
 * last; every line is dirty after zx_init until the next frame run. */
int zx_line_dirty(const zx_machine *m, unsigned y);

/* Decodes screen line y to 256 palette indices: 0-7, plus 8 when BRIGHT.
 * FLASH swaps ink and paper while (frame_count / 16) is odd. */
void zx_screen_line(const zx_machine *m, unsigned y, uint8_t out[256]);

/* Delay for a contended access that starts at frame T-state `tstate`. */
uint32_t zx_contention(uint32_t tstate);

enum { ZX_OK = 0, ZX_ERR_FORMAT = -1, ZX_ERR_UNSUPPORTED = -2 };

/* Snapshot loading (snapshot.c). The whole file is validated before anything
 * changes, so on an error the machine is untouched. Only 48K snapshots load:
 * 128K ones give ZX_ERR_UNSUPPORTED, malformed or truncated ones ZX_ERR_FORMAT. */
int zx_load_sna(zx_machine *m, const uint8_t *data, uint32_t len);
int zx_load_z80(zx_machine *m, const uint8_t *data, uint32_t len);

/* Tape (tape.c). The whole .tap is validated by insert (every block's 2-byte
 * length must fit), then only the pointer is kept: `tap` must outlive its use.
 * While a tape is inserted, LD-BYTES (0x0556) is trapped and loads the next
 * block instantly; with none inserted the ROM routine runs. */
int zx_tape_insert(zx_machine *m, const uint8_t *tap, uint32_t len);
/* Resets the machine, then types LOAD "" + ENTER by itself over the frames
 * that follow, starting once the ROM's editor is waiting for a key. */
void zx_tape_autostart(zx_machine *m);

/* Beeper renderer (beeper.c). Independent of zx_machine: each frame the caller
 * passes that frame's edge list. A frame yields rate * 69888 / 3500000 samples,
 * kept exact as a reduced fraction num/den with a carried remainder. Each sample
 * is the time-weighted average of the level over its span (level -> amplitude
 * 0, 1000, 12000, 13000), then a one-pole DC blocker (pole 0.995, Q15).
 * Integer only; no 64-bit division. Precondition: after reduction the fraction
 * fits 32 bits (true for any rate up to ~13 MHz / den). */
typedef struct zx_beeper {
	uint32_t num, den;     /* samples per frame = num / den, reduced */
	uint32_t rem;          /* carried remainder, 0 <= rem < den */
	uint64_t sum;          /* level-time integral of the partly filled sample */
	uint64_t fill;         /* scaled time already in that sample */
	int32_t prev_x, prev_y;
	uint8_t level;         /* beeper level in effect */
	uint8_t dc_on;
} zx_beeper;

void zx_beeper_init(zx_beeper *b, uint32_t rate_num, uint32_t rate_den);   /* rate = num/den Hz */
uint32_t zx_beeper_frame_samples(const zx_beeper *b);                      /* samples the next render writes */
/* Renders one frame; `out` holds zx_beeper_frame_samples() entries. Edge T-states
 * must ascend and are clamped to the frame. Returns the sample count. */
uint32_t zx_beeper_render(zx_beeper *b, const zx_edge *edges, uint32_t count, int16_t *out);

#ifdef ZX_TESTING
/* Host tests only. */
void zx_test_set_tstate(zx_machine *m, uint32_t t);   /* next instruction starts at frame T-state t */
uint32_t zx_test_step(zx_machine *m);                 /* runs one instruction; T-states incl. contention */
uint8_t zx_test_level(const zx_machine *m);           /* current beeper level */
void zx_beeper_test_dc_block(zx_beeper *b, int on);   /* DC blocker on (default) or off */
#endif

#endif
