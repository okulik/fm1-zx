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
static uint8_t art_on, shown_art;

#define PIDX(i) {PIX_R8(i), PIX_G8(i), PIX_B8(i)}
#define PCOL(r, g, b) {(uint8_t)((r) * 17u), (uint8_t)((g) * 17u), (uint8_t)((b) * 17u)}

const uint8_t ui_palette[UI_PAL_N][3] = {
    PIDX(0), PIDX(1), PIDX(2), PIDX(3), PIDX(4), PIDX(5), PIDX(6), PIDX(7),
    PIDX(8), PIDX(9), PIDX(10), PIDX(11), PIDX(12), PIDX(13), PIDX(14), PIDX(15),
    PCOL(0xc, 0x9, 0xd), /* 16 lilac */
    PCOL(0x8, 0x7, 0xd), /* 17 key bed */
    PCOL(0x6, 0x6, 0xb), /* 18 keys */
    PCOL(0x2, 0x2, 0x1), /* 19 key edges */
    PCOL(0x3, 0x3, 0x6), /* 20 key slits */
    PCOL(0x9, 0x8, 0xd), /* 21 button panel */
    PCOL(0x8, 0x7, 0xc), /* 22 buttons */
    PCOL(0x7, 0x6, 0xb), /* 23 OCT panel */
    PCOL(0xd, 0xe, 0xe), /* 24 silver */
    PCOL(0xe, 0x2, 0x2), /* 25 stripe red */
    PCOL(0xf, 0xa, 0x1), /* 26 stripe yellow */
    PCOL(0x5, 0xb, 0x4), /* 27 stripe green */
    PCOL(0x1, 0x9, 0xd), /* 28 stripe cyan */
};

void ui_pack_row(int fmt, const uint8_t idx[LCD_W], uint8_t *out)
{
    unsigned x;
    if (fmt == PIX_RGB565) {
        for (x = 0; x < LCD_W; x++) {
            const uint8_t *c = ui_palette[idx[x] < UI_PAL_N ? idx[x] : 0u];
            unsigned v = ((c[0] >> 3) << 11) | ((c[1] >> 2) << 5) | (c[2] >> 3);
            *out++ = (uint8_t)(v >> 8);
            *out++ = (uint8_t)v;
        }
        return;
    }
    for (x = 0; x < LCD_W; x += 2) {
        const uint8_t *a = ui_palette[idx[x] < UI_PAL_N ? idx[x] : 0u];
        const uint8_t *b = ui_palette[idx[x + 1u] < UI_PAL_N ? idx[x + 1u] : 0u];
        *out++ = (uint8_t)((a[0] & 0xF0u) | (a[1] >> 4));
        *out++ = (uint8_t)((a[2] & 0xF0u) | (b[0] >> 4));
        *out++ = (uint8_t)((b[1] & 0xF0u) | (b[2] >> 4));
    }
}

extern const uint8_t ui_art_top_pal[];
extern const uint16_t ui_art_top_off[UI_ART_TOP_ROWS];
extern const uint8_t ui_art_top[];

void ui_art_top_row(unsigned y, uint8_t idx[LCD_W])
{
    const uint8_t *p;
    unsigned x = 0;
    if (y >= UI_ART_TOP_ROWS) {
        memset(idx, 0, LCD_W);
        return;
    }
    p = ui_art_top + ui_art_top_off[y];
    while (x < LCD_W) {
        uint8_t v = ui_art_top_pal[*p >> 4];
        unsigned n = (*p++ & 15u) + 1u;
        while (n-- != 0u && x < LCD_W) {
            idx[x++] = v;
        }
    }
}

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

void ui_set_art(int on)
{
    art_on = on != 0;
}

void ui_clear(uint8_t ink, uint8_t paper)
{
    art_on = 0;
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
    if (!art_on) {
        return;
    }
    if (lcd_row < UI_ART_TOP_ROWS) {
        ui_art_top_row(lcd_row, idx);
    } else if (lcd_row >= 130u) {
        /* corner stripes: u = 100 * (x - 193) - 42 * (239 - y), k = floor(u / 500), drawn for 0 <= k < 4;
         * u grows with x, so start at the first x with u >= 0 and step to k = 4 */
        int base = -19300 - 42 * (int)(239u - lcd_row); /* u at x = 0 */
        for (x = 0; x < LCD_W; x++) {
            int u = base + 100 * (int)x;
            if (u >= 0) {
                int k = u / 500;
                if (k >= 4) {
                    break;
                }
                idx[x] = (uint8_t)(25 + k);
            }
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
    if (art_on != shown_art) { /* the art appeared or went: the whole LCD changes */
        shown_art = art_on;
        ui_invalidate();
    }
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
