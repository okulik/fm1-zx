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
#include "fm1_timer.h"

#include "audio.h"
#include "input.h"
#include "lcd.h"
#include "lcdsched.h"
#include "menu.h"
#include "pack.h"
#include "pixels.h"
#include "ramcalls.h"
#include "ui.h"
#include "zx.h"

#ifndef FMZX_LCD_DIVIDER
#define FMZX_LCD_DIVIDER 4
#endif

#define POOL(tag) __attribute__((section(".pool." tag), aligned(4)))
#define RAMTEXT __attribute__((section(".ram_text.game"), noinline))

#define BOOTOPT_MAGIC 0x5A584F50u           /* "ZXOP" */
#define FRAME_TICKS (20000u * FM1_TICKS_PER_US) /* 20 ms of TIMER4 */
#define GESTURE_TICKS (1000000u * FM1_TICKS_PER_US)
#define WDT_8S 0x0Du
#define MASTER_MS 200u
#define TIMER5_PRIO 4u
#define AUDIO_LOW 1024u     /* run a Spectrum frame while fewer samples than this are queued */
#define FRAME_SAMPLES 1024u /* room for one frame's samples: 750000/17 Hz * 69888 / 3.5 MHz < 882 */
#define GAIN_FRAMES 10u     /* MASTER -> gain every 10 frames */
#define NO_COLOUR 0xFFu

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

/* The running game: the machine, its extracted file (a .tap is read from it while it runs), the
 * beeper renderer and one frame's samples. */
zx_machine fmzx_machine POOL("machine");
static uint8_t game_file[PACK_MAX_RAW] POOL("game");
static int16_t frame_samples[FRAME_SAMPLES] POOL("samples");
static zx_beeper beeper;
static km_state kms;
static struct {
    uint8_t loaded;      /* fmzx_machine holds a game */
    uint8_t crop_x;
    uint8_t keys[8];     /* applied before each frame (zx_set_keys) */
    uint8_t kempston;
    uint32_t index;      /* pack entry */
    uint32_t frames;
    uint8_t dirty[LCD_ROWS / 8u]; /* LCD rows the last frame changed */
} game;
/* The border colour each LCD row last went out with (NO_COLOUR: not a border picture). */
static uint8_t border_sent[LCD_ROWS];

static unsigned divider = FMZX_LCD_DIVIDER;
static uint32_t budget;    /* LCD bytes per frame */
static uint32_t lcd_frame; /* the LCD scheduler's frame count */
static int audio_on;       /* the audio DMA runs (it needs the stock audio PLL) */

/* The MASTER pot, 0-1023, refreshed every MASTER_MS (read by emu-gate and, later, the mixer). */
volatile uint32_t fmzx_master;

static void fm1_fault(const fm1_crash_t *c)
{
    (void)c; /* the record stays in .noinit for the next boot */
    fm1_reboot();
}

/* ------------------------------------------------------------------ interrupts --- */

void fm1_timer5_irq(void);

