/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* FM music from the beeper: a decoder that reads the beeper's edge stream
 * (method A), the recogniser of known music engines (method B), a 2-operator
 * FM synth, and the mixer that plays either the FM voices or the beeper
 * itself, 3 frames late, with crossfades.
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
 * frames in a row and stops after this many unconfident ones. Leaving takes a
 * second unconfident frame: a lone one inside music (a note boundary, a frame
 * partly explained) holding the voices costs other sound at most a frame,
 * while dropping to the beeper for it clicks; the decoder itself keeps short
 * rests inside music confident. FM: engines (method B) starts after one claimed
 * frame: its claims are read from the engine, not guessed. */
#define SND_ENTER_FRAMES 2u
#define SND_LEAVE_FRAMES 2u

/* Crossfades (mode switches and FM/beeper decision changes) last
 * rate / SND_XFADE_DIV samples: 4 ms. */
#define SND_XFADE_DIV 250u

enum snd_style { SND_STYLE_NONE, SND_STYLE_SLOTS, SND_STYLE_FLIPS, SND_STYLE_PULSES, SND_STYLE_ENGINE };
/* The Sound choices: the beeper itself; FM from the decoder (method A, "FM: sound"); FM from the
 * known music engines (method B, "FM: engines"). Plan 7 adds FM_C before COUNT. */
enum snd_mode { SND_MODE_BEEPER, SND_MODE_FM_A, SND_MODE_FM_B, SND_MODE_COUNT };

/* What a tonal voice plays in the tune, and so the FM instrument it gets (fm.c): AUTO is no role
 * decided (the synth plays it as LEAD). */
enum snd_role { SND_ROLE_AUTO, SND_ROLE_BASS, SND_ROLE_LEAD, SND_ROLE_PAD };

typedef struct {
	uint32_t period_q8;   /* full period in T-states, Q8 fixed point; 0 = voice off */
	uint8_t level;        /* 0-255 loudness */
} snd_voice;

#define SND_EVENTS 8u     /* most note events in a frame */

typedef struct {          /* a voice change inside a frame */
	uint32_t t;           /* frame T-state, 0 .. ZX_FRAME_TSTATES - 1, ascending within a frame */
	uint32_t period_q8;   /* tonal: the voice's new period (0 = off); drum: ignored */
	uint8_t voice;        /* 0 .. SND_TONAL-1 tonal, SND_TONAL = the drum voice */
	uint8_t level;        /* tonal: loudness; drum: strength (0 = drum off) */
} snd_event;

typedef struct {          /* what a method concluded about one past frame */
	snd_voice tone[SND_TONAL];   /* the state at the frame's start */
	uint8_t drum;         /* 0 = none, else drum strength 1-255 */
	uint8_t style;        /* enum snd_style */
	uint8_t confident;    /* 1 = play FM for this frame */
	uint8_t engine;       /* 0 = none, else 1 + index in the engine table (method B) */
	uint8_t n_ev;         /* events applied in order after the frame's start; method A: 0 */
	uint8_t role[SND_TONAL];   /* enum snd_role of each tonal voice; a voice takes it only when a
	                            * note starts on it (fm.c) */
	snd_event ev[SND_EVENTS];
} snd_frame;

/* ---- decoder (decoder.c, flips.c, slots.c, pulses.c) ------------------- */

#define SND_HIST (SND_DELAY_FRAMES + 1u)   /* frames of edge history: the decided frame and its look-ahead */
#define SND_EDGES_MAX 256u                 /* speaker edges kept per frame; a busier frame is noise */
#define SND_WORK_MAX 48000u                 /* decoder work units per frame (flips.c, slots.c), see decoder.c */
#define SND_SLOTS 3u                        /* most channels (time slots) of a time-slot loop */
#define SND_RISES 256u                      /* rises kept per time-slot channel: 3.5 frames up to 3.6 kHz */
#define SND_LEVEL_BINS 128u                 /* box-level histogram: levels in steps of 2/256 */

/* What the decoder worked out about one frame of the history, when it arrived. */
typedef struct {
	uint32_t hp_q8[SND_TONAL];     /* voices of the frame alone (20 ms): half-period, T x 256, as found; 0 = none */
	uint32_t pair_q8[SND_TONAL];   /* voices of this frame and the one before (40 ms; short pulses: two before, 60 ms) */
	uint16_t n;                    /* speaker edges kept */
	uint16_t expl;                 /* of them, explained by the frame's own voices */
	uint8_t busy;                  /* more than SND_EDGES_MAX edges: noise, not decoded */
	uint8_t clean;                 /* the frame alone decodes into voices that explain >= 90% of its edges (decoder.c) */
	uint8_t change;                /* a pitch change starts here (decoder.c) */
	uint8_t full;                  /* its edges (rises for short pulses) span most of the frame: a note, not a beep */
	uint8_t steady;                /* none of its own voices drifts in pitch within it (flips.c) */
	uint8_t glide;                 /* part of a pitch sweep (decoder.c): no note, and it ends the music context */
	uint8_t alt;                   /* part of a quick alternation, A B A within the look-ahead (decoder.c): no note */
	/* short pulses (pulses.c) */
	uint8_t ear0;                  /* the speaker level at the frame's start: tells rising edges */
	uint8_t pulse;                 /* analysed as short pulses (voices from rising edges, hp_q8 = period / 2) */
	uint16_t width;                /* the engine's pulse width (T) in use for it; 0 = unknown */
	/* time slots (slots.c) */
	uint8_t carrier;               /* most rising edges repeat at the loop length in use (40-400 T) */
	uint8_t tdm;                   /* decoded as time slots: a loop is known, its channels learned */
	uint8_t fit;                   /* % of the frame's box levels a channel combination explains */
} snd_slot;

