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

	/* Timing state (timing.c). All T-states are frame coordinates: 0 is INT. */
	Z80InsnClock clock;                       /* M-cycle positions within the instruction */
	uint32_t run_base;                        /* frame T-state at which the current z80_run began */
	uint32_t insn_t0;                         /* frame T-state at which the current instruction began */
	uint32_t wait;                            /* contention delay accumulated in the current instruction */
	uint8_t insn_open;                        /* a prefix has been fetched; the instruction continues */
	uint32_t io_t;                            /* frame T-state at which the current I/O cycle starts */

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

/* Delay for a contended access that starts at frame T-state `tstate`. */
uint32_t zx_contention(uint32_t tstate);

#ifdef ZX_TESTING
/* Host tests only. */
void zx_test_set_tstate(zx_machine *m, uint32_t t);   /* next instruction starts at frame T-state t */
uint32_t zx_test_step(zx_machine *m);                 /* runs one instruction; T-states incl. contention */
uint8_t zx_test_level(const zx_machine *m);           /* current beeper level */
#endif

#endif
