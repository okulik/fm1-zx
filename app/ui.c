/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "ui.h"
#include <string.h>

/* One grid; a cell is a char and an attribute (ink | paper << 4). `shown_*` is what the last
 * ui_dirty_rows call reported, so a row that is rewritten to the same text is not dirty. */
static uint8_t font[96][8];
static uint8_t cell_ch[UI_ROWS][UI_COLS];
static uint8_t cell_at[UI_ROWS][UI_COLS];
static uint8_t shown_ch[UI_ROWS][UI_COLS];
static uint8_t shown_at[UI_ROWS][UI_COLS];

static uint8_t attr(uint8_t ink, uint8_t paper)
{
    return (uint8_t)((ink & 15u) | ((paper & 15u) << 4));
}

void ui_init(const uint8_t *rom)
{
    memcpy(font, rom + 0x3D00u, sizeof font);
    ui_clear(7, 0);
    memset(shown_ch, 0, sizeof shown_ch); /* 0 is never drawn: every row differs */
}

void ui_clear(uint8_t ink, uint8_t paper)
{
    memset(cell_ch, ' ', sizeof cell_ch);
    memset(cell_at, attr(ink, paper), sizeof cell_at);
}

void ui_text(unsigned col, unsigned row, const char *s, uint8_t ink, uint8_t paper)
{
    if (row >= UI_ROWS) {
        return;
    }
    for (; *s != '\0' && col < UI_COLS; s++, col++) {
        uint8_t c = (uint8_t)*s;
        cell_ch[row][col] = (c >= 32u && c <= 127u) ? c : (uint8_t)'?';
        cell_at[row][col] = attr(ink, paper);
    }
}

void ui_get_text(unsigned row, char out[UI_COLS + 1u])
{
    unsigned i;
    for (i = 0; i < UI_COLS; i++) {
        out[i] = row < UI_ROWS ? (char)cell_ch[row][i] : ' ';
    }
    out[UI_COLS] = '\0';
}

void ui_compose_row(unsigned lcd_row, uint8_t idx[LCD_W])
{
    unsigned col, x, r, y;
    if (lcd_row >= LCD_ROWS) {
        memset(idx, 0, LCD_W);
        return;
    }
    r = lcd_row >> 3;
    y = lcd_row & 7u;
    for (col = 0; col < UI_COLS; col++) {
        uint8_t bits = font[cell_ch[r][col] - 32u][y];
        uint8_t a = cell_at[r][col];
        uint8_t ink = a & 15u, paper = (uint8_t)(a >> 4);
        for (x = 0; x < 8; x++) {
            idx[col * 8u + x] = (bits & (0x80u >> x)) ? ink : paper;
        }
    }
}

void ui_invalidate(void)
{
    memset(shown_ch, 0, sizeof shown_ch);
}

uint32_t ui_dirty_rows(uint8_t rows[LCD_ROWS / 8u])
{
    uint32_t n = 0;
    unsigned r;
    for (r = 0; r < UI_ROWS; r++) {
        if (memcmp(cell_ch[r], shown_ch[r], UI_COLS) != 0 || memcmp(cell_at[r], shown_at[r], UI_COLS) != 0) {
            rows[r] = 0xFFu; /* text row r = LCD rows 8r..8r+7 = all bits of byte r */
            n += 8u;
            memcpy(shown_ch[r], cell_ch[r], UI_COLS);
            memcpy(shown_at[r], cell_at[r], UI_COLS);
        } else {
            rows[r] = 0;
        }
    }
    return n;
}
