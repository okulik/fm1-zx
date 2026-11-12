/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "keymap.h"
#include <string.h>

#define K_(row, bit) ((uint8_t)((row) * 5u + (bit)))
#define KEMP(b) ((uint8_t)(0x40u | (b)))
#define KEMP_RIGHT 0u
#define KEMP_LEFT 1u
#define KEMP_DOWN 2u
#define KEMP_UP 3u
#define KEMP_FIRE 4u

enum { BTN_OCTM, BTN_OCTP, BTN_FX, BTN_SCL, BTN_ENV, BTN_LFO, BTN_EDIT, BTN_GLO, BTN_HOME, BTN_SAVE, BTN_ARP, BTN_SEQ, BTN_PLAY, BTN_REC };

static void set1(keymap *k, unsigned input, uint8_t a)
{
    k->act[input][0] = a;
    k->act[input][1] = KM_NONE;
}

void km_default(keymap *k)
{
    unsigned i;
    for (i = 0; i < KM_INPUTS; i++) {
        k->act[i][0] = KM_NONE;
        k->act[i][1] = KM_NONE;
    }
    /* digits 1-8 on OCT-, OCT+, FX, SCL, ENV, LFO, EDIT, GLO; 9 on SAVE, 0 on ARP */
    set1(k, BTN_OCTM, K_(3, 0));
    set1(k, BTN_OCTP, K_(3, 1));
    set1(k, BTN_FX, K_(3, 2));
    set1(k, BTN_SCL, K_(3, 3));
    set1(k, BTN_ENV, K_(3, 4));
    set1(k, BTN_LFO, K_(4, 4));   /* 6 */
    set1(k, BTN_EDIT, K_(4, 3));  /* 7 */
    set1(k, BTN_GLO, K_(4, 2));   /* 8 */
    set1(k, BTN_SAVE, K_(4, 1));  /* 9 */
    set1(k, BTN_ARP, K_(4, 0));   /* 0 */
    set1(k, BTN_SEQ, K_(0, 0));   /* CAPS SHIFT */
    set1(k, BTN_REC, K_(7, 1));   /* SYMBOL SHIFT */
    /* notes: F3 = 0 ... G5 = 26 */
    set1(k, KM_BUTTONS + 0, KEMP(KEMP_LEFT));
    set1(k, KM_BUTTONS + 1, KEMP(KEMP_UP));      /* F#3 */
    set1(k, KM_BUTTONS + 2, KEMP(KEMP_DOWN));    /* G3 */
    set1(k, KM_BUTTONS + 3, KEMP(KEMP_UP));      /* G#3 */
    set1(k, KM_BUTTONS + 4, KEMP(KEMP_RIGHT));   /* A3 */
    set1(k, KM_BUTTONS + 7, K_(6, 0));           /* C4 ENTER */
    set1(k, KM_BUTTONS + 9, K_(7, 0));           /* D4 SPACE */
    set1(k, KM_BUTTONS + 11, K_(5, 4));          /* E4 Y */
    set1(k, KM_BUTTONS + 12, K_(7, 3));          /* F4 N */
    set1(k, KM_BUTTONS + 19, KEMP(KEMP_FIRE));   /* C5 */
    set1(k, KM_BUTTONS + 21, KEMP(KEMP_FIRE));   /* D5 */
    set1(k, KM_BUTTONS + 23, KEMP(KEMP_FIRE));   /* E5 */
    set1(k, KM_BUTTONS + 24, KEMP(KEMP_FIRE));   /* F5 */
    set1(k, KM_BUTTONS + 26, KEMP(KEMP_FIRE));   /* G5 */
}

void km_reset(km_state *s)
{
    memset(s, 0, sizeof *s);
}

static void press(const uint8_t a[KM_ACTIONS], uint8_t rows[8], uint8_t *kemp)
{
    unsigned i;
    for (i = 0; i < KM_ACTIONS; i++) {
        uint8_t v = a[i];
        if (v < 40u)
            rows[v / 5u] |= (uint8_t)(1u << (v % 5u));
        else if (v >= 0x40u && v < 0x45u)
            *kemp |= (uint8_t)(1u << (v & 7u));
    }
}

void km_apply(const keymap *k, km_state *s, const km_input *in, uint8_t rows[8], uint8_t *kempston)
{
    unsigned i, e, d;
    uint8_t kemp = 0;
    memset(rows, 0, 8);
    for (i = 0; i < KM_BUTTONS; i++)
        if (i != KM_BTN_HOME && i != KM_BTN_PLAY && (in->buttons >> i & 1u))
            press(k->act[i], rows, &kemp);
    for (i = 0; i < KM_NOTES; i++)
        if (in->notes >> i & 1u)
            press(k->act[KM_BUTTONS + i], rows, &kemp);
    for (e = 0; e < KM_ENCODERS; e++) {
        int n = in->enc[e];
        for (d = 0; d < 2; d++) {            /* d 0 = ccw, 1 = cw */
            unsigned add = 0, left = s->pulse[e][d];
            if (d == 0 && n < 0)
                add = (unsigned)(-n) * KM_PULSE_FRAMES;
            else if (d == 1 && n > 0)
                add = (unsigned)n * KM_PULSE_FRAMES;
            left += add;
            if (left > 255u)
                left = 255u;
            if (left) {
                press(k->act[KM_BUTTONS + KM_NOTES + 2u * e + d], rows, &kemp);
                left--;
            }
            s->pulse[e][d] = (uint8_t)left;
        }
    }
    *kempston = kemp;
}
