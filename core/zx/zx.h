/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Public API of the ZX Spectrum 48K machine core. */

#ifndef ZX_H
#define ZX_H

#include <stdint.h>

#include <Z80.h>

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

typedef struct {
	uint16_t af, bc, de, hl, af_, bc_, de_, hl_, ix, iy, sp, pc;
	uint8_t i, r, iff1, iff2, im;
} zx_regs;

typedef struct zx_machine {
	Z80 cpu;
	const uint8_t *rom;                       /* ZX_ROM_SIZE bytes, owned by the caller */
	uint32_t ram_words[ZX_RAM_SIZE / 4u];     /* 0x4000-0xFFFF, word-aligned storage */
	uint32_t frame_tstate;                    /* T-states already run into the next frame */
	uint32_t frame_count;
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

#endif