/* The time-slot decoder's state (slots.c): the loop, the box sampler running
 * across frames, the channels' weights and each channel's rises. Times are
 * absolute T-states (frame x ZX_FRAME_TSTATES + T-state, wrapping at 2^32);
 * the sampler's are relative to the current frame's start. */
typedef struct {
	uint32_t loop_q8;                      /* the loop length P, T x 256; 0 = no loop */
	uint32_t loop_frame;                   /* the last frame with a carrier at P */
	uint32_t other_q8;                     /* a different carrier seen in the last carrier frame */
	uint32_t bare;                         /* frames in a row with sound but no carrier */
	/* the box sampler: grid points every P/2; a box spans two hops (P) */
	int32_t grid_q8;                       /* the next grid point, T x 256 */
	int32_t last_t;                        /* the last speaker edge */
	uint32_t cum;                          /* speaker-high time up to last_t (wraps) */
	uint32_t h[2];                         /* speaker-high time up to the last two grid points */
	uint32_t recip;                        /* 2^24 / box length: high time to a level in 1/256 */
	uint8_t running;                       /* the sampler has started (it starts with a loop) */
	uint8_t level;                         /* the speaker level after last_t */
	uint8_t primed;                        /* grid points evaluated since the sampler started, up to 2 */
	uint16_t lv[2];                        /* the last two box levels (the plateau test) */
	/* the channels' weights, learned from the box levels' plateaus */
	uint16_t hist[SND_LEVEL_BINS];
	uint32_t hist_total;
	uint32_t learnt_frame;                 /* when the weights were last learned */
	uint8_t k;                             /* channels */
	uint8_t w[SND_SLOTS];                  /* their weights (slot / loop), 1/256, ascending */
	uint8_t lut[257];                      /* box level -> channel states, | 0x80 if a combination fits */
	/* the channels: debounced states and rise times */
	uint8_t cur;                           /* debounced states, bit c = channel c */
	uint8_t cand[SND_SLOTS], run[SND_SLOTS];
	uint32_t rise[SND_SLOTS][SND_RISES];   /* a ring per channel */
	uint8_t swing[SND_SLOTS][SND_RISES];   /* the rise came with every channel's (a full swing) */
	uint32_t nrise[SND_SLOTS];             /* rises recorded since the channel was (re)set */
	/* frame counters for the decision */
	uint32_t good, samples;                /* this frame's box levels: fitting, all */
	/* the music context (decoder.c): pitch changes of the decoded voices; it
	 * goes with the loop, so a loop forgotten or replaced clears it */
	uint32_t sref_q8[SND_TONAL];           /* the reference: the voices of the last note (half-periods) */
	uint32_t sref_frame, schange_frame;
	uint32_t pend_q8[SND_TONAL];           /* another note's voices, until they hold for two frames */
	uint32_t pend_frame;
	uint8_t pend_n;                        /* frames in a row they held */
	uint8_t have_sref, have_schange;
	uint8_t sbroken;                       /* undecodable sound since the last pitch change */
} snd_slots;

typedef struct snd_decoder {   /* edge history (SND_DELAY_FRAMES + current) and per-style state */
	uint32_t frames;                       /* frames fed since init */
	uint32_t t[SND_HIST][SND_EDGES_MAX];   /* the edge history: speaker edges' T-states in their frame */
	snd_slot slot[SND_HIST];               /* per frame, at the same ring index (frame % SND_HIST) */
	/* note timing: one note per frame (frame-aligned changes) or free-running long notes */
	int8_t timing;                         /* votes, -3..3: > 0 pitch changes land on frame boundaries */
	uint8_t frame_notes;                   /* the timing in use: 1 = one note per frame */
	/* music context: pitch changes */
	uint8_t have_clean, have_change;
	uint32_t clean_frame;                  /* the last clean frame */
	uint32_t clean_q8[SND_TONAL];          /* the reference: the voices of the last note */
	uint32_t change_frame;                 /* the last pitch change in a frame already decided */
	uint8_t broken;                        /* a decided frame was undecodable since the last pitch change */
	/* alternations: a return to the note before the reference is no pitch change */
	uint32_t prev_q8[SND_TONAL];           /* the reference before the current one */
	uint32_t ref_change_frame;             /* the pitch change that made the current reference */
	uint32_t ref_start, prev_start;        /* the frames the current and the previous reference began */
	uint8_t have_prev, ref_change;
	uint8_t returns;                       /* returns in a row */
	uint8_t alt;                           /* two returns in a row since the last pitch change: a siren, not music */
	/* short pulses: the style in use (selector) and the styles of the reference and of the music context */
	uint8_t pulse_style, clean_pulse, ctx_pulse;
	uint16_t pulse_width;                  /* the commonest pulse width last measured (T) */
	uint8_t busy_run;                      /* undecodable frames decided in a row, before this one */
	/* work accounting (flips.c): units spent on this frame, and the cap */
	uint32_t work, work_cap;
	/* the decisions */
	uint32_t out_q8[SND_TONAL];            /* the last decision's voices, by tone slot */
	uint32_t since_tonal;                  /* decisions since the last confident one with voices */
	uint8_t prev_conf;                     /* the last decision was confident */
	/* time slots (slots.c) */
	snd_slots sl;
	/* scratch for flips.c */
	uint32_t pool[2u * SND_EDGES_MAX];
	uint16_t chain[2u * SND_EDGES_MAX];
	uint8_t used[2u * SND_EDGES_MAX];
} snd_decoder;

void snd_decoder_init(snd_decoder *d);
/* Feeds frame t's edges; fills *out with the decision for frame t - SND_DELAY_FRAMES. */
void snd_decoder_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out);

