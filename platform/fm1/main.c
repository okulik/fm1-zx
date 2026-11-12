/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Start-up after crt0.S, and the main loop. Hardware through the vendored felucca-bsp headers
 * (platform/fm1/VENDORED.md); the firmware logic is app/ (hardware-free, host-tested). */

#include <stdint.h>
#include <string.h>

#include "fm1_cc.h"
#include "fm1_time.h"
#include "fm1_irq.h" /* the one unit with FM1_IRQ_DEFS 1: the crash record and fm1_fault_c */
#include "fm1_sys.h"
#include "fm1_cpu1_stop.h"
#include "fm1_gpio.h"
#include "fm1_input.h"

#include "lcd.h"
#include "lcdsched.h"
#include "menu.h"
#include "pack.h"
#include "pixels.h"
#include "ui.h"
#include "zx.h"

#ifndef FMZX_LCD_DIVIDER
#define FMZX_LCD_DIVIDER 4
#endif

#define POOL(tag) __attribute__((section(".pool." tag), aligned(4)))

#define BOOTOPT_MAGIC 0x5A584F50u           /* "ZXOP" */
#define FRAME_TICKS (20000u * FM1_TICKS_PER_US) /* 20 ms of TIMER4 */
#define GESTURE_TICKS (1000000u * FM1_TICKS_PER_US)
#define WDT_8S 0x0Du

/* Boot options from emu-gate (or anything that writes them before a reset): word 0 BOOTOPT_MAGIC,
 * word 1 bit 0 RGB565, bits 8-11 the LCD divider. First in NOINIT, never cleared by start-up;
 * word 0 is cleared once read. */
uint32_t fmzx_bootopt[2] __attribute__((section(".noinit.bootopt"), aligned(4)));

/* Read by emu-gate (FwStats): keep the layout in step with host/crates/emu-gate/src/fw.rs. */
struct fmzx_stats {
    uint32_t frames;    /* Spectrum frames run in the current game */
    uint32_t underruns; /* audio FIFO ran dry while a game ran */
    uint32_t overflows; /* audio samples dropped, FIFO full */
    uint32_t view;      /* menu_view */
    uint32_t game;      /* the running game's index */
    uint32_t divider;   /* LCD SPI divider */
    uint32_t budget;    /* LCD bytes per 20 ms frame */
    uint32_t pending;   /* LCD rows waiting to be sent */
    lcd_stats lcd;
};
struct fmzx_stats fmzx_stats;

extern const uint8_t zx_rom_data[];
extern const uint8_t _app_image_end[], _app_slot_end[];
extern uint32_t _bss_start[], _bss_end[], _pool_start[], _pool_end[];
extern uint32_t _data_start[], _data_end[], _data_load[];
extern uint32_t _ram_text_start[], _ram_text_end[], _ram_text_load[];
extern void (*__init_array_start[])(void), (*__init_array_end[])(void);

void fmzx_cstart(void);
void fmzx_enter_uboot(void);
void fmzx_main(void);

static uint8_t rom_copy[ZX_ROM_SIZE] POOL("rom");
static uint8_t row_idx[LCD_W];
static menu mn;
static const uint8_t *pack_base;

static void fm1_fault(const fm1_crash_t *c)
{
    (void)c; /* the record stays in .noinit for the next boot */
    fm1_reboot();
}

/* A separate function so emu-gate can see the UBOOT entry (felucca-bsp's is static). */
__attribute__((noinline)) void fmzx_enter_uboot(void)
{
    fm1_wdt_stop();
    fm1_enter_uboot();
}

/* ------------------------------------------------------------------ start-up --- */

/* OCT- now, read straight from the matrix: its column selected through the 595 chain, its row
 * sampled (as fm1_input_scan does for one column). Uses no RAM variables. */
static int oct_down_raw(void)
{
    uint32_t r, c, rows;
    for (r = 0; r < 6u; r++)
        for (c = 0; c < FM1_NCOL; c++)
            if (FM1_KEYMAP[r][c] == FM1_BTN_OCT_DOWN)
                goto found;
    return 0;
found:
    fm1__sr_word(0xFFFFu ^ (1u << c) ^ (c < 2u ? 1u << (11u + c) : 0u));
    fm1_delay_us(FM1_SETTLE_US);
    rows = fm1__rows();
    fm1__sr_word(0xFFFFu);
    return (int)((rows >> r) & 1u);
}

