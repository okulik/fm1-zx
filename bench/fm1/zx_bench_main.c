/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/*
 * Bare-metal benchmark: runs FRAMES emulated frames, then writes a result block
 * at RESULT_ADDR: [0] magic (written last), [1] frames run, [2] FNV-1a-32 of the
 * screen memory 0x4000-0x5AFF, [3] final PC. The emulator reads it back.
 */

#include <stdint.h>

#include "zx.h"

#define FRAMES 150u
#define RESULT_ADDR 0x01C80000u
#define RESULT_MAGIC 0x5A58424Eu

extern const uint8_t zx_rom_data[];
extern const uint32_t zx_rom_data_size;

extern uint32_t _etext, _sdata, _edata, _sbss, _ebss;

static zx_machine machine;
static uint8_t rom_ram[ZX_ROM_SIZE];

/* Marks the start and end of the timed region for the emulator's trace. */
__attribute__((noinline)) void bench_mark(void)
{
    __asm__ volatile("" ::: "memory");
}

int main(void)
{
    const uint32_t *src = &_etext;
    for (uint32_t *p = &_sdata; p < &_edata;) {
        *p++ = *src++;
    }
    for (uint32_t *p = &_sbss; p < &_ebss;) {
        *p++ = 0;
    }

    const uint8_t *rom = zx_rom_data;
    for (uint32_t i = 0; i < ZX_ROM_SIZE; i++) {
        rom_ram[i] = rom[i];
    }

    zx_init(&machine, rom_ram);

    bench_mark();
    for (uint32_t i = 0; i < FRAMES; i++) {
        zx_run_frame(&machine);
    }
    bench_mark();

    uint32_t hash = 2166136261u;
    for (uint32_t a = 0x4000u; a <= 0x5AFFu; a++) {
        hash = (hash ^ zx_peek(&machine, (uint16_t)a)) * 16777619u;
    }
    zx_regs regs;
    zx_get_regs(&machine, &regs);

    volatile uint32_t *result = (volatile uint32_t *)RESULT_ADDR;
    result[1] = zx_frame_count(&machine);
    result[2] = hash;
    result[3] = regs.pc;
    result[0] = RESULT_MAGIC;

    for (;;) {
    }
}