/* The speaker-flip decoder (flips.c), used by decoder.c. t: n edge times, ascending.
 * snd_flips_decode finds up to SND_TONAL voices (half-periods, T x 256, in the order found,
 * zeros after) in t, which it uses as scratch (t may be d->pool), and returns how many edges they
 * explain; *drift (if given) is set if a voice drifts in pitch within t. pulses: t are rising
 * edges of short pulses (pulses.c), each step a whole period (hp_q8 then holds the period).
 * snd_flips_cover follows the voice of half-period hp_q8 through the edges t[0..m) not yet
 * marked in d->used (chains of 3 or more edges, which may run on into t[m..n)), marks the edges
 * it takes, returns their count and puts the time its chains span within t[0..m) in *extent
 * (pulses: the time the pulse train sounds within the frame, t from 0 to ZX_FRAME_TSTATES). */
uint32_t snd_flips_decode(snd_decoder *d, uint32_t *t, uint32_t n, int pulses, uint32_t *hp_q8, uint8_t *drift);
/* Is the half-period p_q8 a whole multiple (2-32 times, within the link tolerance) of q_q8? */
int snd_flips_multiple(uint32_t p_q8, uint32_t q_q8);
uint32_t snd_flips_cover(snd_decoder *d, const uint32_t *t, uint32_t n, uint32_t m, uint32_t hp_q8, int pulses,
                         uint32_t *extent);

/* The time-slot decoder (slots.c), used by decoder.c. snd_slots_frame takes frame d->frames's
 * edges: finds the loop carrier, runs the box sampler through the frame (decoding the channel
 * states and recording each channel's rises) and fills fs->carrier, fs->tdm and fs->fit.
 * Absolute times are frame x ZX_FRAME_TSTATES + T-state. keep_q8: the voices (periods, T x 256,
 * 0 = none) a run of whole-speaker swings may continue (NULL: any run of them is a voice, as when
 * the loop carrier is heard). snd_slots_voice gives channel c's period
 * (T x 256) at the time `at` if a run of agreeing periods spans it, else 0; snd_slots_level its
 * loudness; snd_slots_explained how many of the channels' rises in [from, to) belong to a voice's
 * run (*rises: how many there are, *swings: how many came in full swings); snd_slots_drums the full swings in [from, to) that no run
 * explains (another routine driving the speaker: a drum). */
/* The short-pulse decoder (pulses.c), used by decoder.c. snd_pulses_style tells whether the n
 * stored edges t of a frame starting at speaker level ear0 are short pulses (1, with the pulses'
 * width in *width) or not (0): the speaker high under 20% of the busy time, the pulses of one
 * width. snd_pulses_join puts the rising edges of frames f..f+count-1 of the
 * history into d->pool on one time axis (with the rises lost in merged pulses) and returns their
 * count (the first frame's in *first_n).
 * snd_pulses_decode finds the voices of n rising edges t (scratch, as for snd_flips_decode) as
 * half-periods (period / 2, so every style reports alike) and returns how many rises they
 * explain. */
int snd_pulses_style(snd_decoder *d, const uint32_t *t, uint32_t n, uint8_t ear0, uint32_t *width);
uint32_t snd_pulses_join(snd_decoder *d, uint32_t f, uint32_t count, uint32_t *first_n);
uint32_t snd_pulses_decode(snd_decoder *d, uint32_t *t, uint32_t n, uint32_t *hp_q8, uint8_t *drift);

void snd_slots_reset(snd_slots *s);
void snd_slots_frame(snd_decoder *d, const zx_edge *e, uint32_t n, uint8_t start_level, snd_slot *fs);
uint32_t snd_slots_voice(snd_decoder *d, uint32_t c, uint32_t at, const uint32_t *keep_q8);
uint8_t snd_slots_level(const snd_slots *s, uint32_t c);
uint32_t snd_slots_explained(snd_decoder *d, uint32_t from, uint32_t to, const uint32_t *keep_q8, uint32_t *rises,
                             uint32_t *swings);
uint32_t snd_slots_drums(snd_decoder *d, uint32_t from, uint32_t to);
/* Replaces the time-slot channels with nw channels of weights w (1/256, ascending): each old
 * channel's rises go to the new channel whose weight is nearest its own within the tolerance,
 * one new channel at most; the others start empty. (Used by the weight learning; exposed for
 * its tests.) */
void snd_slots_rechannel(snd_decoder *d, const uint32_t *w, uint32_t nw);

/* ---- FM synth (fm.c) ---------------------------------------------------
 *
 * Phases are 32-bit (2^32 = one cycle). Each tonal voice is 2-operator FM playing the instrument
 * (patch) of its role: out = sin(p + I * sin(r * p)), the modulator at r = 1 or 2 times the
 * carrier, with the index I in "depth" units of 2^16 / (2 pi) phase per unit (Q16 of one cycle
 * per radian); the pad adds a second carrier 4 cents up, the lead a vibrato. Envelope, index and
 * vibrato evolve on an 8 kHz tick (a fixed 0.125 ms, whatever the sample rate), so their
 * constants need no run-time maths. */

typedef struct {
	uint32_t phase;       /* carrier; the modulator's phase is r times it */
	uint32_t phase2;      /* pad: the second carrier */
	uint32_t inc;         /* the note's phase step per sample */
	uint32_t step;        /* the step played: inc with the vibrato (lead), set per tick */
	uint32_t inc2;        /* pad: the second carrier's step */
	uint32_t ticks;       /* 8 kHz ticks since the note started (the vibrato's delay and phase) */
	uint32_t period_q8;   /* the note's period last frame, 0 = off */
	int32_t env;          /* Q16, 0-65536 */
	int32_t rel_step;     /* env decrement per tick while releasing */
	int32_t depth_base;   /* index the note settles at (depth units) */
	int32_t depth_ex;     /* index above the base, decaying */
	int32_t lvl;          /* level Q8 (0-65280), glides to lvl_target */
	int32_t lvl_target;
	int32_t amp;          /* env * level * the patch's gain: out = sin * amp >> 17; set per tick */
	uint8_t stage;        /* 0 off, 1 attack, 2 decay to sustain, 3 release */
	uint8_t role;         /* enum snd_role of the note, latched at its start (never AUTO) */
} snd_fm_voice;

