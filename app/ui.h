/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* A 30x30 text grid of 8x8 cells (the ZX ROM font) that fills the 240x240 LCD.
 * There is exactly one grid, held in static state inside ui.c; the device layer composes LCD
 * rows from it with ui_compose_row and sends the rows ui_dirty_rows reports. */
#ifndef FMZX_UI_H
#define FMZX_UI_H

#include <stdint.h>
#include "pixels.h"

#define UI_COLS 30u
#define UI_ROWS 30u

/* Copies the font (ROM 0x3D00, chars 32-127) and clears the grid to white on black; every row
 * counts as changed afterwards. `rom` is the 16 KiB 48K ROM. */
void ui_init(const uint8_t *rom);
/* Fills every cell with a space. Colours are palette indices 0-15 (8-15 bright). */
void ui_clear(uint8_t ink, uint8_t paper);
/* Draws `s` from cell (col, row); clipped at the grid edge. Chars outside 32-127 show as '?'. */
void ui_text(unsigned col, unsigned row, const char *s, uint8_t ink, uint8_t paper);
/* The characters of grid row `row` (NUL-terminated, UI_COLS + 1 bytes); for tests. */
void ui_get_text(unsigned row, char out[UI_COLS + 1u]);
/* 240 palette indices of LCD row `lcd_row` (0-239). */
void ui_compose_row(unsigned lcd_row, uint8_t idx[LCD_W]);
/* Makes every LCD row count as changed at the next ui_dirty_rows, for when the LCD no longer
 * shows the grid (the game was drawn over it). menu.c calls it on every view change. */
void ui_invalidate(void);
/* Bitmap of LCD rows changed since the last call (bit y&7 of byte y>>3); returns how many.
 * A row is changed when its text cell differs from what the previous call reported. */
uint32_t ui_dirty_rows(uint8_t rows[LCD_ROWS / 8u]);

#endif
