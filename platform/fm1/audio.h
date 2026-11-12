/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The beeper's way out: a FIFO of int16 samples at 750000/17 Hz that the main loop fills a
 * Spectrum frame at a time, drained by the ALNK0 (I2S) interrupt 128 stereo frames at a time
 * into felucca-bsp's double-buffered DMA (fm1_audio.h).
 *
 * The interrupt (fm1_alnk0_irq) and audio_push run from RAMTEXT and read no flash. The FIFO has
 * one writer (the main loop) and one reader (the interrupt); the main loop's other calls only
 * read its indices or set flags, so nothing needs a lock. */
#ifndef FMZX_AUDIO_H
#define FMZX_AUDIO_H

#include <stdint.h>

#define AUDIO_FIFO_SAMPLES 4096u /* a power of two */
#define AUDIO_BLOCK_FRAMES 128u  /* stereo frames per DMA half */

/* The FIFO empty, muted, gain 1024. Interrupts must still be off. */
void audio_init(void);
/* Starts the DMA and its interrupt (priority 3); interrupts must still be off. Returns the
 * felucca-bsp status (FM1_AUDIO_OK = 0). The stock audio PLL must be running. */
int audio_start(void);

/* Samples queued, and room left. */
uint32_t audio_fifo_level(void);
uint32_t audio_fifo_free(void);
/* Main loop (or RAMTEXT code it calls) only: queues n samples; what does not fit is dropped and
 * counted (audio_overflows). RAMTEXT. */
void audio_push(const int16_t *s, uint32_t n);
/* Main loop only, while muted: drops everything queued. */
void audio_flush(void);

/* Output = sample * q10 >> 10, q10 0-1024. */
void audio_set_gain(uint16_t q10);
/* Muted: the interrupt leaves the FIFO alone and plays the last value decaying to 0, and an
 * empty FIFO is not an underrun. */
void audio_mute(int on);

/* Blocks that found the FIFO empty while unmuted; samples dropped by audio_push. */
uint32_t audio_underruns(void);
uint32_t audio_overflows(void);

/* The ALNK0 interrupt body, called by felucca-bsp's isr_alnk0 wrapper (fm1_isr.S). RAMTEXT. */
void fm1_alnk0_irq(void);

#endif
