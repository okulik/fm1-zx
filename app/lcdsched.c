/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "lcdsched.h"

#include <string.h>

void lcd_sched_init(lcd_sched *s, unsigned row_bytes)
{
	memset(s, 0, sizeof *s);
	s->row_bytes = row_bytes;
}

/* Assumes frame numbers never decrease between calls. */
void lcd_sched_mark(lcd_sched *s, unsigned row, uint32_t frame)
{
	if (row >= LCD_ROWS)
		return;
	if (!s->pending[row]) {
		s->pending[row] = 1;
		s->pending_since[row] = frame;
	}
}

void lcd_sched_mark_all(lcd_sched *s, uint32_t frame)
{
	unsigned r;
	for (r = 0; r < LCD_ROWS; r++)
		lcd_sched_mark(s, r, frame);
}

void lcd_sched_begin_frame(lcd_sched *s, uint32_t frame, uint32_t budget_bytes)
{
	s->frame = frame;
	s->budget = budget_bytes;
	s->stats.frames++;
}

int lcd_sched_next_band(lcd_sched *s, unsigned *row0, unsigned *rows)
{
	unsigned r, first = LCD_ROWS, n, fit;
	uint32_t cost;

	for (r = 0; r < LCD_ROWS; r++) {
		if (!s->pending[r])
			continue;
		if (first == LCD_ROWS || s->pending_since[r] < s->pending_since[first])
			first = r;
	}
	if (first == LCD_ROWS || s->budget < LCD_WINDOW_COST + s->row_bytes)
		return 0;

	n = 1;
	while (n < LCD_BAND_ROWS && first + n < LCD_ROWS && s->pending[first + n])
		n++;
	fit = (s->budget - LCD_WINDOW_COST) / s->row_bytes;
	if (n > fit)
		n = fit;

	cost = n * s->row_bytes + LCD_WINDOW_COST;
	s->budget -= cost;
	for (r = first; r < first + n; r++) {
		s->pending[r] = 0;
		s->stats.pending_frames_sum += s->frame - s->pending_since[r] + 1u;
	}
	s->stats.row_sends += n;
	s->stats.bytes += cost;
	s->stats.bands++;
	*row0 = first;
	*rows = n;
	return 1;
}

unsigned lcd_sched_pending(const lcd_sched *s)
{
	unsigned r, n = 0;
	for (r = 0; r < LCD_ROWS; r++)
		n += s->pending[r];
	return n;
}

uint32_t lcd_budget_bytes(unsigned divider)
{
	return 150000u / (divider + 1u);
}