typedef struct snd_fm {   /* voice phases, envelopes */
	uint32_t rate_num, rate_den;
	uint64_t kfreq;       /* inc = kfreq / period_q8 */
	uint32_t tick_acc;    /* 8 kHz tick: += 8000 * den per sample, a tick per num */
	snd_fm_voice v[SND_TONAL];
	int32_t drum_env;     /* the noise's envelope, Q24 */
	int32_t drum_rel;     /* its decrement per tick while releasing */
	int32_t drum_tenv;    /* the tone's envelope, Q24 */
	int32_t drum_trel;
	int32_t drum_lvl;     /* 0-255 */
	int32_t drum_amp;     /* the noise's and the tone's amplitude: out = x * amp >> 18 */
	int32_t drum_tamp;
	int32_t drum_lp;      /* one-pole low-passed noise */
	uint32_t drum_phase;  /* the tone's phase */
	uint32_t drum_inc;    /* its step per sample (190 Hz) */
	uint32_t seed;        /* xorshift32 */
	uint8_t drum_stage;   /* 0 off, 1 decaying, 3 release */
	uint8_t drum_held;    /* the previous frame had a drum */
} snd_fm;

void snd_fm_init(snd_fm *f, uint32_t rate_num, uint32_t rate_den);
/* The voices and the drum to rest, as snd_fm_init leaves them; the rate stays. */
void snd_fm_reset(snd_fm *f);
/* Renders n samples of the frame *fr (all of its voices, whatever fr->confident): the voices
 * and the drum as at the frame's start, then each event from sample t * n / ZX_FRAME_TSTATES,
 * as if the frame had changed that voice there (a new period starts a note, period 0 releases
 * it; a drum event starts the drum, level 0 releases it). */
void snd_fm_render(snd_fm *f, const snd_frame *fr, int16_t *out, uint32_t n);

/* ---- the live rule: each voice's role by its pitch (roles.c) ------------
 *
 * Per tonal voice slot, a slow average of the pitch it plays while it sounds; the slots that
 * sounded recently, ranked by it with hysteresis (a margin to swap two), give the roles: the
 * lowest bass, the highest lead, a third the pad; two voices bass and lead; a voice alone lead.
 * A slot silent for longer than the window leaves the ranking and keeps its history. */

#define SND_ROLES_NONE (-(1 << 30))

typedef struct snd_roles {
	int32_t avg[SND_TONAL];     /* the slot's average pitch as log2 of its period (Q16 octaves:
	                             * higher is lower); SND_ROLES_NONE before it sounds, so it ranks
	                             * above any pitch */
	uint8_t n[SND_TONAL];       /* frames heard (the average's weight), up to 255; 0 = no history */
	uint8_t left[SND_TONAL];    /* frames left of the window since the slot last sounded; 0 =
	                             * not heard recently (out of the ranking) */
	uint8_t order[SND_TONAL];   /* the slots by average, lowest pitch first (with the hysteresis);
	                             * slots without history last */
	uint8_t role[SND_TONAL];    /* each slot's live role (enum snd_role) */
} snd_roles;

void snd_roles_init(snd_roles *r);
/* Tracks every sounding tonal voice of *fr (its frame-start voices and its events) and fills each
 * fr->role[k] that is SND_ROLE_AUTO with the live rule's role; a role already set is kept. */
void snd_roles_frame(snd_roles *r, snd_frame *fr);

/* ---- method B: known music engines (engines.c) -------------------------
 *
 * B recognises the music routines of a table of engines by the code around the OUTs that drive
 * the speaker (the frame's OUT sites, zx_out_sites), reads their notes from memory and
 * registers, checks them against the frame's edges and reports them with note events placed in
 * the frame. Anything it does not recognise is not B's (the beeper plays it).
 *
 * An entry's code is matched at an anchor address: its signature is `len` bytes from the
 * anchor, the self-modified operand bytes (`wild`) left out and the absolute addresses inside
 * it (`reloc`, 16-bit words) hashed as their offset from the anchor, so the engine is
 * recognised wherever it sits; FNV-1a 64 of that is `digest`. Each OUT the engine drives the
 * speaker from is a site at a fixed offset from the anchor. Every address a reader uses is
 * relative to the anchor or read from an operand inside the matched code. */

enum snd_site_role {
	SND_SITE_TONAL,   /* the OUT of a tonal channel (chan) */
	SND_SITE_DRUM,    /* the OUT of a drum routine */
	SND_SITE_STOP     /* an OUT that silences every channel (the end of a note) */
};

typedef struct {
	uint16_t off;     /* the OUT's offset from the anchor */
	uint8_t role;     /* enum snd_site_role */
	uint8_t chan;     /* tonal: the channel (and the voice it plays on); SND_CHAN_ANY: any channel's
	                   * (entries with flips: one OUT for all channels) */
} snd_engine_site;

#define SND_CHAN_ANY 0xFFu

#define SND_B_SINCE_ALL 0xFFFFFFFFu   /* snd_b_read.since / row: not known (all the frame) */

