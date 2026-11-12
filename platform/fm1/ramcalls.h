/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* XIP code (the image in flash) and RAMTEXT code (SRAM) are out of each other's branch reach, so
 * XIP reaches RAMTEXT through this one table of function pointers (main.c fills it in). It is
 * external, so the compiler cannot turn the indirect calls back into direct ones; check_reach.py
 * fails the build on any direct call between the two. */
#ifndef FMZX_RAMCALLS_H
#define FMZX_RAMCALLS_H

#include <stdint.h>
#include "zx.h"

struct ram_calls {
    void (*init)(zx_machine *, const uint8_t *);
    int (*load_sna)(zx_machine *, const uint8_t *, uint32_t);
    int (*load_z80)(zx_machine *, const uint8_t *, uint32_t);
    int (*tape_insert)(zx_machine *, const uint8_t *, uint32_t);
    void (*tape_autostart)(zx_machine *);
    void (*beeper_init)(zx_beeper *, uint32_t, uint32_t);
    void (*compose_zx)(const zx_machine *, unsigned, unsigned, uint8_t *);
    void (*game_step)(void);    /* main.c: one Spectrum frame */
    void (*game_release)(void); /* main.c: all Spectrum keys and Kempston up */
    void (*lcd_pump)(void);     /* lcd.c */
};
extern struct ram_calls fmzx_ram;

/* lcd.c's RAMTEXT pump, for the table. */
void lcd_pump_ram(void);

#endif
