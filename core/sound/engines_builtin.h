/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B's built-in engine entries, as initializers (SND_ENGINE_*) with their readers and
 * tables: engines_builtin.c builds the firmware's table from them, the host's engines_test.c its
 * test table (the built-in entries plus the test fixtures'). Include once per file. */

#ifndef ENGINES_BUILTIN_H
#define ENGINES_BUILTIN_H

#include "sound.h"

/* Castaway's title tune (anchor C864h in the pack, the EXX before the loop; the loop C868h-C8B6h,
 * 154 T a pass plus contention, 156.6 T on average). Channel 0's OUT (C87Ch) gives its level,
 * channel 1's (C886h) its own: two square channels, each a DEC counter reloaded from N (the
 * operands at +09h / +16h); the phase wraps at the operand at +0Dh / +1Dh, plus 1 (1: a square,
 * 3: ROM bytes 0-3, an octave down); the mask at +11h / +21h is 0 for a rest.
 * f = 3.5 MHz / ((wrap + 1) x N x 156.6). IX counts passes (IXH from the operand at +03h, IXL
 * from 0); a slide (its JR at +29h / +37h with offset 0) changes N every 256 passes by the
 * operand at +3Ch / +2Eh (N - s with a borrow, else FFh). The note's setup (C7EAh-C867h)
 * rewrites the operands between notes. */
#define CW_LOOP_Q8 40090u   /* 156.6 x 256 */

/* Passes of the note so far, from IX (the whole note, or since the last slide step). */
static uint32_t cw_passes(const zx_machine *m, uint16_t a, uint32_t ix, int whole)
{
	uint32_t passes = (0u - (ix & 0xFFu)) & 0xFFu;

	if (whole || (zx_peek(m, (uint16_t)(a + 0x29)) && zx_peek(m, (uint16_t)(a + 0x37))))   /* no slide */
		passes += ((zx_peek(m, (uint16_t)(a + 0x03)) - (ix >> 8)) & 0xFFu) << 8;
	return passes;
}

static void read_castaway(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	static const uint8_t n_at[2] = { 0x09, 0x16 }, wrap_at[2] = { 0x0D, 0x1D }, mask_at[2] = { 0x11, 0x21 };
	static const uint8_t slide_at[2] = { 0x3C, 0x2E }, jr_at[2] = { 0x37, 0x29 };
	uint32_t c, passes = cw_passes(m, a, r->ix, 0), row = cw_passes(m, a, r->ix, 1), now_passes;
	zx_regs now;

	(void)en;
	(void)site;
	for (c = 0; c < 2; c++) {
		uint32_t n = zx_peek(m, (uint16_t)(a + n_at[c]));
		uint32_t wrap = (uint32_t)zx_peek(m, (uint16_t)(a + wrap_at[c])) + 1u;
		uint32_t back = n;
		if (!zx_peek(m, (uint16_t)(a + jr_at[c]))) {   /* sliding: each step N = N - s with a borrow, else FFh */
			uint32_t sl = zx_peek(m, (uint16_t)(a + slide_at[c]));
			back = (n + sl) & 0xFFu;
			if (n == 0xFF || back >= sl) back = 0;   /* (clamped: not known) */
		}
		if (zx_peek(m, (uint16_t)(a + mask_at[c])) && n) {
			out->v[c].period_q8 = wrap * n * CW_LOOP_Q8;
			out->v[c].level = 200;
			out->back[c].period_q8 = wrap * back * CW_LOOP_Q8;
			out->back[c].level = 200;
		}
	}
	/* the operands are the note at the frame's end: not to be read if the loop is not running
	 * then (the next note's setup), the next note's if it began after the last edge */
	zx_get_regs(m, &now);
	now_passes = cw_passes(m, a, now.ix, 0);
	if (!zx_peek(m, (uint16_t)(a + 0x29)) || !zx_peek(m, (uint16_t)(a + 0x37))) out->step = 256u * CW_LOOP_Q8 >> 8;
	if ((uint16_t)(now.pc - a - 0x04) >= 0x4F) {
		out->stale = 2;
	} else if (now_passes < passes) {
		out->stale = 1;
		passes = now_passes;
	}
	out->since = passes * CW_LOOP_Q8 >> 8;
	out->row = row * CW_LOOP_Q8 >> 8;
}

static const uint16_t cw_wild[] = { 0x03, 0x09, 0x0D, 0x11, 0x16, 0x1D, 0x21, 0x29, 0x2E, 0x37, 0x3C };
static const uint16_t cw_reloc[] = { 0x06, 0x2B, 0x34, 0x39, 0x42, 0x47, 0x4B, 0x4E, 0x51 };
static const snd_engine_site cw_sites[] = { { 0x18, SND_SITE_TONAL, 0 }, { 0x22, SND_SITE_TONAL, 1 } };
static const uint16_t cw_pos[] = { 0, 44 };
_Static_assert(sizeof cw_pos / sizeof cw_pos[0] == 2, "a pos per channel");

#define SND_ENGINE_CASTAWAY {                                                                          \
	"Castaway", 0x45eefa6915ef8919ull, 0x53, sizeof cw_wild / sizeof cw_wild[0], sizeof cw_reloc / sizeof cw_reloc[0], \
	sizeof cw_sites / sizeof cw_sites[0], 2, 156, cw_wild, cw_reloc, cw_sites, cw_pos, read_castaway, 0, 0, 0, 0, 0, { 0 }, \
}

