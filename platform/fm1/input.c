/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#define FM1_IRQ_DEFS 0 /* main.c owns the interrupt definitions */
#include "fm1_irq.h"
#include "fm1_input.h"
#include "fm1_adc.h"

#include "input.h"

void input_init(void)
{
    fm1_input_init();
    fm1_adc_init();
}

void input_tick_isr(void)
{
    fm1_input_tick();
}

void input_poll(km_input *held, uint32_t *pressed_buttons, uint32_t *pressed_notes)
{
    uint32_t e, b, n;
    uint32_t k = fm1__lock();
    held->buttons = fm1_in.buttons;
    held->notes = fm1_in.notes;
    fm1__unlock(k);
    b = fm1_input_edges(0);
    n = fm1_input_note_edges();
    for (e = 0; e < KM_ENCODERS; e++) {
        int32_t s = fm1_enc_take(e);
        held->enc[e] = (int8_t)(s > 127 ? 127 : s < -127 ? -127 : s);
    }
    if (pressed_buttons)
        *pressed_buttons = b;
    if (pressed_notes)
        *pressed_notes = n;
}

uint16_t input_master(void)
{
    uint32_t i, sum = 0, ok = 0;
    for (i = 0; i < 4u; i++) {
        int32_t v = fm1_adc_read(FM1_ADC_MASTER);
        if (v >= 0) {
            sum += (uint32_t)v;
            ok++;
        }
    }
    return ok ? (uint16_t)(sum / ok) : 0u;
}
