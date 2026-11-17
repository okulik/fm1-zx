/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The boot probe (probe.h). Draws with felucca-bsp's SPI calls directly, one row per RAMWR, and
 * waits for every transfer. */

#include "probe.h"

#include "fm1_cc.h"
#include "fm1_time.h"
#include "fm1_sys.h"
#include "fm1_lcd_hw.h"
#include "pixels.h"

#define W 240u
#define BAND_TOP 8u
#define BAND_STEP 14u
#define BAND_H 10u
#define FAULT_TOP 180u
#define BITS_TOP 194u
#define BITS_STEP 12u
#define BITS_H 9u

static uint8_t row[W * 2u] __attribute__((aligned(4)));
static int fmt = -1;          /* -1: the probe has not set the panel up */
static int fw_fmt;            /* the firmware's format: the panel's after lcd.c's lcd_init */
static uint32_t stage_col[16];
static unsigned stages;

static void wait_ms(uint32_t ms)
{
    while (ms--) {
        fm1_delay_us(1000u);
        fm1_wdt_feed();
    }
}

static void cmd(uint8_t c)
{
    fm1_lcd_send_cmd(c);
    (void)fm1_lcd_wait();
    fm1_lcd_deselect();
}

static void data(const uint8_t *p, uint32_t n)
{
    fm1_lcd_send_data(p, n);
    (void)fm1_lcd_wait();
}

/* the arguments one byte at a time through the SPI buffer, as m-bootloader sends them */
static void cmd_args(uint8_t c, unsigned n, uint32_t v)
{
    unsigned i;
    cmd(c);
    FM1_LCD_PC_OUT |= FM1_LCD_DC;
    FM1_LCD_PC_OUT &= ~FM1_LCD_CS;
    for (i = 0; i < n; i++) {
        FM1_LCD_SPI_CON |= 0x4000u;
        FM1_LCD_SPI_BUF = (uint8_t)(v >> (8u * (n - 1u - i)));
        (void)fm1_lcd_wait();
    }
    fm1_lcd_deselect();
}

static void put(unsigned x, uint32_t c)
{
    if (fmt == PIX_RGB565) {
        uint32_t r = c >> 8 & 15u, g = c >> 4 & 15u, b = c & 15u;
        uint32_t p = (r << 1 | r >> 3) << 11 | (g << 2 | g >> 2) << 5 | (b << 1 | b >> 3);
        row[2u * x] = (uint8_t)(p >> 8);
        row[2u * x + 1u] = (uint8_t)p;
    } else {
        uint8_t *q = &row[3u * (x >> 1)];
        if (!(x & 1u)) {
            q[0] = (uint8_t)(c >> 4);
            q[1] = (uint8_t)((q[1] & 0x0Fu) | (c & 15u) << 4);
        } else {
            q[1] = (uint8_t)((q[1] & 0xF0u) | (c >> 8 & 15u));
            q[2] = (uint8_t)c;
        }
    }
}

static void send_row(unsigned y)
{
    cmd_args(0x2A, 4, W - 1u);                         /* CASET 0 .. 239 */
    cmd_args(0x2B, 4, (uint32_t)y << 16 | y);          /* RASET y .. y */
    cmd(0x2C);                                         /* RAMWR */
    data(row, fmt == PIX_RGB565 ? W * 2u : W * 3u / 2u);
    fm1_lcd_deselect();
}

/* As send_row, the pixel bytes one at a time through the SPI buffer, without DMA (as the
 * commands go, and as m-bootloader draws). */
static void send_row_pio(unsigned y)
{
    unsigned i, n = fmt == PIX_RGB565 ? W * 2u : W * 3u / 2u;
    cmd_args(0x2A, 4, W - 1u);
    cmd_args(0x2B, 4, (uint32_t)y << 16 | y);
    cmd(0x2C);
    FM1_LCD_PC_OUT |= FM1_LCD_DC;
    FM1_LCD_PC_OUT &= ~FM1_LCD_CS;
    for (i = 0; i < n; i++) {
        FM1_LCD_SPI_CON |= 0x4000u;
        FM1_LCD_SPI_BUF = row[i];
        (void)fm1_lcd_wait();
    }
    fm1_lcd_deselect();
}

static void fill(unsigned y0, unsigned h, uint32_t c)
{
    unsigned x, y;
    for (x = 0; x < W; x++)
        put(x, c);
    for (y = y0; y < y0 + h; y++)
        send_row(y);
}

/* The panel's own start-up, then DISPON: as lcd.c's lcd_init, which the probe runs before. */
static void panel_up(int f)
{
    fmt = f;
    fm1_lcd_hw_init();
    wait_ms(80);
    cmd(0x01);                                         /* SWRESET */
    wait_ms(150);
    cmd(0x11);                                         /* SLPOUT */
    wait_ms(120);
    cmd_args(0x3A, 1, f == PIX_RGB565 ? 0x55u : 0x03u); /* COLMOD */
    cmd_args(0x36, 1, 0);                              /* MADCTL */
    cmd(0x21);                                         /* INVON */
    cmd(0x13);                                         /* NORON */
    fm1_lcd_deselect();
    fill(0, 240, 0x000u);
}

