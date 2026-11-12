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
/* The colour mode without boot options: RGB444 on the panel (default), RGB565 for the emulator
 * preview package (CMake FMZX_LCD_RGB565; fm1-emu's LCD shows RGB565 only). */
#ifndef FMZX_LCD_RGB565
#define FMZX_LCD_RGB565 0
#endif
#ifndef FMZX_PRODUCT
#define FMZX_PRODUCT "FM-1_948"
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
    lcd_stats lcd;      /* frames, row_sends, pending_frames_sum, bytes, bands, max_age */
};
struct fmzx_stats fmzx_stats;

/* The product identity the update package carries in its markers (CMake FMZX_PRODUCT), kept in the
 * image (app.ld) so that `fmzx-pack verify` can match the package against the image. */
const char fmzx_product[] __attribute__((section(".rodata.fmzx_product"), used)) = FMZX_PRODUCT;

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
    uint8_t rows[8];     /* Spectrum key rows, applied before each frame (zx_set_keys) */
    uint8_t kempston;    /* applied before each frame (zx_set_kempston) */
    keymap keys;         /* the game's key map, from its pack entry */
    uint32_t index;      /* pack entry */
    uint32_t frames;
    uint8_t dirty[LCD_ROWS / 8u]; /* LCD rows the last frame changed */
} game;
/* The FM-1 inputs for the next frame's km_apply: the held masks of the last poll and the encoder
 * detents of every poll since the last frame (the ones the keyboard uses left out). */
static km_input held_in;
static uint8_t play_held; /* PLAY, which never reaches km_apply, held at the last poll */
/* The on-screen keyboard's key (CMD_OSK_KEY): its rows, OR-ed into the frame's rows from the
 * frame after PLAY pressed it until the frame after PLAY is released (one frame at least). */
static uint8_t osk_rows[8];
static uint8_t osk_down;

/* What the Spectrum reads now, copied from fmzx_machine.keys and .kempston after zx_set_keys and
 * zx_set_kempston, for emu-gate (zx_machine's layout differs between the host and the device). */
uint8_t fmzx_keys_now[8] __attribute__((aligned(4)));
uint32_t fmzx_kempston_now;

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
    zx_set_keys(m, game.rows);
    zx_set_kempston(m, game.kempston);
    for (r = 0; r < 8u; r++)
        fmzx_keys_now[r] = m->keys[r];
    fmzx_kempston_now = m->kempston;
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
    zx_machine *m = &fmzx_machine;
    unsigned i;
    for (i = 0; i < 8u; i++)
        game.rows[i] = 0;
    game.kempston = 0;
    zx_set_keys(m, game.rows);
    zx_set_kempston(m, 0);
    for (i = 0; i < 8u; i++)
        fmzx_keys_now[i] = m->keys[i];
    fmzx_kempston_now = m->kempston;
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
    game.keys = g.keys;
    km_reset(&kms);
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

#define BTN(b) (1u << (b))
#define BTN_SAVE 9u
#define BTN_REC 13u
#define ENC_SELECT 0u
#define ENC_ALGORITHM 1u
/* What the on-screen keyboard takes from the key map: PLAY (OK), SAVE (CAPS SHIFT), REC (SYMBOL
 * SHIFT) and the SELECT and ALGORITHM encoders (the cursor). */
#define OSK_BUTTONS (BTN(KM_BTN_PLAY) | BTN(BTN_SAVE) | BTN(BTN_REC))

/* The keyboard's key and its sticky shifts (CMD_OSK_KEY's arg) as Spectrum rows. */
static void osk_press(uint32_t arg)
{
    unsigned k = arg & 0xFFu;
    memset(osk_rows, 0, sizeof osk_rows);
    if (k < 40u)
        osk_rows[k / 5u] |= (uint8_t)(1u << (k % 5u));
    if (arg & (1u << 8))
        osk_rows[0] |= 1u; /* CAPS SHIFT = key 0 */
    if (arg & (1u << 9))
        osk_rows[7] |= 2u; /* SYMBOL SHIFT = key 36 */
    osk_down = 1;
}

/* Back to the picker: the game stops, silent and released. */
static void game_leave(void)
{
    audio_mute(1);
    audio_flush();
    if (game.loaded)
        fmzx_ram.game_release();
    km_reset(&kms);
    osk_down = 0;
    game.loaded = 0;
}

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
    case CMD_LEAVE:
        game_leave();
        break;
    case CMD_DIVIDER:
        divider = arg;
        lcd_set_divider(divider);
        budget = lcd_budget_bytes(divider);
        fmzx_stats.divider = divider;
        fmzx_stats.budget = budget;
        break;
    case CMD_OSK_KEY:
        osk_press(arg);
        break;
    default: /* CMD_RESUME: the run rule in fmzx_main does the rest */
        break;
    }
}

