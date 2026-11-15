/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* XIP code (the image in flash) and RAMTEXT code (SRAM) are out of each other's branch reach, so
 * XIP reaches RAMTEXT through this one table of function pointers (main.c fills it in). It is
 * external, so the compiler cannot turn the indirect calls back into direct ones; check_reach.py
 * fails the build on any direct call between the two. */
#ifndef FMZX_RAMCALLS_H
#define FMZX_RAMCALLS_H

#include <stdint.h>
#include "sound.h"
#include "zx.h"

struct ram_calls {
    void (*init)(zx_machine *, const uint8_t *);
    int (*load_sna)(zx_machine *, const uint8_t *, uint32_t);
    int (*load_z80)(zx_machine *, const uint8_t *, uint32_t);
    int (*tape_insert)(zx_machine *, const uint8_t *, uint32_t);
    void (*tape_autostart)(zx_machine *);
    void (*beeper_init)(zx_beeper *, uint32_t, uint32_t);
    void (*mixer_init)(snd_mixer *, uint32_t, uint32_t);
    void (*compose_zx)(const zx_machine *, unsigned, unsigned, uint8_t *);
    void (*screen_line)(const zx_machine *, unsigned, uint8_t *);
    uint8_t (*border_line)(const zx_machine *, unsigned);
    void (*game_step)(void);    /* main.c: one Spectrum frame */
    void (*game_release)(void); /* main.c: all Spectrum keys and Kempston up */
    void (*lcd_pump)(void);     /* lcd.c */
};
extern struct ram_calls fmzx_ram;

/* lcd.c's RAMTEXT pump, for the table. */
void lcd_pump_ram(void);

/* What the device copy of app/pixmodes.c (XIP) calls for zx_screen_line, zx_border_line and
 * pix_compose_zx (platform/fm1/CMakeLists.txt renames them): main.c, through the table. */
void fmzx_xip_screen_line(const zx_machine *m, unsigned y, uint8_t *out);
uint8_t fmzx_xip_border_line(const zx_machine *m, unsigned frame_line);
void fmzx_xip_compose_zx(const zx_machine *m, unsigned row, unsigned crop_x, uint8_t *idx);

#endif