/* Escape from Colony 8's music (anchor 9493h in the pack: the tone-burst drum, the ROM-noise drum,
 * the pattern step and the music loop 94FCh-951Eh, 153 T a pass plus contention, 155.9 T on
 * average). Three 16-bit phase accumulators with steps BC, DE (each set swapped by the loop's
 * EXX: at the first two OUTs the alternate set is in use) and SP; a channel is high while its
 * accumulator's high byte is below its duty threshold (the operands at +6Dh / +77h / +7Fh, 0 =
 * silent). f = step x 3.5 MHz / (155.9 x 65536). The main DE counts the note's passes down from
 * the operand at +59h; the steps are loaded from the operands at +5Ch / +63h / +66h. The OUT after
 * the loop (+8Eh) silences the speaker; the drums drive it from +13h (a tone burst) and +27h /
 * +2Dh (ROM noise) while the loop is stopped. */
#define C8_LOOP_Q8 39910u          /* 155.9 x 256 */
#define C8_PERIOD_K 2615568998u    /* 155.9 x 256 x 65536: period_q8 = C8_PERIOD_K / step */

/* Passes left of a DEC E / JP NZ / DEC D / JP NZ count at D, E. */
static uint32_t c8_left(uint32_t de)
{
	uint32_t d = de >> 8, e = de & 0xFFu;

	return (e ? e : 256u) + 256u * (d ? d - 1u : 255u);
}

static void read_colony8(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	static const uint8_t thr_at[3] = { 0x6D, 0x77, 0x7F };
	static const uint8_t op_at[3] = { 0x5C, 0x63, 0x66 };
	uint32_t c, step[3], de, de0, left0, left;
	zx_regs now;

	(void)en;
	zx_get_regs(m, &now);
	de0 = zx_peek(m, (uint16_t)(a + 0x59)) | (uint32_t)zx_peek(m, (uint16_t)(a + 0x5A)) << 8;
	left0 = c8_left(de0);
	if ((uint16_t)(now.pc - a - 0x69) >= 0x23) {   /* the loop is not running: between notes, a drum */
		out->stale = 2;
		return;
	}
	if (site < 2) {   /* the alternate set is in use */
		step[0] = r->bc_;
		step[1] = r->de;
		de = r->de_;
	} else {
		step[0] = r->bc;
		step[1] = r->de_;
		de = r->de;
	}
	step[2] = r->sp;
	left = c8_left(de);
	for (c = 0; c < 3; c++)
		if ((zx_peek(m, (uint16_t)(a + op_at[c])) | (uint32_t)zx_peek(m, (uint16_t)(a + op_at[c] + 1u)) << 8) != step[c]) out->stale = 1;
	if (left0 < left) out->stale = 1;
	if (out->stale) {   /* a note began after the last edge: its voices from the frame's end */
		if ((uint16_t)(now.pc - a - 0x6C) < 0x17) {   /* 94FFh-9515h: the alternate set is in use */
			step[0] = now.bc_;
			step[1] = now.de;
			de = now.de_;
		} else {
			step[0] = now.bc;
			step[1] = now.de_;
			de = now.de;
		}
		step[2] = now.sp;
		left = c8_left(de);
	}
	for (c = 0; c < 3; c++) {
		uint32_t thr = zx_peek(m, (uint16_t)(a + thr_at[c]));
		if (thr && step[c]) {
			uint32_t duty = thr <= 128u ? thr : 256u - thr;
			out->v[c].period_q8 = C8_PERIOD_K / step[c];
			out->v[c].level = (uint8_t)(96u + duty * 159u / 128u);
		}
	}
	out->since = out->row = left0 >= left ? (left0 - left) * C8_LOOP_Q8 >> 8 : 0;
}