void fm1_timer5_irq(void)
{
    fm1_timer5_ack();
    input_tick_isr();
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

/* ------------------------------------------------------------------ the game --- */

/* One Spectrum frame, in RAMTEXT with the core: input applied, the frame run, its beeper edges
 * rendered and queued, and the LCD rows it changed noted in game.dirty (paper lines the core
 * reports, border rows whose colour differs from what the LCD last got). */
static void RAMTEXT game_step(void)
{
    zx_machine *m = &fmzx_machine;
    const zx_edge *e;
    uint32_t n, y, r;
    zx_set_keys(m, game.keys);
    zx_set_kempston(m, game.kempston);
    zx_run_frame(m);
    zx_beeper_set_level(&beeper, zx_edges_start_level(m));
    e = zx_edges(m, &n);
    n = zx_beeper_render(&beeper, e, n, frame_samples);
    audio_push(frame_samples, n);
    for (r = 0; r < LCD_ROWS / 8u; r++)
        game.dirty[r] = 0;
    for (y = 0; y < ZX_SCREEN_LINES; y++)
        if (zx_line_dirty(m, y)) {
            r = LCD_TOP_BORDER + y;
            game.dirty[r >> 3] |= (uint8_t)(1u << (r & 7u));
        }
    for (r = 0; r < LCD_ROWS; r++) {
        if (r == LCD_TOP_BORDER)
            r += ZX_SCREEN_LINES; /* the paper rows */
        if (zx_border_line(m, ZX_PAPER_FIRST_LINE - LCD_TOP_BORDER + r) != border_sent[r])
            game.dirty[r >> 3] |= (uint8_t)(1u << (r & 7u));
    }
}

/* Leaving the game running: every Spectrum key and Kempston bit up. */
static void RAMTEXT game_release(void)
{
    unsigned i;
    for (i = 0; i < 8u; i++)
        game.keys[i] = 0;
    game.kempston = 0;
    zx_set_keys(&fmzx_machine, game.keys);
    zx_set_kempston(&fmzx_machine, 0);
}

/* The RAMTEXT code XIP calls (ramcalls.h). */
struct ram_calls fmzx_ram = {
    zx_init, zx_load_sna, zx_load_z80, zx_tape_insert, zx_tape_autostart, zx_beeper_init,
    pix_compose_zx, game_step, game_release, lcd_pump_ram,
};

static uint16_t gain_q10(uint32_t master)
{
    return (uint16_t)(master >= 1016u ? 1024u : master * master >> 10);
}

/* Loads pack entry `index` into fmzx_machine; ZX_OK, or the error (the game does not run). */
static int game_start(uint32_t index)
{
    zx_machine *m = &fmzx_machine;
    pack_game g;
    uint32_t len, i;
    int e;
    audio_mute(1);
    audio_flush();
    game.loaded = 0;
    game.index = index;
    game.frames = 0;
    fmzx_stats.frames = 0;
    fmzx_stats.game = index;
    if (!pack_base || pack_game_at(pack_base, index, &g) != ZX_OK ||
        pack_extract(&g, game_file, sizeof game_file, &len) != ZX_OK)
        return ZX_ERR_FORMAT;
    fmzx_ram.init(m, rom_copy);
    if (g.kind == PACK_SNA) {
        e = fmzx_ram.load_sna(m, game_file, len);
    } else if (g.kind == PACK_Z80) {
        e = fmzx_ram.load_z80(m, game_file, len);
    } else {
        e = fmzx_ram.tape_insert(m, game_file, len);
        if (e == ZX_OK)
            fmzx_ram.tape_autostart(m);
    }
    if (e != ZX_OK)
        return e;
    fmzx_ram.beeper_init(&beeper, 750000u, 17u);
    for (i = 0; i < LCD_ROWS; i++)
        border_sent[i] = NO_COLOUR;
    game.crop_x = g.crop_x;
    game.loaded = 1;
    audio_set_gain(gain_q10(fmzx_master));
    lcd_mark_all(lcd_frame);
    return ZX_OK;
}

/* ----------------------------------------------------------------- main loop --- */

void lcd_compose_band(unsigned row0, unsigned rows)
{
    uint8_t *out = lcd_band_out();
    unsigned i, rb = lcd_row_bytes();
    int fmt = lcd_format();
    for (i = 0; i < rows; i++) {
        unsigned r = row0 + i;
        if (menu_row_is_ui(&mn, r)) {
            ui_compose_row(r, row_idx);
            border_sent[r] = NO_COLOUR;
        } else if (!game.loaded) {
            memset(row_idx, 0, sizeof row_idx);
            border_sent[r] = NO_COLOUR;
        } else {
            fmzx_ram.compose_zx(&fmzx_machine, r, game.crop_x, row_idx);
            border_sent[r] = row_idx[0];
        }
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

extern void isr_timer5(void);

static void handle_event(int ev)
{
    uint32_t arg = 0;
    if (ev == EV_NONE)
        return;
    switch (menu_event(&mn, ev, &arg)) {
    case CMD_START:
        if (game_start(arg) == ZX_OK)
            menu_set_view(&mn, VIEW_GAME);
        else
            menu_message(&mn, "Load failed");
        break;
    case CMD_RESET:
        if (game_start(game.index) != ZX_OK)
            menu_message(&mn, "Load failed");
        break;
    case CMD_DIVIDER:
        divider = arg;
        lcd_set_divider(divider);
        budget = lcd_budget_bytes(divider);
        break;
    default:
        break;
    }
}

/* Menu-style controls (picker, HOME menu, key page, keyboard, message): SELECT and ALGORITHM
 * turns and OCT-/OCT+ move, PLAY is OK, HOME is BACK. */
static void menu_controls(const km_input *in, uint32_t pressed)
{
    uint32_t e;
    int32_t n, i;
    for (e = 0; e < 2u; e++) {
        n = in->enc[e];
        for (i = 0; i < (n < 0 ? -n : n); i++)
            handle_event(n > 0 ? EV_DOWN : EV_UP);
    }
    if (pressed & (1u << KM_BTN_PLAY))
        handle_event(EV_OK);
    else if (pressed & (1u << KM_BTN_HOME))
        handle_event(EV_BACK);
    else if (pressed & 1u) /* OCT- */
        handle_event(EV_UP);
    else if (pressed & 2u) /* OCT+ */
        handle_event(EV_DOWN);
}

/* One poll. In the game and while paused, PLAY pauses / resumes and HOME opens the menu (the
 * game's own controls come with the key mapping). */
static void poll_input(void)
{
    km_input in;
    uint32_t pressed, notes;
    int v;
    input_poll(&in, &pressed, &notes);
    v = menu_view(&mn);
    if (v == VIEW_GAME || v == VIEW_PAUSED) {
        if (pressed & (1u << KM_BTN_PLAY))
            handle_event(EV_PAUSE);
        else if (pressed & (1u << KM_BTN_HOME))
            handle_event(EV_HOME);
    } else {
        menu_controls(&in, pressed);
    }
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

static void stats_update(void)
{
    fmzx_stats.lcd = *lcd_get_stats();
    fmzx_stats.view = (uint32_t)menu_view(&mn);
    fmzx_stats.divider = divider;
    fmzx_stats.budget = budget;
    fmzx_stats.underruns = audio_underruns();
    fmzx_stats.overflows = audio_overflows();
}

/* One LCD frame: the UI's changed rows marked, the scheduler's budget renewed. */
static void lcd_frame_end(void)
{
    lcd_frame++;
    ui_frame(lcd_frame);
    lcd_begin_frame(lcd_frame, budget);
    stats_update();
}

/* A Spectrum frame and everything it sends on: samples to the FIFO, changed rows to the LCD. */
static void game_frame(void)
{
    unsigned r;
    /* fmzx_stats as the frame finds it: `frames` run, `pending` LCD rows still to go */
    fmzx_stats.pending = lcd_unsent();
    fmzx_ram.game_step();
    game.frames++;
    fmzx_stats.frames = game.frames;
    for (r = 0; r < LCD_ROWS; r++)
        if (game.dirty[r >> 3] & (1u << (r & 7u)))
            lcd_mark_row(r, lcd_frame + 1u);
    if (game.frames % GAIN_FRAMES == 0)
        audio_set_gain(gain_q10(fmzx_master));
    audio_mute(0);
    lcd_frame_end();
}

/* Every FRAME_TICKS of TIMER4 (no catching up after falling behind). */
static int frame_due(uint32_t *next)
{
    if ((int32_t)(fm1_ticks() - *next) < 0)
        return 0;
    *next += FRAME_TICKS;
    if ((int32_t)(fm1_ticks() - *next) >= 0)
        *next = fm1_ticks() + FRAME_TICKS;
    return 1;
}

void fmzx_main(void)
{
    int fmt = PIX_RGB444, view, last_view = -1;
    uint32_t next, master_at;

    if (fmzx_bootopt[0] == BOOTOPT_MAGIC) {
        fmt = (fmzx_bootopt[1] & 1u) ? PIX_RGB565 : PIX_RGB444;
        divider = (fmzx_bootopt[1] >> 8) & 15u;
    }
    fmzx_bootopt[0] = 0;

    lcd_init(fmt);
    lcd_set_divider(divider);
    budget = lcd_budget_bytes(divider);
    lcd_mark_all(0); /* the first picture and DISPON do not wait for the UI */

    memcpy(rom_copy, zx_rom_data, ZX_ROM_SIZE);
    ui_init(rom_copy);
    menu_init(&mn, open_pack());
    menu_set_divider(&mn, divider);

    input_init();
    fmzx_master = input_master();
    audio_init();
    audio_on = audio_start() == 0;
    fm1_timer5_start(isr_timer5, TIMER5_PRIO);
    fm1_irq_enable_all();

    next = fm1_ticks();
    master_at = next;
    for (;;) {
        poll_input();
        if ((uint32_t)(fm1_ticks() - master_at) >= MASTER_MS * 1000u * FM1_TICKS_PER_US) {
            master_at += MASTER_MS * 1000u * FM1_TICKS_PER_US;
            fmzx_master = input_master();
        }
        view = menu_view(&mn);
        if (view != last_view) { /* every view change repaints the whole panel */
            last_view = view;
            fmzx_stats.view = (uint32_t)view;
            lcd_mark_all(lcd_frame);
        }
        if (menu_runs_game(&mn)) {
            /* State-based like the release below: running means unmuted, so a resumed game's
             * queued samples drain again. Only a game that has not yet queued its first frame
             * stays muted (game_frame unmutes after that push), so a start is no underrun. */
            if (game.frames)
                audio_mute(0);
            /* paced by the audio out; by TIMER4 if it did not start */
            if (audio_on ? audio_fifo_level() < AUDIO_LOW : frame_due(&next)) {
                game_frame();
                if (!audio_on)
                    audio_flush();
                else
                    next = fm1_ticks() + 2u * FRAME_TICKS;
            } else if (audio_on && frame_due(&next)) {
                /* no Spectrum frame for 40 ms (a stalled audio out): the LCD and the UI still
                 * get a frame every 20 ms */
                fmzx_stats.pending = lcd_unsent();
                lcd_frame_end();
            }
        } else {
            /* state-based: whatever the way out of the game was, it is silent and released */
            audio_mute(1);
            if (game.loaded)
                fmzx_ram.game_release();
            km_reset(&kms);
            if (frame_due(&next)) {
                /* what the last frame left: rows still pending, rows sent */
                fmzx_stats.pending = lcd_unsent();
                lcd_frame_end();
            }
        }
        lcd_pump();
        fm1_wdt_feed();
    }
}