typedef struct {          /* what a reader found */
	snd_voice v[SND_TONAL];   /* channel c's voice (period 0 = silent) */
	uint32_t since;           /* T-states the values have held: up to the site's last edge, or
	                           * (stale 1) up to the frame's end */
	uint32_t row;             /* T-states since the note began (the engine paused before it), up to
	                           * the site's last edge (>= since; SND_B_SINCE_ALL: not known) */
	uint32_t step;            /* T-states between changes when they come in steps (a slide), else 0 */
	snd_voice back[SND_TONAL];   /* step: the voices one step before v (period 0 for a channel
	                              * whose voice is not known then) */
	uint8_t stale;            /* 0: v are the voices of the site's last edge; 1: the engine moved on
	                           * after that edge, v are its voices at the frame's end; 2: the
	                           * memory is not to be read now (between notes) */
	uint32_t left;            /* entries with min_len: T-states from the site's last edge to the
	                           * note's end (its last edge to come, plus half a period) */
} snd_b_read;

typedef struct snd_engine {   /* one table entry */
	const char *name;         /* e.g. "Castaway", "ROM BEEP" */
	uint64_t digest;          /* FNV-1a 64 of the signature */
	uint16_t len;             /* signature bytes from the anchor */
	uint8_t n_wild, n_reloc, n_sites;
	uint8_t channels;         /* tonal channels, at most SND_TONAL */
	uint16_t pass;            /* T-states of one pass of the sound loop (the check), 0 = one channel */
	const uint16_t *wild;     /* offsets of bytes left out */
	const uint16_t *reloc;    /* offsets of absolute addresses (little-endian words) */
	const snd_engine_site *sites;
	const uint16_t *pos;      /* per channel: its OUT's T-state in a pass, from channel 0's */
	/* Reads the voices at the end of the frame: m the machine (its memory and registers at the
	 * frame's end), en this entry, anchor the matched code, r the registers at the last edge of
	 * tonal site `site` (index in sites). */
	void (*read)(const zx_machine *m, const struct snd_engine *en, uint16_t anchor, uint32_t site, const zx_regs *r,
	             snd_b_read *out);
	uint8_t drum;             /* the drum's strength, for entries with drum sites */
	/* 0, or: each edge flips one channel's level, the speaker sounds their XOR (engines.c, flip;
	 * pass 0) */
	uint8_t flips;
	/* 0, or: each note is one call of the routine (one tonal site, a square of equal halves at
	 * exactly the period read; engines.c, observe_calls), and a note shorter than this (T) is
	 * not B's (ROM BEEP) */
	uint32_t min_len;
	const void *data;         /* the reader's data: NULL for the built-in entries; a learned entry's
	                           * snd_learned_ext (learned.c) */
	uint8_t strict;           /* 1: consecutive rises must lie one period apart, not a whole number
	                           * of periods (engines.c, known; learned entries); 0: the built-ins */
	uint8_t role[SND_TONAL];  /* each channel's stored role (enum snd_role), which B gives the
	                           * frames this entry claims; SND_ROLE_AUTO: none (the live rule's),
	                           * and always for c >= channels. A learned entry's from its
	                           * instrument bytes; a built-in's 0, or a kind-1 record's
	                           * (snd_learned_roles) in a table the firmware assembles */
} snd_engine;

extern const snd_engine snd_engine_table[];   /* the built-in engines (engines_builtin.c) */
extern const uint32_t snd_engine_count;
#define SND_ENGINE_BUILTIN_MAX 6u   /* snd_engine_count, as a constant (engines_builtin.c checks it) */
/* The name of table[index] (index < the table's count). */
const char *snd_engine_name(const snd_engine *table, uint32_t index);
/* The signature digest of entry en with its anchor at `anchor` in m's memory. */
uint64_t snd_engine_digest(const zx_machine *m, const snd_engine *en, uint16_t anchor);

/* core/sound/engines_test.c, compiled by the host build only (never the firmware's): the built-in
 * entries plus test fixtures' engines. */
extern const snd_engine snd_engine_test_table[];
extern const uint32_t snd_engine_test_count;
/* Also there: the misread table, entries whose readers are off, each for a table of its own: ROM
 * BEEP's with its period a semitone too long, then Test flips' with channel 1's period halved,
 * channel 0's halved (in a note whose channel 1 is slow) and channel 1's doubled (misreads). */
extern const snd_engine snd_engine_misread_table[];
extern const uint32_t snd_engine_misread_count;

/* Learned entries (learned.c): a game's music routines, learned by the personal-pack builder and
 * stored in the pack as data (docs/pack-format.md). Each entry is an snd_engine whose reader runs
 * small programs: bytecode for a stack machine of uint32_t values, at most 8 deep and
 * SND_PROG_MAX bytes long, that compute a channel's period (T-states x 256) or the note's timing
 * from memory (at the frame's end) and registers. */
#define SND_LEARNED_MAX 8u          /* learned entries per game */
#define SND_PROG_MAX 64u            /* bytes of a program */
#define SND_LEARNED_LEN_MAX 512u    /* signature bytes of a learned entry (the matching's work) */
#define SND_LEARNED_SITES_MAX 16u   /* sites of a learned entry */
#define SND_LEARNED_WORK_MAX 4096u  /* sites x signature bytes over a game's entries: the digest
                                     * bytes find() hashes for an OUT no entry is known at */

typedef struct {          /* a learned entry's programs and rules (snd_engine.data), pointing into its list */
	const uint8_t *prog[SND_TONAL];   /* per channel: its period; 0 (or silent) = the channel is silent */
	uint8_t prog_len[SND_TONAL];
	const uint8_t *timing;            /* the note's T-states so far, from the registers given; NULL: not known */
	uint8_t timing_len;
	uint16_t stale_from, stale_len;   /* frame-end pcs (from the anchor) where the memory is not to be read */
	uint8_t level[SND_TONAL];
} snd_learned_ext;

/* Runs the program p[0..len) with the machine m (memory at the frame's end), the anchor and the
 * registers ctx: 0 with the result in *out, 1 silent (a zero divisor or a zero popped by the
 * silence op), -1 invalid. */