/* `n` detents of an encoder as `n` events: ccw (negative) `ccw`, cw `cw`. */
static void enc_events(int32_t n, int ccw, int cw)
{
    int32_t i;
    for (i = 0; i < (n < 0 ? -n : n); i++)
        handle_event(n > 0 ? cw : ccw);
}

/* Menu-style controls (picker, HOME menu, key page, message): SELECT and ALGORITHM turns and
 * OCT-/OCT+ move, PLAY is OK, HOME is BACK. */
static void menu_controls(const km_input *in, uint32_t pressed)
{
    enc_events(in->enc[ENC_SELECT], EV_UP, EV_DOWN);
    enc_events(in->enc[ENC_ALGORITHM], EV_UP, EV_DOWN);
    if (pressed & BTN(KM_BTN_PLAY))
        handle_event(EV_OK);
    else if (pressed & BTN(KM_BTN_HOME))
        handle_event(EV_BACK);
    else if (pressed & 1u) /* OCT- */
        handle_event(EV_UP);
    else if (pressed & 2u) /* OCT+ */
        handle_event(EV_DOWN);
}

/* The on-screen keyboard: SELECT moves left/right, ALGORITHM up/down, SAVE and REC toggle CAPS
 * and SYMBOL SHIFT for the next key, PLAY presses the key, HOME closes. */
static void osk_controls(const km_input *in, uint32_t pressed)
{
    enc_events(in->enc[ENC_SELECT], EV_LEFT, EV_RIGHT);
    enc_events(in->enc[ENC_ALGORITHM], EV_UP, EV_DOWN);
    if (pressed & BTN(BTN_SAVE))
        handle_event(EV_CAPS);
    if (pressed & BTN(BTN_REC))
        handle_event(EV_SYM);
    if (pressed & BTN(KM_BTN_PLAY))
        handle_event(EV_OK);
    if (pressed & BTN(KM_BTN_HOME))
        handle_event(EV_HOME);
}

static int8_t enc_add(int8_t a, int8_t b)
{
    int32_t v = (int32_t)a + b;
    return (int8_t)(v > 127 ? 127 : v < -128 ? -128 : v);
}

/* One poll. The held inputs are kept for the next frame's key mapping; the encoder detents add
 * up until then. In the game and while paused, PLAY pauses / resumes and HOME opens the menu. */
static void poll_input(void)
{
    km_input in;
    uint32_t pressed, notes, e;
    int v;
    input_poll(&in, &pressed, &notes);
    v = menu_view(&mn);
    held_in.buttons = in.buttons;
    held_in.notes = in.notes;
    play_held = (in.buttons & BTN(KM_BTN_PLAY)) != 0;
    if (v == VIEW_GAME || v == VIEW_OSK)
        for (e = v == VIEW_OSK ? 2u : 0u; e < KM_ENCODERS; e++)
            held_in.enc[e] = enc_add(held_in.enc[e], in.enc[e]);
    if (v == VIEW_GAME || v == VIEW_PAUSED) {
        if (pressed & BTN(KM_BTN_PLAY))
            handle_event(EV_PAUSE);
        else if (pressed & BTN(KM_BTN_HOME))
            handle_event(EV_HOME);
    } else if (v == VIEW_OSK) {
        osk_controls(&in, pressed);
    } else {
        menu_controls(&in, pressed);
    }
}

/* The next frame's Spectrum keys and Kempston: the held inputs through the game's key map (under
 * the keyboard without the inputs it uses), plus the keyboard's key. */
static void input_apply(void)
{
    km_input in = held_in;
    unsigned i;
    int osk = menu_view(&mn) == VIEW_OSK;
    if (osk)
        in.buttons &= ~OSK_BUTTONS;
    km_apply(&game.keys, &kms, &in, game.rows, &game.kempston);
    memset(held_in.enc, 0, sizeof held_in.enc);
    if (osk && osk_down) {
        for (i = 0; i < 8u; i++)
            game.rows[i] |= osk_rows[i];
        if (!play_held)
            osk_down = 0; /* released: this was its last frame */
    } else {
        osk_down = 0;
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

/* A Spectrum frame and everything it sends on: samples to the FIFO, changed rows to the LCD.
 * noinline: emu-gate counts its instructions (budgets) by its entry. */
static __attribute__((noinline)) void game_frame(void)
{
    unsigned r;
    /* fmzx_stats as the frame finds it: `frames` run, `pending` LCD rows still to go */
    fmzx_stats.pending = lcd_unsent();
    input_apply();
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
    int fmt = FMZX_LCD_RGB565 ? PIX_RGB565 : PIX_RGB444, view, last_view = -1;
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
            memset(held_in.enc, 0, sizeof held_in.enc);
            osk_down = 0;
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
