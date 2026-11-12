/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Chooses which LCD rows to send each frame, oldest pending first, within a
 * per-frame byte budget. */

#ifndef APP_LCDSCHED_H
#define APP_LCDSCHED_H

#include <stdint.h>

#include "pixels.h"

#define LCD_BAND_ROWS 8u
#define LCD_WINDOW_COST 11u /* CASET+RASET+RAMWR bytes per band */

typedef struct {
	uint32_t frames, row_sends, pending_frames_sum, bytes, bands;
	uint32_t max_age; /* the longest a row waited, in frames (sent in its marking frame = 1) */
} lcd_stats;

typedef struct lcd_sched {
	uint32_t pending_since[LCD_ROWS]; /* frame at which the row became pending; valid while pending[row] */
	uint8_t pending[LCD_ROWS];
	uint32_t frame;
	uint32_t budget; /* bytes left this frame */
	uint32_t row_bytes;
	lcd_stats stats;
} lcd_sched;

void lcd_sched_init(lcd_sched *s, unsigned row_bytes);
void lcd_sched_mark(lcd_sched *s, unsigned row, uint32_t frame); /* keeps the older pending_since */
void lcd_sched_mark_all(lcd_sched *s, uint32_t frame);
void lcd_sched_begin_frame(lcd_sched *s, uint32_t frame, uint32_t budget_bytes);
int lcd_sched_next_band(lcd_sched *s, unsigned *row0, unsigned *rows); /* 1 = band reserved */
unsigned lcd_sched_pending(const lcd_sched *s);
uint32_t lcd_budget_bytes(unsigned divider); /* 150000 / (divider + 1) */

#endif