int snd_prog_eval(const uint8_t *p, uint32_t len, const zx_machine *m, uint16_t anchor, const zx_regs *ctx, uint32_t *out);
/* Decodes a game's entry list (list[0..avail), 4-byte aligned) into en[0..*n) and ext[0..*n),
 * their arrays and programs pointing into list: ZX_OK, or ZX_ERR_FORMAT (then *n = 0) for any
 * bound or program that is not valid, or more than max entries. A learned entry's role[c] is its
 * instrument[c] (above SND_ROLE_PAD: SND_ROLE_AUTO). The list may also hold kind-1 records (roles
 * for a built-in entry), checked and counted like entries but decoded into no engine, so *n
 * counts the learned entries alone. */
int snd_learned_decode(const uint8_t *list, uint32_t avail, snd_engine *en, snd_learned_ext *ext, uint32_t max, uint32_t *n);
/* For each kind-1 record of a list snd_learned_decode accepted: sets role[] of the entries of
 * table[0..n) that are not learned and whose digest is the record's (roles above SND_ROLE_PAD, or
 * for channels past the record's or the entry's: SND_ROLE_AUTO). A record matching no such entry
 * changes nothing; a later record for the same digest wins. */
void snd_learned_roles(const uint8_t *list, uint32_t avail, snd_engine *table, uint32_t n);

#define SND_B_MATCHES 8u      /* anchors remembered */
#define SND_B_UNKNOWN 8u      /* OUT addresses remembered as matching no entry */
#define SND_B_HOLD 50u        /* frames an engine holds after its last edge (1 s: a tune's rests) */

typedef struct {          /* a remembered match */
	uint16_t anchor;
	uint8_t entry;        /* 1 + index in the table; 0 = free */
	uint8_t ok;           /* the signature matched in frame `frame` */
	uint32_t frame;       /* the frame it was last checked */
	uint32_t used;        /* the frame it was last used */
} snd_b_match;

typedef struct {          /* an OUT address that matched no entry (engines.c, find) */
	uint16_t pc;
	uint8_t valid;
	uint32_t sum;         /* the hash of the bytes the entries' signatures cover there, then */
	uint32_t used;        /* the frame it was last looked up */
} snd_b_unknown;

enum snd_b_kind { SND_B_NONE, SND_B_MINE, SND_B_FOREIGN };

typedef struct {          /* a channel's level as the check followed it (engines.c) */
	int32_t set;          /* when it took its level, T from the frame's start */
	int32_t obs;          /* the last time it was known */
	int32_t rise;         /* its last counted rise */
	uint8_t bit;          /* 0, 1; 2 = not known; 3 = entries with flips: seen only as a flip
	                       * hidden in the XOR, guessed (engines.c, cancelled) */
	uint8_t has_rise;
	uint8_t paired;       /* entries with flips: a pair of its flips counted since it was
	                       * followed afresh */
} snd_b_track;

typedef struct {          /* the check's state at the end of a frame, for the next */
	snd_b_track tr[SND_TONAL];
	int32_t t;            /* the last tonal edge (its inference waits for the next) */
	uint8_t chan, level, has;
} snd_b_carry;

typedef struct {          /* what B saw in one frame, when it arrived */
	uint8_t kind;         /* enum snd_b_kind: no edges, all an engine's, some from other code */
	uint8_t entry;        /* 1 + index of the engine (MINE; FOREIGN: whose edges end the frame) */
	uint8_t ok;           /* MINE: the voices passed the check against the edges (2: nothing in
	                       * the frame to check them by; the next frame's check decides) */
	uint8_t tonal;        /* tonal- or stop-site edges in the frame */
	uint8_t drum;         /* drum-site edges in the frame */
	uint8_t last_mine;    /* the frame's last edge is an engine's (a hold may follow) */
	uint8_t read;         /* v holds the voices of the frame's tonal edges */
	snd_voice v[SND_TONAL];   /* the voices at the frame's end */
	uint32_t t_row;       /* the first tonal edge with the voices v (where they change to them, if
	                       * they differ); NOWHERE: none */
	uint8_t carry;        /* cy: the channels at the frame's end, with the voices v, for the next */
	snd_b_carry cy;
	uint8_t step;         /* sliding: the voices were bv from the edge at t_step (a step before v;
	                       * min_len entries: bv silent, where the note before ended) */
	snd_voice bv[SND_TONAL];
	uint32_t t_step;
	uint8_t next;         /* the engine moved on to the voices nv at t_next, after its last edge
	                       * (played if the next frame is MINE and passes: nothing checks them here) */
	snd_voice nv[SND_TONAL];
	uint32_t t_next;
	uint32_t t_first, t_last;   /* the first and last tonal edge */
	uint32_t d_first, d_last;   /* the first and last drum edge */
	/* entries with min_len: */
	uint8_t whole;        /* the note at the frame's end lasts at least min_len */
	uint8_t cont;         /* the frame's first edges carry on the note the frame before ended with */
} snd_b_rec;

typedef struct snd_b {        /* method B's state */
	const snd_engine *table;
	uint32_t n;               /* entries in table */
	uint32_t frames;          /* frames fed since init */
	snd_b_match match[SND_B_MATCHES];
	snd_b_unknown unknown[SND_B_UNKNOWN];
	uint16_t back, ahead;     /* the bytes any signature with a site at an address covers: from
	                           * back before it to ahead after (the table's) */
	snd_b_rec rec[SND_HIST];  /* the frames fed last, at index frame % SND_HIST */
	snd_voice lastv[SND_TONAL];   /* the voices last read (or moved on to), of engine last_entry */
	uint8_t last_entry;
	/* the decisions: the voices and the drum as the last decided frame left them */
	snd_voice tone[SND_TONAL];
	uint8_t drum;
	uint8_t engine;           /* 1 + index of the engine holding, 0 = none */
	uint32_t held;            /* frames decided since the engine's last edge */
	uint32_t carry[SND_TONAL + 1u];   /* offs falling past the frame: T-state in the next, or ~0 */
} snd_b;

