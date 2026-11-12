/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The FM-1's key matrix, encoders and MASTER pot: the scan runs from TIMER5 (10 kHz), the main
 * loop takes snapshots. */
#ifndef FMZX_INPUT_H
#define FMZX_INPUT_H

#include <stdint.h>
#include "keymap.h"

/* Pins and the ADC; interrupts must still be off (PA2 is the key matrix /OE and the backlight). */
void input_init(void);

/* Called from the TIMER5 interrupt: one column of the scan. */
void input_tick_isr(void);

/* Main loop only. held: the debounced masks (buttons bit i = button i, notes bit n = note n) and
 * the encoder detents since the last poll (+ = clockwise). pressed_*: press edges since the last
 * poll. Any of the pointers may be NULL except held. */
void input_poll(km_input *held, uint32_t *pressed_buttons, uint32_t *pressed_notes);

/* The MASTER pot, 0-1023: the average of 4 reads. Main loop only. */
uint16_t input_master(void);

#endif