/* The whole panel in one colour, set up as SLOOP and m-bootloader do it (RGB565) and drawn
 * without DMA: early in the start-up, before the RAM is set up. */
void probe_early(uint32_t c)
{
    unsigned x, y;
    fmt = PIX_RGB565;
    fm1_lcd_hw_init();
    wait_ms(80);
    cmd(0x01);
    wait_ms(150);
    cmd(0x11);
    wait_ms(120);
    cmd_args(0x3A, 1, 0x55u);
    cmd_args(0x36, 1, 0);
    cmd(0x21);
    cmd(0x13);
    cmd(0x29);
    fm1_lcd_deselect();
    for (x = 0; x < W; x++)
        put(x, c);
    for (y = 0; y < 240u; y++)
        send_row_pio(y);
}

/* SLOOP's first step (and M-VAVE V15's): VDC14 to 7 (1.60 V), SYSVDD to 15 (1.38 V). */
void probe_power(void)
{
    fm1_vdc14_raise(7);
    fm1_sysvdd_raise(15);
}

static void spin(void)
{
    volatile uint32_t n;
    for (n = 0; n < 15000000u; n++)
        ;
}

/* n blinks, then the backlight on for a while: by a CPU loop (timer 0; nothing else needed) or
 * by TIMER4 (timer 1: if TIMER4 does not count, the backlight goes dark and stays dark). */
void probe_mark(unsigned n, int timer)
{
    unsigned i;
    FM1_LCD_PA_DIR &= ~FM1_LCD_BL;
    for (i = 0; i < n; i++) {
        fm1_lcd_backlight(0);
        if (timer)
            wait_ms(200);
        else
            spin();
        fm1_lcd_backlight(1);
        if (timer)
            wait_ms(200);
        else
            spin();
    }
    for (i = 0; i < 4u; i++) {
        if (timer)
            wait_ms(200);
        else
            spin();
    }
}

/* The panel as SLOOP and m-bootloader set it up (RGB565), DISPON, then two bands: a white one
 * without DMA (rows 8-17), a cyan one with DMA (rows 22-31). */
void probe_start(int f)
{
    unsigned x, y;
    fw_fmt = f;
    panel_up(PIX_RGB565);
    cmd(0x29);                                         /* DISPON */
    fm1_lcd_deselect();
    for (x = 0; x < W; x++)
        put(x, 0xFFFu);
    for (y = BAND_TOP; y < BAND_TOP + BAND_H; y++)
        send_row_pio(y);
    fill(BAND_TOP + BAND_STEP, BAND_H, 0x0FFu);
    stages = 0;
}

/* After lcd.c's lcd_init (SWRESET, the panel's memory undefined, DISPON not yet sent) every band
 * so far is drawn again. */
void probe_stage(unsigned k, uint32_t c)
{
    unsigned i;
    if (fmt < 0 || k < 1u || k >= 16u)
        return;
    fmt = fw_fmt;                                      /* lcd_init has set the panel's format */
    stage_col[0] = 0xFFFu;
    stage_col[k] = c;
    if (k + 1u > stages)
        stages = k + 1u;
    cmd(0x29);                                         /* DISPON */
    fm1_lcd_deselect();
    for (i = 0; i < stages; i++)
        fill(BAND_TOP + i * BAND_STEP, BAND_H, stage_col[i]);
}

void probe_fault(const uint32_t *words, unsigned n)
{
    unsigned i, b, x, y;
    (void)fm1_lcd_wait();                              /* a band in flight, if any */
    fm1_lcd_deselect();
    if (fmt < 0)
        panel_up(PIX_RGB565);
    cmd(0x29);
    fm1_lcd_deselect();
    fill(FAULT_TOP, BAND_H, 0xF00u);
    for (i = 0; i < n && BITS_TOP + i * BITS_STEP + BITS_H <= 240u; i++) {
        for (x = 0; x < W; x++)
            put(x, 0x000u);
        for (b = 0; b < 32u; b++)
            for (x = 0; x < 6u; x++)
                put(b * 7u + x + 8u, (words[i] >> (31u - b) & 1u) ? 0xFFFu : 0x004u);
        for (y = 0; y < BITS_H; y++)
            send_row(BITS_TOP + i * BITS_STEP + y);
    }
    for (;;) {                                         /* fast blinks: a fault even with a dark panel */
        fm1_lcd_backlight(0);
        wait_ms(100);
        fm1_lcd_backlight(1);
        wait_ms(100);
    }
}
