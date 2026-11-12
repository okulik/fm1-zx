/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* ST7789 driver over felucca-bsp's fm1_lcd_hw.h. The command sequence is the panel's (ST7789
 * datasheet): SWRESET, SLPOUT, COLMOD, MADCTL, INVON, NORON, and per band CASET, RASET, RAMWR
 * followed by the pixel bytes (high byte first). Every SPI transfer is waited for before the
 * next one starts (fm1_lcd_wait, which counts timeouts), except the band's pixel DMA, which
 * runs while lcd_pump returns to the main loop.
 *
 * The pump runs from RAMTEXT: XIP code is out of its branch reach, so it reaches the
 * scheduler, the application's composer and the waits through the pointers in `far`, and XIP
 * reaches it through main.c's table (ramcalls.h). */

#include "lcd.h"

#include "fm1_cc.h"
#include "fm1_time.h"
#include "fm1_sys.h"
#include "fm1_lcd_hw.h"
#include "pixels.h"
#include "ramcalls.h"

#ifndef FMZX_LCD_DIVIDER
#define FMZX_LCD_DIVIDER 4
#endif

#define RAMTEXT __attribute__((section(".ram_text.lcd"), noinline))

enum {
    ST_SWRESET = 0x01, ST_SLPOUT = 0x11, ST_NORON = 0x13, ST_INVON = 0x21, ST_DISPON = 0x29,
    ST_CASET = 0x2A, ST_RASET = 0x2B, ST_RAMWR = 0x2C, ST_MADCTL = 0x36, ST_COLMOD = 0x3A,
};

/* The SPI DMA reads SRAM only: the band buffers in POOL, the command arguments in RAM. */
static uint8_t band[2][LCD_BAND_ROWS * LCD_MAX_ROW_BYTES] __attribute__((section(".pool.lcd_band"), aligned(4)));
static uint8_t caset[4] __attribute__((aligned(4))) = {0, 0, 0, LCD_W - 1u};
static uint8_t args[4] __attribute__((aligned(4)));

static lcd_sched sched;

static struct {
    int fmt;
    unsigned row_bytes, divider;
    uint8_t xfer;      /* an SPI transfer was started and not yet waited for */
    uint8_t compose;   /* band[compose] is the one being composed / ready */
    uint8_t ready;     /* band[compose] holds a band to send: rows ready_row0 .. + ready_rows */
    uint8_t sending;   /* band[compose ^ 1] is going out: rows send_row0 .. + send_rows */
    uint8_t idle;      /* the scheduler had nothing more this frame */
    uint8_t on;        /* DISPON sent */
    unsigned ready_row0, ready_rows, send_row0, send_rows;
    unsigned shown;                  /* rows sent at least once */
    uint8_t shown_bits[LCD_ROWS / 8u];
} st;

/* XIP functions called from the RAMTEXT pump. External, so the compiler cannot turn the
 * indirect calls back into direct ones. */
struct lcd_far {
    int (*next_band)(lcd_sched *, unsigned *, unsigned *);
    void (*compose)(unsigned, unsigned);
    void (*wait)(void);
};
extern struct lcd_far fmzx_lcd_far;
struct lcd_far fmzx_lcd_far;
#define far fmzx_lcd_far

static void lcd_wait(void)
{
    if (st.xfer) {
        (void)fm1_lcd_wait();
        st.xfer = 0;
    }
}

static void wait_ms(uint32_t ms)
{
    while (ms--) {
        fm1_delay_us(1000u);
        fm1_wdt_feed();
    }
}

static void cmd(uint8_t c)
{
    lcd_wait();
    fm1_lcd_send_cmd(c);
    st.xfer = 1;
}

static void cmd_arg(uint8_t c, uint8_t a)
{
    cmd(c);
    lcd_wait();
    args[0] = a;
    fm1_lcd_send_data(args, 1);
    st.xfer = 1;
    lcd_wait();
}

static void RAMTEXT shown(unsigned row0, unsigned rows)
{
    unsigned r;
    for (r = row0; r < row0 + rows; r++) {
        uint8_t m = (uint8_t)(1u << (r & 7u));
        if (!(st.shown_bits[r >> 3] & m)) {
            st.shown_bits[r >> 3] |= m;
            st.shown++;
        }
    }
}

static void RAMTEXT compose_next(void)
{
    unsigned row0, rows;
    if (st.ready || st.idle)
        return;
    if (!far.next_band(&sched, &row0, &rows)) {
        st.idle = 1;
        return;
    }
    far.compose(row0, rows);
    st.ready_row0 = row0;
    st.ready_rows = rows;
    st.ready = 1;
}