static const uint16_t c8_wild[] = { 0x3B, 0x3C, 0x59, 0x5A, 0x5C, 0x5D, 0x5F, 0x60, 0x63, 0x64, 0x66, 0x67, 0x6D, 0x77, 0x7F };
static const uint16_t c8_reloc[] = { 0x19, 0x1C, 0x38, 0x44, 0x47, 0x4F, 0x53, 0x56, 0x87, 0x8B, 0x91 };
static const snd_engine_site c8_sites[] = {
	{ 0x72, SND_SITE_TONAL, 0 }, { 0x7A, SND_SITE_TONAL, 1 }, { 0x84, SND_SITE_TONAL, 2 }, { 0x8E, SND_SITE_STOP, 0 },
	{ 0x13, SND_SITE_DRUM, 0 },  { 0x27, SND_SITE_DRUM, 0 },  { 0x2D, SND_SITE_DRUM, 0 },
};
static const uint16_t c8_pos[] = { 0, 34, 83 };
_Static_assert(sizeof c8_pos / sizeof c8_pos[0] == 3, "a pos per channel");

#define SND_ENGINE_COLONY8 {                                                                           \
	"Colony 8", 0xae5de8a81485ea1full, 0x93, sizeof c8_wild / sizeof c8_wild[0], sizeof c8_reloc / sizeof c8_reloc[0], \
	sizeof c8_sites / sizeof c8_sites[0], 3, 155, c8_wild, c8_reloc, c8_sites, c8_pos, read_colony8, 200, 0, 0, 0, 0, { 0 }, \
}

/* The 48K ROM's BEEPER (anchor 03B5h, 43h bytes to its RET; BASIC's BEEP and the key click call
 * it, and many games): HL the half-period count, DE the cycles less one. Its setup shifts L right
 * twice and enters a slide of three NOPs at IX = +1Ch + s, s = ~L & 3, so at its one OUT (+2Ch)
 * H, L (shifted) and IX give the pitch. From one OUT to the next, either way round (the path
 * after a high OUT is 16 T shorter, its first DEC C loop one turn longer): 60 T after a low OUT,
 * 4 for each NOP from IX on (3 - s), 8 (INC B, INC C), 16 L + 11 the first DEC C loop, 21 the
 * outer loop's pass, H x 1024 its passes of 3Fh turns, 18 (XOR, OUT): a half-period of
 * 130 + 16 L + 1024 H - 4 s T, which is 118 + 4 HL for the HL BEEPER was called with. The cycles left
 * are DE at the OUT (DEC DE follows the low OUT), so the note goes on (2 DE + 1) half-periods
 * after a high OUT (bit 4 of A), 2 DE after a low one, and ends half a period after that. The
 * absolute operands at +0Eh (the slide) and +28h (the loop) move with the code. */
static void read_beep(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	const uint32_t slide = (uint16_t)(r->ix - a - 0x1C), de = r->de < 4096u ? r->de : 4096u;
	uint32_t half;

	(void)en;
	(void)m;
	(void)site;
	if (slide > 3) return;   /* not BEEPER's state: no note */
	half = 130u + 16u * (r->hl & 0xFFu) + 1024u * (r->hl >> 8) - 4u * slide;
	out->v[0].period_q8 = half << 9;
	out->v[0].level = 200;
	out->left = (2u * de + 1u + ((r->af >> 12) & 1u)) * half;   /* (DE capped: long enough) */
}

static const uint16_t beep_reloc[] = { 0x0E, 0x28 };
static const snd_engine_site beep_sites[] = { { 0x2C, SND_SITE_TONAL, 0 } };
static const uint16_t beep_pos[] = { 0 };