void snd_b_init(snd_b *b, const snd_engine *table, uint32_t n);
/* Feeds frame t (zm right after zx_run_frame; zm may be NULL: nothing is B's); fills *out with
 * the decision for frame t - SND_DELAY_FRAMES. A frame an engine claims carries that table
 * entry's stored roles (snd_engine.role), any other frame SND_ROLE_AUTO for every voice. */
void snd_b_frame(snd_b *b, const zx_machine *zm, const zx_edge *e, uint32_t n, uint8_t start_level, snd_frame *out);

/* ---- effects follower (fx_read.c, fx.c) --------------------------------
 *
 * In FM: engines, the frames the mixer gives to the delayed beeper are re-voiced as FM: a tone
 * that follows the beeper's pitch (the lead's sin(p + 1.3 sin 2p)), a tick at an edge after
 * silence, brief noise for irregular edges. The reader (fx_read.c, once a frame, flash on the
 * device, reached through `read`) follows the decoder's stored speaker transitions of the frame
 * the mixer is about to play (t - 3) and turns them into a short event list at sample positions;
 * it classifies frame t on its own as it arrives, so the frames after the one played are known.
 * The voice (fx.c, per sample, SRAM) renders the events. The mixer puts the result in place of
 * the delayed beeper under a crossfade gain, where the gate says PLAY.
 *
 * The rules, in T-states: a gap of 30,000 or more starts a new sound (a tick, the history
 * cleared). The last 10 half periods (gaps) are kept; over the last up to 6 full periods
 * (adjacent pairs) the median and the spread (max - min) / median: under 4 half periods, or a
 * spread under 0.3, is a tone of the median's pitch (at most 12 kHz), else noise at the rate of
 * the last up to 8 gaps' mean. It sounds while there are 2 half periods or more and the last
 * transition is under min(2.5 x the largest of the last 4 half periods, 30,000) ago.
 * Gates: a frame with more edges than the decoder keeps is busy; one whose gaps are under 200 T
 * for more than 40% is two-voice music; a frame is irregular when most of its windows (from its
 * 4th transition, a history of its own) read noise, and one in a run of 3 or more irregular frames
 * stays on the beeper. */

/* Why the frame played last did or did not go to the follower, as the field snd_fx.gate holds it:
 * SND_FX_OFF is effects off or a mode other than FM: engines (the mixer sets it); SND_FX_B_REJECTED
 * is method B's own frame that its check refused, in a run of under 3 such frames (a brief
 * rejection inside a known tune stays on the beeper; a longer run goes through the other gates). While the mixer
 * plays FM music the field keeps the reader's gate: SndMixer::fx_gate (host) reports OFF for it. */
enum snd_fx_gate { SND_FX_PLAY, SND_FX_OFF, SND_FX_TWO_VOICE, SND_FX_BUSY, SND_FX_IRREGULAR, SND_FX_B_REJECTED };
/* What the follower sounds. */
enum snd_fx_sound { SND_FX_SILENT, SND_FX_TONE, SND_FX_NOISE };

/* Events per frame: on overflow intermediate pitch updates are dropped, the last kept. */
#define SND_FX_EVENTS 32u
#define SND_FX_HALVES 10u   /* half periods kept */

typedef struct {          /* the follower from a sample of the frame on */
	uint32_t step;        /* TONE: the tone's phase step per sample (2^32 = a cycle); NOISE: the
	                       * sample-and-hold's (2^32 = a new value) */
	uint16_t at;          /* the sample it starts on */
	uint16_t k;           /* NOISE: the low-pass coefficient, Q15 */
	uint8_t sound;        /* enum snd_fx_sound */
	uint8_t tick;         /* a tick starts here */
} snd_fx_event;

typedef struct snd_fx {
	/* the reader, set by snd_fx_init: snd_fx_read */
	void (*read)(struct snd_fx *f, const snd_decoder *d, const snd_b *b, uint32_t n);
	/* constants (snd_fx_init) */
	uint32_t rate_num, rate_den;
	uint32_t kstep;       /* 3.5 MHz x 2^20 / rate: a period of P T-states steps kstep x 2^12 / P */
	uint32_t kx;          /* the noise's low-pass: 2 pi fc / rate (Q16) = kx / mean gap ... */
	uint32_t x_lo, x_hi;  /* ... within these (fc 150 Hz - 8 kHz) */
	uint32_t tick_inc;    /* the tick's phase step (1.5 kHz) */
	/* the reader's state */
	uint32_t last;        /* the last transition: absolute T-state (frame x ZX_FRAME_TSTATES + T, wrapping) */
	uint32_t stop;        /* while sounding: when it stops if no transition comes first (absolute) */
	uint32_t step;        /* the sound's step and low-pass, as in snd_fx_event */
	uint32_t irr;         /* bit k: frame t - k is irregular (t: the frame fed last) */
	uint32_t rej;         /* bit k: frame t - 3 - k was method B's refused frame (as it was played) */
	uint16_t half[SND_FX_HALVES];   /* the last half periods, oldest first */
	uint16_t k;
	uint8_t n_half;
	uint8_t have_last;    /* `last` is under 30,000 T before the frame's end: the next transition continues its sound */
	uint8_t sound;        /* enum snd_fx_sound from `last` (until `stop`) */
	uint8_t gate;         /* enum snd_fx_gate of the frame the events are for */
	uint8_t n_ev;
	snd_fx_event ev[SND_FX_EVENTS];   /* the frame the mixer plays next: ev[0] at sample 0 */
	/* the voice (fx.c); envelopes Q16, amplitudes: out = x (Q15) * amp >> 15 */
	uint32_t tick_acc;    /* 8 kHz tick, as fm.c's */
	uint32_t phase, cur, target;    /* the tone: phase, step played, step it glides to */
	uint32_t tick_ph;
	uint32_t hold_ph, nstep, seed;  /* the noise: sample-and-hold phase and step, xorshift32 */
	int32_t hold, lp;     /* the value held, low-passed (Q15) */
	int32_t env_tone, env_noise, env_tick;
	int32_t amp_tone, amp_noise, amp_tick;
	uint16_t lp_k;
	uint8_t on;           /* enum snd_fx_sound playing */
	uint8_t idle;         /* the mixer skipped the voice this frame: the reader restarts its envelopes */
	int16_t buf[SND_MAX_FRAME_SAMPLES];   /* the frame rendered, for the mixer */
} snd_fx;

