/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Game picker, HOME menu, key-map page, on-screen keyboard: hardware-free state machines that
 * draw into the ui.h grid. The device layer maps FM-1 controls to EV_* and acts on CMD_*. */
#ifndef FMZX_MENU_H
#define FMZX_MENU_H

#include <stdint.h>
#include "ui.h"

enum { EV_NONE, EV_UP, EV_DOWN, EV_LEFT, EV_RIGHT, EV_OK, EV_BACK, EV_HOME, EV_PAUSE, EV_CAPS, EV_SYM };
enum { VIEW_PICKER, VIEW_GAME, VIEW_MENU, VIEW_KEYS, VIEW_OSK, VIEW_PAUSED, VIEW_MESSAGE };
enum { CMD_NONE, CMD_START, CMD_RESUME, CMD_RESET, CMD_LEAVE, CMD_DIVIDER, CMD_OSK_KEY, CMD_SOUND, CMD_SCREEN };

typedef struct menu {
    uint8_t view, item, divider, osk_x, osk_y, caps, sym;
    uint8_t sound; /* SND_MODE_* (core/sound/sound.h) */
    uint8_t screen; /* PIX_CROP ... (pixels.h) */
    uint32_t count, sel, top, keys_page;
    char msg[UI_COLS + 1u];
} menu;

/* Starts in VIEW_PICKER with `count` games (from pack_open), LCD speed 4, Sound Beeper, Screen Crop. */
void menu_init(menu *mn, uint32_t game_count);
/* Returns a CMD_*. arg: CMD_START game index (the view stays VIEW_PICKER; the device then calls
 * menu_set_view(VIEW_GAME) or menu_message), CMD_DIVIDER the new divider, CMD_OSK_KEY
 * key | caps << 8 | sym << 9 with key 0-39 = Spectrum row*5+bit (the sticky shifts are cleared),
 * CMD_SOUND the new SND_MODE_* (the Sound item cycles Beeper -> FM: sound -> FM: engines ->
 * Beeper), CMD_SCREEN the new PIX_* screen mode (the Screen item, right after Sound, cycles
 * Crop -> Fit -> Fit smooth -> Border -> Crop). Both choices are kept until power off: no view
 * change, reset or game start touches them. */
int menu_event(menu *mn, int ev, uint32_t *arg);
int menu_view(const menu *mn);
void menu_set_view(menu *mn, int view);
/* VIEW_MESSAGE showing `text` centred; EV_OK or EV_BACK returns to the picker. */
void menu_message(menu *mn, const char *text);
/* The LCD speed shown in the menu; CMD_DIVIDER cycles from it 4 -> 2 -> 1 -> 0 -> 4. */
void menu_set_divider(menu *mn, unsigned d);
/* Nonzero only in VIEW_GAME and VIEW_OSK. The device runs Spectrum frames only while this is
 * nonzero and releases all Spectrum keys and Kempston whenever it is zero (state-based, so no
 * transition can be missed). Every view change also invalidates the ui grid (ui_invalidate), so
 * the next draw repaints even when its content is identical to the last one. */
int menu_runs_game(const menu *mn);
/* Nonzero when LCD row 0-239 shows the UI grid: picker/menu/keys/message every row, game none,
 * paused rows 0-23, OSK rows 200-239. */
int menu_row_is_ui(const menu *mn, unsigned lcd_row);
/* Draws the current view into the ui grid. Game names and the selected game's key map come from
 * the pack (pack_game_at); pack_base may be NULL when there are no games. */
void menu_draw(const menu *mn, const uint8_t *pack_base);

#endif