#define SND_ENGINE_ROM_BEEP_READ(rd) {                                                                 \
	"ROM BEEP", 0x51a995b5e20f53abull, 0x43, 0, 2, 1, 1, 0, 0, beep_reloc, beep_sites, beep_pos, rd, 0, 0, 70000u, 0, 0, { 0 }, \
}
#define SND_ENGINE_ROM_BEEP SND_ENGINE_ROM_BEEP_READ(read_beep)   /* min_len 70,000 T: 20 ms */

/* The little-endian word at a. */
static uint32_t peek16(const zx_machine *m, uint32_t a)
{
	return zx_peek(m, (uint16_t)a) | (uint32_t)zx_peek(m, (uint16_t)(a + 1u)) << 8;
}

/* Cave Runner's title tune (anchor 92DCh, the note's setup and its loop to 930Eh). IY points at
 * the note [length, N1, N2]; each pass OUTs A from +1Ch, D and E count down from N1 and N2 (0:
 * 256) and each that reaches 0 is reloaded and flips the speaker (XOR A, 18h): two channels
 * flipping the one OUT. A pass is 56 T, 11 more every 256 passes (B wraps), and 21 more for each
 * counter reloaded in it, plus about 2 T of the OUT's contention after each reload (56 T is a
 * whole number of contention cycles, so the OUT keeps its place in them until a reload moves
 * it). A channel's half-period is n passes, in which the other (n') reloads n / n' times on
 * average: half = (56 + 11/256) n + 23 (1 + n / n') T, f = 3.5 MHz / (2 x half) (within 0.03% of
 * the spacing of its flips). B (from 0) and C (from the length) count the note's passes; the
 * CALLs at +0Eh and +17h move with the code. */
static void read_mm_title(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	const uint32_t k = 256u * ((zx_peek(m, r->iy) - r->bc) & 0xFFu) + ((0u - (r->bc >> 8)) & 0xFFu);   /* passes */
	uint32_t c, n[2], q = 14336u * k;   /* 56 x 256 per pass */

	(void)en;
	(void)a;
	(void)site;
	for (c = 0; c < 2; c++) {
		n[c] = zx_peek(m, (uint16_t)(r->iy + 1u + c));
		if (!n[c]) n[c] = 256u;
	}
	for (c = 0; c < 2; c++) {
		out->v[c].period_q8 = 28694u * n[c] + 11776u + 11776u * n[c] / n[1u - c];   /* 2 x 256 x half */
		out->v[c].level = 200;
		q += 5376u * (k / n[c]);   /* 21 x 256 per reload */
	}
	out->since = out->row = (q + 154u * k) >> 8;   /* (0.6 T of contention a pass) */
}

static const uint16_t mmt_reloc[] = { 0x0F, 0x18 };
static const snd_engine_site mmt_sites[] = { { 0x1C, SND_SITE_TONAL, SND_CHAN_ANY } };

#define SND_ENGINE_MM_TITLE {                                                                          \
	"Cave Runner title", 0x21ad39e14d5cbeddull, 0x33, 0, 2, 1, 2, 0, 0, mmt_reloc, mmt_sites, beep_pos, read_mm_title, 0, 1, 0, 0, 0, { 0 }, \
}

/* Cave Runner's in-game tune (anchor 881Fh, the music flag test to the burst loop's end at 885Eh).
 * Once a pass of the game's main loop, unless bit 1 of the flag (the operand at +01h) is set
 * (music off), the note index (the operand at +1Ch) steps and one burst of 768 passes of 40 T
 * plays its note E, the tune table's (the operand at +29h) byte (index & 7Eh) / 2: one channel
 * flipping the OUT at +33h every E passes, f = 3.5 MHz / (2 x E x 40). Music off: no voice, so
 * its edges fail the check (the game's sounds are the beeper's). */
static void read_mm_game(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	uint32_t e;

	(void)en;
	(void)site;
	(void)r;
	if (zx_peek(m, (uint16_t)peek16(m, a + 0x01u)) & 2u) return;
	e = zx_peek(m, (uint16_t)(peek16(m, a + 0x29u) + ((zx_peek(m, (uint16_t)peek16(m, a + 0x1Cu)) & 0x7Eu) >> 1)));
	out->v[0].period_q8 = e * 20480u;   /* 2 x 40 x 256 */
	out->v[0].level = e ? 200 : 0;
}

