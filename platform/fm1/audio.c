/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#define FM1_IRQ_DEFS 0 /* main.c owns the interrupt definitions */
#include "fm1_irq.h"
#include "fm1_audio.h"

#include "audio.h"

#define RAMTEXT __attribute__((section(".ram_text.audio"), noinline))
#define POOL(tag) __attribute__((section(".pool." tag), aligned(4)))

#define HALF_WORDS (2u * AUDIO_BLOCK_FRAMES) /* int32 words per DMA half: L, R */
#define FIFO_MASK (AUDIO_FIFO_SAMPLES - 1u)
#define ALNK0_PRIO 3u

void isr_alnk0(void);

/* The DMA's two halves and the FIFO, in SRAM (the DMA and the interrupt read no flash). */
static int32_t abuf[2u * HALF_WORDS] POOL("audio_dma");
static int16_t fifo[AUDIO_FIFO_SAMPLES] POOL("audio_fifo");

static struct {
    volatile uint32_t head;   /* written by the main loop: samples ever queued */
    volatile uint32_t tail;   /* written by the interrupt: samples ever played */
    volatile uint32_t gain;   /* Q10 */
    volatile uint32_t muted;
    volatile uint32_t underruns, overflows;
    int32_t last;             /* the interrupt's last output, int16 scale */
} au;

void audio_init(void)
{
    au.head = au.tail = 0;
    au.gain = 1024u;
    au.muted = 1;
    au.underruns = au.overflows = 0;
    au.last = 0;
}

int audio_start(void)
{
    return fm1_audio_init(abuf, HALF_WORDS, isr_alnk0, ALNK0_PRIO);
}

uint32_t audio_fifo_level(void)
{
    return au.head - au.tail;
}

uint32_t audio_fifo_free(void)
{
    return AUDIO_FIFO_SAMPLES - (au.head - au.tail);
}

void RAMTEXT audio_push(const int16_t *s, uint32_t n)
{
    uint32_t head = au.head, room = AUDIO_FIFO_SAMPLES - (head - au.tail), i;
    if (n > room) {
        au.overflows += n - room;
        n = room;
    }
    for (i = 0; i < n; i++)
        fifo[(head + i) & FIFO_MASK] = s[i];
    __asm__ volatile("" ::: "memory"); /* the samples before the index that publishes them */
    au.head = head + n;
}

void audio_flush(void)
{
    au.head = au.tail;
}

void audio_set_gain(uint16_t q10)
{
    au.gain = q10 > 1024u ? 1024u : q10;
}

void audio_mute(int on)
{
    au.muted = on ? 1u : 0u;
}

uint32_t audio_underruns(void)
{
    return au.underruns;
}

uint32_t audio_overflows(void)
{
    return au.overflows;
}

/* One DMA half: 128 frames, each sample to both channels as Q23 (int16 << 8). */
static void RAMTEXT fill(int32_t *out)
{
    uint32_t i, tail = au.tail, head = au.head, dry = 0;
    int32_t v = au.last, gain = (int32_t)au.gain;
    int quiet = au.muted != 0;
    for (i = 0; i < AUDIO_BLOCK_FRAMES; i++) {
        if (!quiet && tail != head) {
            v = fifo[tail & FIFO_MASK] * gain >> 10;
            tail++;
        } else {
            dry |= !quiet;
            v -= v >> 4; /* toward 0 (stops within 15 of it from above) */
        }
        out[2u * i] = out[2u * i + 1u] = v * 256;
    }
    au.last = v;
    au.tail = tail;
    if (dry)
        au.underruns++;
}

void RAMTEXT fm1_alnk0_irq(void)
{
    uint8_t p = fm1_audio_pending();
    fm1_audio_ack_aux(p);
    if (p & FM1_AUDIO_HALF) {
        fill(abuf + fm1_audio_free_half() * HALF_WORDS);
        fm1_audio_ack_half();
    }
}