/* Constants for the rate, the reader set, the follower silent (sound_init.c, flash). */
void snd_fx_init(snd_fx *f, uint32_t rate_num, uint32_t rate_den);
/* Called once a frame after the decoder and method B have taken frame t (n: the samples the mixer
 * plays for it): classifies frame t, then reads frame t - 3 into the event list and sets the gate
 * (SND_FX_OFF before frame t - 3 exists). Flash on the device: it calls nothing in the other sound
 * files. b: method B, for its records (unused yet). */
void snd_fx_read(snd_fx *f, const snd_decoder *d, const snd_b *b, uint32_t n);
/* Renders the event list into n samples (SRAM). */
void snd_fx_render(snd_fx *f, int16_t *out, uint32_t n);

/* ---- mixer (mixer.c) --------------------------------------------------- */

#define SND_RING_SAMPLES (SND_DELAY_FRAMES * SND_MAX_FRAME_SAMPLES)

typedef struct snd_mixer {   /* delay rings, hysteresis, crossfade; owns both methods and an fm */
	snd_decoder dec;
	snd_b b;
	snd_fm fm;
	snd_roles roles;                  /* the live rule for the frames the synth renders */
	snd_fx fx;                        /* the effects follower */
	int16_t ring[SND_RING_SAMPLES];   /* beeper samples, SND_DELAY_FRAMES frames late */
	uint32_t rd, wr, fill;            /* ring read and write index, samples held */
	uint32_t mute;                    /* samples to pop before the delayed path may fade in */
	int32_t live_g, del_g, fm_g;      /* Q15 gains (32768 = 1): live beeper, delayed path, FM in it */
	int32_t step;                     /* gain change per sample of a crossfade */
	int32_t fx_g;                     /* Q15: the follower in place of the delayed beeper */
	uint8_t effects;                  /* 1: FM: engines re-voices the beeper (1 after init; the device sets it) */
	uint8_t primed;                   /* frames pushed, up to SND_DELAY_FRAMES */
	uint8_t mode;                     /* mode of the previous frame */
	uint8_t fm_choice;                /* the FM mode in use or last used (FM_A after init) */
	uint8_t started;                  /* a frame has run since init */
	uint8_t fm_on;                    /* the hysteresis' decision: FM */
	uint8_t run;                      /* frames in a row disagreeing with fm_on */
} snd_mixer;

/* Uses the built-in engine table for method B. */
void snd_mixer_init(snd_mixer *m, uint32_t rate_num, uint32_t rate_den);
/* Re-initialises method B with another engine table (tests). */
void snd_mixer_set_engines(snd_mixer *m, const snd_engine *table, uint32_t n);
/* zm: the machine right after zx_run_frame (NULL: nothing is method B's); beeper: this frame's n
 * beeper samples (from zx_beeper_render). Writes n samples to out: SND_MODE_BEEPER -> a copy of
 * beeper (no delay); FM modes -> 3-frame-delayed FM or beeper, FM_A playing method A's
 * decisions, FM_B method B's.
 *
 * The delay is a sample FIFO holding the first SND_DELAY_FRAMES frames' samples
 * after init, so the delayed stream is continuous (no sample repeated or
 * dropped) even though frames have 880 or 881 samples; frame t's output starts
 * within one sample of frame t - 3's first sample. The FIFO and both methods are
 * fed in every mode, so an FM mode has its look-ahead at a switch.
 *
 * One hysteresis: FM after SND_ENTER_FRAMES confident decisions in a row (FM_B: after one), the
 * beeper after SND_LEAVE_FRAMES unconfident ones; in Beeper mode it follows the last FM mode's
 * method (FM_A's after init). An unconfident decision that does not leave FM (a lone one inside
 * music: a note boundary, a partly explained frame) keeps the synth's voices as they are.
 *
 * Mode switches: Beeper -> FM fades the live beeper out, plays silence, and
 * fades the delayed path in at the switch frame's first sample (60 ms later);
 * FM -> Beeper crossfades from the delayed path to the live beeper (the last 3
 * frames of delayed material are dropped); FM_A <-> FM_B keeps the delayed path
 * and the synth running, only FM and the delayed beeper crossfade if the decision
 * changes. n above SND_MAX_FRAME_SAMPLES: the beeper is copied and the mixer is
 * left as it was.
 *
 * Every frame the synth renders first goes through the live rule (snd_roles_frame), which gives
 * its voices without a role theirs; an FM turn-on starts the rule afresh, with the synth, and an
 * FM_A <-> FM_B switch starts it afresh alone (method A's slots are not method B's channels).
 *
 * The effects follower reads every frame in every mode; in FM_B with `effects` on it replaces the
 * delayed beeper where its gate says SND_FX_PLAY, under a gain ramping like the others (fx_g), so
 * with effects off, and in the other modes once that ramp is down, the output is as without it. */
void snd_mixer_frame(snd_mixer *m, uint8_t mode, const zx_machine *zm, const zx_edge *e, uint32_t n_edges,
                     uint8_t start_level, const int16_t *beeper, uint32_t n, int16_t *out);

#endif
