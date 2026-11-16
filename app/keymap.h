/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* FM-1 buttons, note keys and encoders mapped to Spectrum keys and Kempston. */
#ifndef FMZX_KEYMAP_H
#define FMZX_KEYMAP_H

#include <stdint.h>

#define KM_BUTTONS 14u   /* OCT-,OCT+,FX,SCL,ENV,LFO,EDIT,GLO,HOME,SAVE,ARP,SEQ,PLAY,REC = 0..13 */
#define KM_NOTES 27u     /* note 0 = F3 ... 26 = G5 */
#define KM_ENCODERS 7u   /* SELECT, ALGORITHM, K1, K2, K3, K4, PRESETS */
#define KM_INPUTS (KM_BUTTONS + KM_NOTES + 2u * KM_ENCODERS)   /* 55: buttons, notes, enc e ccw = 41+2e, cw = 42+2e */
#define KM_ACTIONS 2u
#define KM_NONE 0xFFu
#define KM_BTN_HOME 8u   /* reserved: menu */
#define KM_BTN_PLAY 12u  /* reserved: pause / OK (a pack game cannot map it; BASIC's map gives it ENTER) */
#define KM_PULSE_FRAMES 3u

/* action byte: 0-39 Spectrum key (row*5 + bit; row r = A(8+r)), 0x40|b Kempston bit b (0 right,1 left,2 down,3 up,4 fire), KM_NONE */
typedef struct { uint8_t act[KM_INPUTS][KM_ACTIONS]; } keymap;

typedef struct { uint32_t buttons, notes; int8_t enc[KM_ENCODERS]; } km_input;   /* held masks + detents this poll (+ = cw) */

/* remaining pulse calls per encoder direction: [e][0] = ccw, [e][1] = cw */
typedef struct km_state { uint8_t pulse[KM_ENCODERS][2]; } km_state;

void km_default(keymap *k);
/* The picker's Spectrum BASIC (no pack entry): all 40 keys. The default's digits, CAPS and SYMBOL
 * SHIFT on the buttons, A..Z on the notes F3..F#5 in chromatic order, SPACE on G5, ENTER on PLAY
 * (which then does not pause: the device pauses on PLAY only when the map leaves it unset). */
void km_basic(keymap *k);
void km_reset(km_state *s);
/* One poll: rows (bit set = pressed) and kempston are fully rewritten. HOME never maps; PLAY maps
 * only where the table sets it (BASIC's). */
void km_apply(const keymap *k, km_state *s, const km_input *in, uint8_t rows[8], uint8_t *kempston);

#endif
