/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "menu.h"
#include "keymap.h"
#include "pack.h"
#include "pixels.h"
#include "sound.h"
#include <string.h>

#define INK 7u
#define PAPER 0u
#define BAR_INK 0u
#define BAR_PAPER 13u    /* bright cyan */
#define TITLE_INK 14u    /* bright yellow */
#define KEY_INK 7u
#define KEY_PAPER 1u     /* blue */

#define PICKER_FIRST 1u  /* grid row of the first game name */
#define PICKER_ROWS 24u
#define KEYS_FIRST 2u
#define KEYS_PAGE 24u
#define KEYS_MAX_PAGE 4u /* 110 mappings at most, 24 per page */
#define OSK_FIRST 25u    /* grid rows 25-28 hold the keys, 29 the status line */
#define OSK_KEYS 10u
#define OSK_KEY_W 3u

enum { IT_RESUME, IT_RESET, IT_KEYBOARD, IT_KEYMAP, IT_SPEED, IT_SOUND, IT_EFFECTS, IT_SCREEN, IT_LEAVE, IT_COUNT };

/* The Sound item's choices, by SND_MODE_*. */
static const char *const sound_names[] = { "Beeper", "FM: sound", "FM: engines" };
_Static_assert(sizeof sound_names / sizeof sound_names[0] == SND_MODE_COUNT, "a name per SND_MODE_*");

/* The Screen item's choices, by PIX_* screen mode. */
static const char *const screen_names[] = { "Crop", "Fit", "Fit smooth", "Border" };
_Static_assert(sizeof screen_names / sizeof screen_names[0] == PIX_MODE_COUNT, "a name per screen mode");

