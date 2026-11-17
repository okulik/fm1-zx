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

/* The UI palette: indices 0-15 are the Spectrum's (as PIX_R8/G8/B8), 16-24 the menu art's band
 * colours, 25-28 the corner stripes. RGB888; the panel keeps 4 bits (RGB444) or 5-6-5 of each. */
#define UI_PAL_N 29u
extern const uint8_t ui_palette[UI_PAL_N][3];

/* Packs 240 UI palette indices (0-28) into panel bytes, the layout of pix_pack (PIX_RGB444 or
 * PIX_RGB565). Plain flash code: it calls nothing in RAMTEXT. */
void ui_pack_row(int fmt, const uint8_t idx[LCD_W], uint8_t *out);

/* The menu art's top band: 240 x 64 pixels, as UI palette indices. Row y (0-63). */
#define UI_ART_TOP_ROWS 64u
void ui_art_top_row(unsigned y, uint8_t idx[LCD_W]);

/* Copies the font (ROM 0x3D00, chars 32-127) and clears the grid to white on black; every row
 * counts as changed afterwards. `rom` is the 16 KiB 48K ROM. */
void ui_init(const uint8_t *rom);
/* Turns the menu art on (picker and HOME menu only): the band over LCD rows 0-63 and the corner
 * stripes over rows 130-239, except on cells whose paper is not black (the selection bar is drawn
 * over them). ui_clear turns it off; a change repaints every LCD row. */
void ui_set_art(int on);
/* Fills every cell with a space. Colours are palette indices 0-15 (8-15 bright). */
void ui_clear(uint8_t ink, uint8_t paper);
/* Draws `s` from cell (col, row); clipped at the grid edge. Chars outside 32-127 show as '?'. */
void ui_text(unsigned col, unsigned row, const char *s, uint8_t ink, uint8_t paper);
/* The characters of grid row `row` (NUL-terminated, UI_COLS + 1 bytes); for tests. */
void ui_get_text(unsigned row, char out[UI_COLS + 1u]);
/* 240 UI palette indices (0-28; the art and stripes use 16-28) of LCD row `lcd_row` (0-239).
 * Pack them with ui_pack_row, not pix_pack, which masks with & 15. */
void ui_compose_row(unsigned lcd_row, uint8_t idx[LCD_W]);
/* Makes every LCD row count as changed at the next ui_dirty_rows, for when the LCD no longer
 * shows the grid (the game was drawn over it). menu.c calls it on every view change. */
void ui_invalidate(void);
/* Bitmap of LCD rows changed since the last call (bit y&7 of byte y>>3); returns how many.
 * A row is changed when its text cell differs from what the previous call reported. */
uint32_t ui_dirty_rows(uint8_t rows[LCD_ROWS / 8u]);

#endif