static void RAMTEXT send_ready(void)
{
    unsigned r1 = st.ready_row0 + st.ready_rows - 1u;
    far.wait();
    fm1_lcd_send_cmd(ST_CASET);
    st.xfer = 1;
    far.wait();
    fm1_lcd_send_data(caset, 4);
    st.xfer = 1;
    far.wait();
    fm1_lcd_send_cmd(ST_RASET);
    st.xfer = 1;
    far.wait();
    args[0] = (uint8_t)(st.ready_row0 >> 8);
    args[1] = (uint8_t)st.ready_row0;
    args[2] = (uint8_t)(r1 >> 8);
    args[3] = (uint8_t)r1;
    fm1_lcd_send_data(args, 4);
    st.xfer = 1;
    far.wait();
    fm1_lcd_send_cmd(ST_RAMWR);
    st.xfer = 1;
    far.wait();
    fm1_lcd_send_data(band[st.compose], st.ready_rows * st.row_bytes);
    st.xfer = 1;
    st.send_row0 = st.ready_row0;
    st.send_rows = st.ready_rows;
    st.sending = 1;
    st.ready = 0;
    st.compose ^= 1u;
}

void RAMTEXT lcd_pump_ram(void)
{
    if (st.sending) {
        if (!(FM1_LCD_SPI_CON & 0x8000u)) {
            compose_next();
            return;
        }
        far.wait();
        fm1_lcd_deselect();
        st.sending = 0;
        shown(st.send_row0, st.send_rows);
        if (!st.on && st.shown == LCD_ROWS) {
            fm1_lcd_send_cmd(ST_DISPON);
            st.xfer = 1;
            far.wait();
            fm1_lcd_deselect();
            st.on = 1;
        }
    }
    compose_next();
    if (st.ready) {
        send_ready();
        compose_next();
    }
}

void lcd_pump(void)
{
    fmzx_ram.lcd_pump();
}

int lcd_busy(void)
{
    return st.sending && !(FM1_LCD_SPI_CON & 0x8000u);
}

void lcd_init(int fmt)
{
    unsigned i;
    far.next_band = lcd_sched_next_band;
    far.compose = lcd_compose_band;
    far.wait = lcd_wait;
    for (i = 0; i < sizeof st.shown_bits; i++)
        st.shown_bits[i] = 0;
    st.shown = 0;
    st.on = st.sending = st.ready = st.idle = st.xfer = st.compose = 0;
    st.fmt = fmt;
    st.row_bytes = pix_row_bytes(fmt);
    st.divider = FMZX_LCD_DIVIDER;
    lcd_sched_init(&sched, st.row_bytes);

    fm1_lcd_hw_init();
    wait_ms(80);
    cmd(ST_SWRESET);
    wait_ms(150);
    cmd(ST_SLPOUT);
    wait_ms(120);
    cmd_arg(ST_COLMOD, fmt == PIX_RGB565 ? 0x55u : 0x03u);
    cmd_arg(ST_MADCTL, 0x00u);
    cmd(ST_INVON);
    cmd(ST_NORON);
    lcd_wait();
    fm1_lcd_deselect();
    fm1_lcd_baud(st.divider);
}

void lcd_set_divider(unsigned d)
{
    uint32_t n;
    /* the band in flight finishes at the old clock (lcd_pump does the rest of its bookkeeping);
     * bounded like fm1_lcd_wait */
    for (n = 0; n < 4000000u && lcd_busy(); n++)
        ;
    if (n == 4000000u)
        fm1_lcd_timeouts++;
    st.divider = d;
    fm1_lcd_baud(d);
}

unsigned lcd_divider(void)
{
    return st.divider;
}

int lcd_format(void)
{
    return st.fmt;
}

unsigned lcd_row_bytes(void)
{
    return st.row_bytes;
}

uint8_t *lcd_band_out(void)
{
    return band[st.compose];
}

void lcd_mark_row(unsigned row, uint32_t frame)
{
    lcd_sched_mark(&sched, row, frame);
    st.idle = 0;
}

void lcd_mark_all(uint32_t frame)
{
    lcd_sched_mark_all(&sched, frame);
    st.idle = 0;
}

void lcd_begin_frame(uint32_t frame, uint32_t budget_bytes)
{
    lcd_sched_begin_frame(&sched, frame, budget_bytes);
    st.idle = 0;
}

unsigned lcd_pending(void)
{
    return lcd_sched_pending(&sched);
}

unsigned lcd_unsent(void)
{
    return lcd_sched_pending(&sched) + (st.ready ? st.ready_rows : 0u) + (st.sending ? st.send_rows : 0u);
}

const lcd_stats *lcd_get_stats(void)
{
    return &sched.stats;
}