/* OCT- held through the first second: the user wants the USB update mode (UBOOT). */
static int uboot_gesture(void)
{
    uint32_t t0;
    fm1_input_init(); /* pins only here; its state in .bss is zeroed right after */
    FM1_PR(FM1_PA, FM1_OUT) &= ~(1u << 2); /* PA2 low: the 595s' /OE (and the backlight) */
    FM1_PR(FM1_PA, FM1_DIR) &= ~(1u << 2);
    t0 = fm1_ticks();
    while ((uint32_t)(fm1_ticks() - t0) < GESTURE_TICKS) {
        if (!oct_down_raw())
            return 0;
        fm1_delay_ms(10);
        fm1_wdt_feed();
    }
    return 1;
}

void fmzx_cstart(void)
{
    uint32_t *p;
    const uint32_t *s;
    void (**f)(void);

    fm1_cpu1_stop();
    fm1_time_init();
    fm1_wdt_arm(WDT_8S);
    if (uboot_gesture())
        fmzx_enter_uboot();
    fm1_irq_init();
    for (p = _bss_start; p < _bss_end; p++)
        *p = 0;
    for (p = _pool_start; p < _pool_end; p++)
        *p = 0;
    for (p = _data_start, s = _data_load; p < _data_end; p++, s++)
        *p = *s;
    for (p = _ram_text_start, s = _ram_text_load; p < _ram_text_end; p++, s++)
        *p = *s;
    __asm__ volatile("csync" ::: "memory");
    for (f = __init_array_start; f < __init_array_end; f++)
        (*f)();
    fm1_mailbox_clear();
    fmzx_main();
    for (;;)
        ;
}

/* ----------------------------------------------------------------- main loop --- */

void lcd_compose_band(unsigned row0, unsigned rows)
{
    uint8_t *out = lcd_band_out();
    unsigned i, rb = lcd_row_bytes();
    int fmt = lcd_format();
    for (i = 0; i < rows; i++) {
        unsigned r = row0 + i;
        if (menu_row_is_ui(&mn, r))
            ui_compose_row(r, row_idx);
        else
            memset(row_idx, 0, sizeof row_idx); /* the Spectrum screen comes with the games */
        pix_pack(fmt, row_idx, out + i * rb);
    }
}

/* The game pack after the image, at the first 4 KiB boundary, up to the app slot's end. */
static uint32_t open_pack(void)
{
    uint32_t base = ((uint32_t)(uintptr_t)_app_image_end + 4095u) & ~4095u;
    uint32_t end = (uint32_t)(uintptr_t)_app_slot_end, count = 0;
    if (base < end && pack_open((const uint8_t *)(uintptr_t)base, end - base, &count) == ZX_OK) {
        pack_base = (const uint8_t *)(uintptr_t)base;
        return count;
    }
    pack_base = 0;
    return 0;
}

static void ui_frame(uint32_t frame)
{
    uint8_t dirty[LCD_ROWS / 8u];
    unsigned r;
    menu_draw(&mn, pack_base);
    if (ui_dirty_rows(dirty))
        for (r = 0; r < LCD_ROWS; r++)
            if (dirty[r >> 3] & (1u << (r & 7u)))
                lcd_mark_row(r, frame);
}

void fmzx_main(void)
{
    int fmt = PIX_RGB444;
    unsigned divider = FMZX_LCD_DIVIDER;
    uint32_t frame = 0, next, budget;

    if (fmzx_bootopt[0] == BOOTOPT_MAGIC) {
        fmt = (fmzx_bootopt[1] & 1u) ? PIX_RGB565 : PIX_RGB444;
        divider = (fmzx_bootopt[1] >> 8) & 15u;
    }
    fmzx_bootopt[0] = 0;

    lcd_init(fmt);
    lcd_set_divider(divider);
    budget = lcd_budget_bytes(divider);

    memcpy(rom_copy, zx_rom_data, ZX_ROM_SIZE);
    ui_init(rom_copy);
    menu_init(&mn, open_pack());
    menu_set_divider(&mn, divider);

    next = fm1_ticks();
    for (;;) {
        if ((int32_t)(fm1_ticks() - next) >= 0) {
            next += FRAME_TICKS;
            if ((int32_t)(fm1_ticks() - next) >= 0) /* fell behind: no catching up */
                next = fm1_ticks() + FRAME_TICKS;
            fm1_wdt_feed();
            /* what the last frame left: rows still pending, rows sent */
            fmzx_stats.pending = lcd_pending();
            fmzx_stats.lcd = *lcd_get_stats();
            frame++;
            ui_frame(frame);
            lcd_begin_frame(frame, budget);
            fmzx_stats.view = (uint32_t)menu_view(&mn);
            fmzx_stats.divider = divider;
            fmzx_stats.budget = budget;
        }
        lcd_pump();
    }
}