static const uint16_t mmg_reloc[] = { 0x01, 0x1C, 0x20, 0x29, 0x2D };
static const snd_engine_site mmg_sites[] = { { 0x33, SND_SITE_TONAL, 0 } };

#define SND_ENGINE_MM_GAME {                                                                           \
	"Cave Runner in game", 0xa3a926aa54f53cc1ull, 0x40, 0, 5, 1, 1, 0, 0, mmg_reloc, mmg_sites, beep_pos, read_mm_game, 0, 1, 0, 0, 0, { 0 }, \
}

/* Unison's music (anchor 81B4h, the melody to the last drum's end at 82A6h). The melody (+12h):
 * BC counts down from N (the operand at +19h) and DE from N - 1 (+2Bh), each reload ADDs 10h to
 * A' and OUTs it (+1Eh, +30h), so each flips the speaker: one note doubled a step apart. A pass
 * is 48 T, and 46 more for each counter reloaded in it, plus about 1.5 T of the OUT's contention
 * (48 T is a whole number of contention cycles, so the OUT keeps its place in them until a
 * reload moves it). A channel's half-period is n passes, in which the other (n') reloads n / n'
 * times on average: half = 48 n + 47.5 (1 + n / n') T, f = 3.5 MHz / (2 x half) (within 0.04% of
 * the spacing of its flips). The interrupt routine (+36h) sequences it and runs the four drums
 * (+A3h, +B6h, +CAh, +DEh), each OUTing from two sites, until the next interrupt. */
static void read_unison(const zx_machine *m, const snd_engine *en, uint16_t a, uint32_t site, const zx_regs *r, snd_b_read *out)
{
	uint32_t c, n[2];

	(void)en;
	(void)site;
	(void)r;
	for (c = 0; c < 2; c++)
		n[c] = peek16(m, a + 0x19u + 0x12u * c);
	for (c = 0; c < 2; c++) {
		const uint32_t o = n[1u - c] ? n[1u - c] : 65536u;   /* (0: the counter runs 65,536 passes) */
		if (!n[c]) continue;   /* silent */
		out->v[c].period_q8 = 24576u * n[c] + 24320u + 24320u * n[c] / o;   /* 2 x 256 x half */
		out->v[c].level = 200;
	}
}

static const uint16_t unison_wild[] = { 0x19, 0x1A, 0x2B, 0x2C };
static const uint16_t unison_reloc[] = { 0x08, 0x0C, 0x16, 0x22, 0x28, 0x34, 0x38, 0x3E, 0x42,
                                        0x49, 0x61, 0x68, 0x6E, 0x76, 0x7F, 0x89, 0x96, 0xA1 };
static const snd_engine_site unison_sites[] = {
	{ 0x1E, SND_SITE_TONAL, 0 }, { 0x30, SND_SITE_TONAL, 1 }, { 0xAA, SND_SITE_DRUM, 0 }, { 0xB2, SND_SITE_DRUM, 0 },
	{ 0xBD, SND_SITE_DRUM, 0 },  { 0xC6, SND_SITE_DRUM, 0 },  { 0xD2, SND_SITE_DRUM, 0 }, { 0xD9, SND_SITE_DRUM, 0 },
	{ 0xE5, SND_SITE_DRUM, 0 },  { 0xEF, SND_SITE_DRUM, 0 },
};

#define SND_ENGINE_UNISON {                                                                             \
	"Unison", 0xf45d15034e0cc08dull, 0xF3, 4, sizeof unison_reloc / sizeof unison_reloc[0], sizeof unison_sites / sizeof unison_sites[0], 2, 0, \
	unison_wild, unison_reloc, unison_sites, beep_pos, read_unison, 200, 1, 0, 0, 0, { 0 }, \
}

#define SND_ENGINES_BUILTIN SND_ENGINE_CASTAWAY, SND_ENGINE_COLONY8, SND_ENGINE_ROM_BEEP, SND_ENGINE_MM_TITLE, \
	SND_ENGINE_MM_GAME, SND_ENGINE_UNISON

#endif
