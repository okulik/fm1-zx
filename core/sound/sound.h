/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* FM music from the beeper: a decoder that reads the beeper's edge stream,
 * a 2-operator FM synth, and the mixer that plays either the FM voices or
 * the beeper itself, 3 frames late, with crossfades.
 *
 * Integer and fixed-point arithmetic only (no float, no libm, no 64-bit
 * division), so the host, the device and the wasm build give the same samples.
 * Every struct is plain data that the caller allocates (statically on the
 * device); the init functions set every field. */

#ifndef SOUND_H
#define SOUND_H

#include <stdint.h>

#include "zx.h"

#define SND_DELAY_FRAMES 3u   /* the FM choice's fixed delay (60 ms) and the decoder's look-ahead */
#define SND_TONAL 3u          /* tonal voices; plus one drum voice */

/* Most samples a frame may have: 1024 covers sample rates up to 51.2 kHz
 * (the FM-1 renders 880-881 per frame at 750000/17 Hz). */
#define SND_MAX_FRAME_SAMPLES 1024u

/* Mixer hysteresis, in decoder frames: FM starts after this many confident
 * frames in a row and stops after this many unconfident ones. Leaving is
 * immediate (confidence over coverage); the decoder itself keeps short rests
 * inside music confident. */
#define SND_ENTER_FRAMES 2u
#define SND_LEAVE_FRAMES 1u

/* Crossfades (mode switches and FM/beeper decision changes) last
 * rate / SND_XFADE_DIV samples: 4 ms. */
#define SND_XFADE_DIV 250u

enum snd_style { SND_STYLE_NONE, SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES };
enum snd_mode { SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_COUNT }; /* Plans 6-7 add FM_B, FM_C before COUNT */

typedef struct {
	uint32_t period_q8;   /* full period in T-states, Q8 fixed point; 0 = voice off */
	uint8_t level;        /* 0-255 loudness */
} snd_voice;

typedef struct {          /* what the decoder concluded about one past frame */
	snd_voice tone[SND_TONAL];
	uint8_t drum;         /* 0 = none, else drum strength 1-255 */
	uint8_t style;        /* enum snd_style */
	uint8_t confident;    /* 1 = play FM for this frame */
} snd_frame;

/* ---- decoder (decoder.c) ---------------------------------------------- */

typedef struct snd_decoder {   /* edge history (SND_DELAY_FRAMES + current) and per-style state */
	uint32_t frames;           /* frames fed since init */
} snd_decoder;

void snd_decoder_init(snd_decoder *d);
/* Feeds frame t's edges; fills *out with the decision for frame t - SND_DELAY_FRAMES. */
void snd_decoder_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out);

/* ---- FM synth (fm.c) ---------------------------------------------------
 *
 * Phases are 32-bit (2^32 = one cycle). Each tonal voice is 2-operator FM at
 * a 1:1 ratio: out = sin(p + I * sin(p)), with the index I in "depth" units
 * of 2^16 / (2 pi) phase per unit (Q16 of one cycle per radian). Envelope
 * and index evolve on an 8 kHz tick (a fixed 0.125 ms, whatever the sample
 * rate), so their constants need no run-time maths. */

typedef struct {
	uint32_t phase;       /* carrier (and modulator: ratio 1:1) */
	uint32_t inc;         /* phase step per sample */
	uint32_t period_q8;   /* the note's period last frame, 0 = off */
	int32_t env;          /* Q16, 0-65536 */
	int32_t rel_step;     /* env decrement per tick while releasing */
	int32_t depth_base;   /* index the note settles at (depth units) */
	int32_t depth_ex;     /* index above the base, decaying */
	int32_t lvl;          /* level Q8 (0-65280), glides to lvl_target */
	int32_t lvl_target;
	int32_t amp;          /* (env >> 8) * (lvl >> 8): 0-65280, set per tick */
	uint8_t stage;        /* 0 off, 1 attack, 2 decay to sustain, 3 release */
} snd_fm_voice;

typedef struct snd_fm {   /* voice phases, envelopes */
	uint32_t rate_num, rate_den;
	uint64_t kfreq;       /* inc = kfreq / period_q8 */
	uint32_t tick_acc;    /* 8 kHz tick: += 8000 * den per sample, a tick per num */
	snd_fm_voice v[SND_TONAL];
	int32_t drum_env;     /* Q16 */
	int32_t drum_rel;     /* decrement per tick while releasing */
	int32_t drum_lvl;     /* 0-255 */
	int32_t drum_amp;
	int32_t drum_lp;      /* one-pole low-passed noise */
	uint32_t seed;        /* xorshift32 */
	uint8_t drum_stage;   /* 0 off, 1 decaying, 3 release */
	uint8_t drum_held;    /* the previous frame had a drum */
} snd_fm;

void snd_fm_init(snd_fm *f, uint32_t rate_num, uint32_t rate_den);
/* Renders n samples of the frame *fr (all of its voices, whatever fr->confident). */
void snd_fm_render(snd_fm *f, const snd_frame *fr, int16_t *out, uint32_t n);

/* ---- mixer (mixer.c) --------------------------------------------------- */

#define SND_RING_SAMPLES (SND_DELAY_FRAMES * SND_MAX_FRAME_SAMPLES)

typedef struct snd_mixer {   /* delay rings, hysteresis, crossfade; owns a decoder and an fm */
	snd_decoder dec;
	snd_fm fm;
	int16_t ring[SND_RING_SAMPLES];   /* beeper samples, SND_DELAY_FRAMES frames late */
	uint32_t rd, wr, fill;            /* ring read and write index, samples held */
	uint32_t mute;                    /* samples to pop before the delayed path may fade in */
	int32_t live_g, del_g, fm_g;      /* Q15 gains (32768 = 1): live beeper, delayed path, FM in it */
	int32_t step;                     /* gain change per sample of a crossfade */
	uint8_t primed;                   /* frames pushed, up to SND_DELAY_FRAMES */
	uint8_t mode;                     /* mode of the previous frame */
	uint8_t started;                  /* a frame has run since init */
	uint8_t fm_on;                    /* the hysteresis' decision: FM */
	uint8_t run;                      /* frames in a row disagreeing with fm_on */
} snd_mixer;

void snd_mixer_init(snd_mixer *m, uint32_t rate_num, uint32_t rate_den);
/* beeper: this frame's n beeper samples (from zx_beeper_render). Writes n samples to out:
 * SND_MODE_BEEPER -> a copy of beeper (no delay); FM modes -> 3-frame-delayed FM or beeper.
 *
 * The delay is a sample FIFO holding the first SND_DELAY_FRAMES frames' samples
 * after init, so the delayed stream is continuous (no sample repeated or
 * dropped) even though frames have 880 or 881 samples; frame t's output starts
 * within one sample of frame t - 3's first sample. The FIFO and the decoder are
 * fed in every mode, so an FM mode has its look-ahead at a switch.
 *
 * Mode switches: Beeper -> FM fades the live beeper out, plays silence, and
 * fades the delayed path in at the switch frame's first sample (60 ms later);
 * FM -> Beeper crossfades from the delayed path to the live beeper (the last 3
 * frames of delayed material are dropped). n above SND_MAX_FRAME_SAMPLES:
 * the beeper is copied and the mixer is left as it was. */
void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_edge *e, uint32_t n_edges, uint8_t start_level,
                     const int16_t *beeper, uint32_t n, int16_t *out);

#endif