static unsigned slen(const char *s)
{
    unsigned n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

/* Writes the decimal digits of v at out and returns their count. */
static unsigned put_u32(char *out, uint32_t v)
{
    char t[10];
    unsigned n = 0, i;
    do {
        t[n++] = (char)('0' + v % 10u);
        v /= 10u;
    } while (v != 0);
    for (i = 0; i < n; i++) {
        out[i] = t[n - 1u - i];
    }
    return n;
}

/* Draws s padded with spaces over the full grid width. */
static void full_row(unsigned row, const char *s, uint8_t ink, uint8_t paper)
{
    char buf[UI_COLS + 1u];
    unsigned i = 0;
    while (i < UI_COLS && s[i] != '\0') {
        buf[i] = s[i];
        i++;
    }
    while (i < UI_COLS) {
        buf[i++] = ' ';
    }
    buf[UI_COLS] = '\0';
    ui_text(0, row, buf, ink, paper);
}

static void centred(unsigned row, const char *s, uint8_t ink, uint8_t paper)
{
    unsigned n = slen(s);
    ui_text(n >= UI_COLS ? 0u : (UI_COLS - n) / 2u, row, s, ink, paper);
}

/* The Effects item exists only while Sound is FM: engines. */
static int item_shown(const menu *mn, unsigned item)
{
    return item != IT_EFFECTS || mn->sound == SND_MODE_FM_B;
}

void menu_init(menu *mn, uint32_t game_count)
{
    memset(mn, 0, sizeof *mn);
    mn->view = VIEW_PICKER;
    mn->count = game_count;
    mn->divider = 4;
    mn->sound = SND_MODE_FM_B;
    mn->effects = 1u;
    mn->screen = PIX_FIT_SMOOTH;
}

int menu_view(const menu *mn)
{
    return mn->view;
}

void menu_set_view(menu *mn, int view)
{
    ui_invalidate();
    mn->view = (uint8_t)view;
    if (view == VIEW_MENU) {
        mn->item = 0;
    } else if (view == VIEW_KEYS) {
        mn->keys_page = 0;
    } else if (view == VIEW_OSK) {
        mn->caps = 0;
        mn->sym = 0;
    }
}

void menu_message(menu *mn, const char *text)
{
    unsigned i = 0;
    while (i < UI_COLS && text[i] != '\0') {
        mn->msg[i] = text[i];
        i++;
    }
    mn->msg[i] = '\0';
    ui_invalidate();
    mn->view = VIEW_MESSAGE;
}

void menu_set_divider(menu *mn, unsigned d)
{
    mn->divider = (uint8_t)d;
}

int menu_runs_game(const menu *mn)
{
    return mn->view == VIEW_GAME || mn->view == VIEW_OSK;
}

int menu_row_is_ui(const menu *mn, unsigned lcd_row)
{
    switch (mn->view) {
    case VIEW_GAME:
        return 0;
    case VIEW_PAUSED:
        return lcd_row < LCD_TOP_BORDER;
    case VIEW_OSK:
        return lcd_row >= LCD_ROWS - 5u * 8u && lcd_row < LCD_ROWS;
    default:
        return lcd_row < LCD_ROWS;
    }
}

/* ---- on-screen keyboard layout: the Spectrum's physical rows ---- */

/* Spectrum key index (row*5 + bit) for cursor column x (0-9) of physical row y (0-3). */
static unsigned osk_key(unsigned x, unsigned y)
{
    switch (y) {
    case 0:
        return x < 5u ? 15u + x : 29u - x;   /* 1-5 on row 3, 6-0 on row 4 */
    case 1:
        return x < 5u ? 10u + x : 34u - x;   /* Q-T on row 2, Y-P on row 5 */
    case 2:
        return x < 5u ? 5u + x : 39u - x;    /* A-G on row 1, H-ENTER on row 6 */
    default:
        return x < 5u ? x : 44u - x;         /* CAPS Z-V on row 0, B-SPACE on row 7 */
    }
}

static const char key_chars[40] = {
    ' ', 'Z', 'X', 'C', 'V',
    'A', 'S', 'D', 'F', 'G',
    'Q', 'W', 'E', 'R', 'T',
    '1', '2', '3', '4', '5',
    '0', '9', '8', '7', '6',
    'P', 'O', 'I', 'U', 'Y',
    ' ', 'L', 'K', 'J', 'H',
    ' ', ' ', 'M', 'N', 'B',
};

static const char *key_name(unsigned k, char *buf)
{
    switch (k) {
    case 0:
        return "CAPS";
    case 30:
        return "ENTER";
    case 35:
        return "SPACE";
    case 36:
        return "SYM";
    default:
        buf[0] = key_chars[k];
        buf[1] = '\0';
        return buf;
    }
}

static const char *key_label(unsigned k)
{
    switch (k) {
    case 0:
        return "CAP";
    case 30:
        return "ENT";
    case 35:
        return "SPC";
    case 36:
        return "SYM";
    default:
        return 0;
    }
}

/* ---- key map page ---- */

static const char *const btn_names[KM_BUTTONS] = {
    "OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC",
};
static const char *const enc_names[KM_ENCODERS] = { "SEL", "ALG", "K1", "K2", "K3", "K4", "PRE" };
static const char *const note_names[12] = { "F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E" };
static const char *const kemp_names[5] = { "RIGHT", "LEFT", "DOWN", "UP", "FIRE" };

/* Appends s to out at *n. */
static void put_str(char *out, unsigned *n, const char *s)
{
    while (*s != '\0') {
        out[(*n)++] = *s++;
    }
}

static void input_name(unsigned input, char *out, unsigned *n)
{
    if (input < KM_BUTTONS) {
        put_str(out, n, btn_names[input]);
    } else if (input < KM_BUTTONS + KM_NOTES) {
        unsigned note = input - KM_BUTTONS;       /* 0 = F3 */
        put_str(out, n, note_names[note % 12u]);
        *n += put_u32(out + *n, 3u + (note + 5u) / 12u); /* octaves change at C: note 7 = C4 */
    } else {
        unsigned e = (input - KM_BUTTONS - KM_NOTES) / 2u;
        put_str(out, n, enc_names[e]);
        out[(*n)++] = ((input - KM_BUTTONS - KM_NOTES) & 1u) ? '+' : '-';
    }
}

/* The text of mapping line `line` (0-based, notes, then buttons, then encoders; one line per
 * action that is set) into out (NUL-terminated, at most UI_COLS chars); 0 if there is no such line. */
static int keys_line(const keymap *k, unsigned line, char *out)
{
    unsigned pass, i, a, n = 0;
    for (pass = 0; pass < 3u; pass++) {
        unsigned lo = pass == 0 ? KM_BUTTONS : pass == 1 ? 0u : KM_BUTTONS + KM_NOTES;
        unsigned hi = pass == 0 ? KM_BUTTONS + KM_NOTES : pass == 1 ? KM_BUTTONS : KM_INPUTS;
        for (i = lo; i < hi; i++) {
            for (a = 0; a < KM_ACTIONS; a++) {
                uint8_t act = k->act[i][a];
                if (act == KM_NONE || (act >= 40u && (act < 0x40u || act > 0x44u))) {
                    continue;
                }
                if (n++ == line) {
                    unsigned len = 0;
                    char kb[2];
                    input_name(i, out, &len);
                    while (len < 5u) {
                        out[len++] = ' ';
                    }
                    if (act >= 0x40u) {
                        put_str(out, &len, "KEMPSTON ");
                        put_str(out, &len, kemp_names[act - 0x40u]);
                    } else {
                        put_str(out, &len, key_name(act, kb));
                    }
                    out[len] = '\0';
                    return 1;
                }
            }
        }
    }
    return 0;
}

static unsigned keys_total(const keymap *k)
{
    char tmp[UI_COLS + 12u];
    unsigned n = 0;
    while (keys_line(k, n, tmp)) {
        n++;
    }
    return n;
}

/* ---- events ---- */

static unsigned next_divider(unsigned d)
{
    return d == 0u ? 4u : d >= 3u ? 2u : d - 1u;
}

static int event_inner(menu *mn, int ev, uint32_t *arg)
{
    *arg = 0;
    switch (mn->view) {
    case VIEW_PICKER:
        if (mn->count == 0) {
            return CMD_NONE;
        }
        if (ev == EV_UP) {
            mn->sel = mn->sel == 0 ? mn->count - 1u : mn->sel - 1u;
        } else if (ev == EV_DOWN) {
            mn->sel = mn->sel + 1u >= mn->count ? 0u : mn->sel + 1u;
        } else if (ev == EV_OK) {
            *arg = mn->sel;
            return CMD_START;
        }
        if (mn->sel < mn->top) {
            mn->top = mn->sel;
        } else if (mn->sel >= mn->top + PICKER_ROWS) {
            mn->top = mn->sel - (PICKER_ROWS - 1u);
        }
        return CMD_NONE;

    case VIEW_GAME:
        if (ev == EV_HOME) {
            menu_set_view(mn, VIEW_MENU);
        } else if (ev == EV_PAUSE) {
            mn->view = VIEW_PAUSED;
        }
        return CMD_NONE;

    case VIEW_PAUSED:
        if (ev == EV_PAUSE) {
            mn->view = VIEW_GAME;
        } else if (ev == EV_HOME) {
            menu_set_view(mn, VIEW_MENU);
        }
        return CMD_NONE;

    case VIEW_MENU:
        if (ev == EV_UP) {
            mn->item = (uint8_t)(mn->item == 0 ? IT_COUNT - 1u : mn->item - 1u);
            if (!item_shown(mn, mn->item)) {
                mn->item--;
            }
        } else if (ev == EV_DOWN) {
            mn->item = (uint8_t)(mn->item + 1u >= IT_COUNT ? 0u : mn->item + 1u);
            if (!item_shown(mn, mn->item)) {
                mn->item++;
            }
        } else if (ev == EV_HOME || ev == EV_BACK) {
            mn->view = VIEW_GAME;
            return CMD_RESUME;
        } else if (ev == EV_OK) {
            switch (mn->item) {
            case IT_RESUME:
                mn->view = VIEW_GAME;
                return CMD_RESUME;
            case IT_RESET:
                mn->view = VIEW_GAME;
                return CMD_RESET;
            case IT_KEYBOARD:
                menu_set_view(mn, VIEW_OSK);
                break;
            case IT_KEYMAP:
                menu_set_view(mn, VIEW_KEYS);
                break;
            case IT_SPEED:
                mn->divider = (uint8_t)next_divider(mn->divider);
                *arg = mn->divider;
                return CMD_DIVIDER;
            case IT_SOUND:
                mn->sound = (uint8_t)(mn->sound + 1u >= SND_MODE_COUNT ? 0u : mn->sound + 1u);
                *arg = mn->sound;
                return CMD_SOUND;
            case IT_EFFECTS:
                mn->effects = (uint8_t)!mn->effects;
                *arg = mn->effects;
                return CMD_EFFECTS;
            case IT_SCREEN:
                mn->screen = (uint8_t)(mn->screen + 1u >= PIX_MODE_COUNT ? 0u : mn->screen + 1u);
                *arg = mn->screen;
                return CMD_SCREEN;
            default:
                mn->view = VIEW_PICKER;
                return CMD_LEAVE;
            }
        }
        return CMD_NONE;

    case VIEW_KEYS:
        if (ev == EV_BACK || ev == EV_HOME) {
            mn->view = VIEW_MENU;
        } else if (ev == EV_DOWN) {
            if (mn->keys_page < KEYS_MAX_PAGE) {
                mn->keys_page++;
            }
        } else if (ev == EV_UP) {
            if (mn->keys_page > 0) {
                mn->keys_page--;
            }
        }
        return CMD_NONE;

    case VIEW_OSK:
        if (ev == EV_LEFT) {
            mn->osk_x = (uint8_t)(mn->osk_x == 0 ? OSK_KEYS - 1u : mn->osk_x - 1u);
        } else if (ev == EV_RIGHT) {
            mn->osk_x = (uint8_t)(mn->osk_x + 1u >= OSK_KEYS ? 0u : mn->osk_x + 1u);
        } else if (ev == EV_UP) {
            mn->osk_y = (uint8_t)(mn->osk_y == 0 ? 3u : mn->osk_y - 1u);
        } else if (ev == EV_DOWN) {
            mn->osk_y = (uint8_t)(mn->osk_y >= 3u ? 0u : mn->osk_y + 1u);
        } else if (ev == EV_CAPS) {
            mn->caps ^= 1u;
        } else if (ev == EV_SYM) {
            mn->sym ^= 1u;
        } else if (ev == EV_HOME || ev == EV_BACK) {
            mn->view = VIEW_GAME;
        } else if (ev == EV_OK) {
            *arg = osk_key(mn->osk_x, mn->osk_y) | ((uint32_t)mn->caps << 8) | ((uint32_t)mn->sym << 9);
            mn->caps = 0;
            mn->sym = 0;
            return CMD_OSK_KEY;
        }
        return CMD_NONE;

    default: /* VIEW_MESSAGE */
        if (ev == EV_OK || ev == EV_BACK) {
            mn->view = VIEW_PICKER;
        }
        return CMD_NONE;
    }
}

int menu_event(menu *mn, int ev, uint32_t *arg)
{
    uint8_t before = mn->view;
    int c = event_inner(mn, ev, arg);
    if (mn->view != before) {
        ui_invalidate();
    }
    return c;
}

/* ---- drawing ---- */

static void draw_picker(const menu *mn, const uint8_t *pack)
{
    unsigned r;
    ui_clear(INK, PAPER);
    centred(0, "FM-1 ZX", TITLE_INK, PAPER);
    if (mn->count == 0) {
        centred(14, "No games", INK, PAPER);
        return;
    }
    for (r = 0; r < PICKER_ROWS && mn->top + r < mn->count; r++) {
        uint32_t i = mn->top + r;
        int sel = i == mn->sel;
        char line[UI_COLS + 1u];
        pack_game g;
        const char *name = "(bad entry)";
        if (pack != 0 && pack_game_at(pack, i, &g) == 0) {
            name = g.name;
        }
        line[0] = ' ';
        {
            unsigned n = 1;
            while (n < UI_COLS && name[n - 1u] != '\0') {
                line[n] = name[n - 1u];
                n++;
            }
            line[n] = '\0';
        }
        full_row(PICKER_FIRST + r, line, sel ? BAR_INK : INK, sel ? BAR_PAPER : PAPER);
    }
    {
        char foot[24];
        unsigned n = put_u32(foot, mn->sel + 1u);
        foot[n++] = '/';
        n += put_u32(foot + n, mn->count);
        foot[n] = '\0';
        ui_text(UI_COLS - n - 1u, UI_ROWS - 1u, foot, INK, PAPER);
    }
}

static void draw_menu(const menu *mn)
{
    static const char *const items[IT_COUNT] = {
        "Resume", "Reset game", "Keyboard", "Key map", "LCD speed: ", "Sound: ", "Effects: ", "Screen: ", "Back to games",
    };
    unsigned i, row = 0;
    ui_clear(INK, PAPER);
    centred(0, "FM-1 ZX", TITLE_INK, PAPER);
    for (i = 0; i < IT_COUNT; i++) {
        char line[UI_COLS + 1u];
        unsigned n = 0;
        if (!item_shown(mn, i)) {
            continue;
        }
        int sel = i == mn->item;
        line[n++] = ' ';
        put_str(line, &n, items[i]);
        if (i == IT_SPEED) {
            n += put_u32(line + n, mn->divider);
        } else if (i == IT_SOUND) {
            put_str(line, &n, sound_names[mn->sound < SND_MODE_COUNT ? mn->sound : 0u]);
        } else if (i == IT_EFFECTS) {
            put_str(line, &n, mn->effects ? "FM" : "Beeper");
        } else if (i == IT_SCREEN) {
            put_str(line, &n, screen_names[mn->screen < PIX_MODE_COUNT ? mn->screen : 0u]);
        }
        line[n] = '\0';
        full_row(4u + 2u * row++, line, sel ? BAR_INK : INK, sel ? BAR_PAPER : PAPER);
    }
}

static void draw_keys(const menu *mn, const uint8_t *pack)
{
    keymap km;
    pack_game g;
    char title[UI_COLS + 1u];
    unsigned n = 0, total, page, r;
    uint32_t pages;
    if (pack != 0 && mn->count != 0 && pack_game_at(pack, mn->sel, &g) == 0) {
        km = g.keys;
        put_str(title, &n, "KEYS ");
        while (n < UI_COLS && g.name[n - 5u] != '\0') {
            title[n] = g.name[n - 5u];
            n++;
        }
    } else {
        km_default(&km);
        put_str(title, &n, "KEYS");
    }
    title[n] = '\0';
    ui_clear(INK, PAPER);
    ui_text(0, 0, title, TITLE_INK, PAPER);
    total = keys_total(&km);
    pages = (total + KEYS_PAGE - 1u) / KEYS_PAGE;
    if (pages == 0) {
        pages = 1;
    }
    page = mn->keys_page < pages ? (unsigned)mn->keys_page : (unsigned)(pages - 1u);
    for (r = 0; r < KEYS_PAGE; r++) {
        char line[UI_COLS + 12u];
        if (!keys_line(&km, page * KEYS_PAGE + r, line)) {
            break;
        }
        ui_text(0, KEYS_FIRST + r, line, INK, PAPER);
    }
    {
        char foot[24];
        unsigned f = 0;
        put_str(foot, &f, "PAGE ");
        f += put_u32(foot + f, page + 1u);
        foot[f++] = '/';
        f += put_u32(foot + f, pages);
        foot[f] = '\0';
        ui_text(0, UI_ROWS - 1u, foot, INK, PAPER);
    }
}

static void draw_osk(const menu *mn)
{
    unsigned x, y;
    for (y = 0; y < 4u; y++) {
        for (x = 0; x < OSK_KEYS; x++) {
            unsigned k = osk_key(x, y);
            const char *lbl = key_label(k);
            char cell[OSK_KEY_W + 1u];
            int cur = x == mn->osk_x && y == mn->osk_y;
            if (lbl != 0) {
                memcpy(cell, lbl, OSK_KEY_W);
            } else {
                cell[0] = ' ';
                cell[1] = key_chars[k];
                cell[2] = ' ';
            }
            cell[OSK_KEY_W] = '\0';
            ui_text(x * OSK_KEY_W, OSK_FIRST + y, cell, cur ? BAR_INK : KEY_INK, cur ? BAR_PAPER : KEY_PAPER);
        }
    }
    full_row(OSK_FIRST + 4u, "", INK, PAPER);
    ui_text(0, OSK_FIRST + 4u, "CAPS SHIFT", mn->caps ? BAR_INK : INK, mn->caps ? BAR_PAPER : PAPER);
    ui_text(12, OSK_FIRST + 4u, "SYMBOL SHIFT", mn->sym ? BAR_INK : INK, mn->sym ? BAR_PAPER : PAPER);
}

static void draw_paused(void)
{
    unsigned r;
    for (r = 0; r < 3u; r++) {
        full_row(r, "", INK, PAPER);
    }
    centred(1, "PAUSED", TITLE_INK, PAPER);
}

void menu_draw(const menu *mn, const uint8_t *pack_base)
{
    switch (mn->view) {
    case VIEW_PICKER:
        draw_picker(mn, pack_base);
        break;
    case VIEW_MENU:
        draw_menu(mn);
        break;
    case VIEW_KEYS:
        draw_keys(mn, pack_base);
        break;
    case VIEW_OSK:
        draw_osk(mn);
        break;
    case VIEW_PAUSED:
        draw_paused();
        break;
    case VIEW_MESSAGE:
        ui_clear(INK, PAPER);
        centred(14, mn->msg, INK, PAPER);
        break;
    default:
        break;
    }
}
