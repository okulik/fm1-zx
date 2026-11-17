// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_sys::{PIX_BORDER, PIX_CROP, PIX_FIT, PIX_FIT_SMOOTH, PIX_MODE_COUNT, SND_MODE_BEEPER, SND_MODE_COUNT, SND_MODE_FM_A, SND_MODE_FM_B};
use zx_tests::*;

const ROM: &[u8] = include_bytes!("../../../../third_party/zx-rom/48.rom");

fn rows_text(ui: &Ui) -> Vec<String> {
    (0..UI_ROWS).map(|r| ui.row_text(r)).collect()
}

fn find_row(ui: &Ui, needle: &str) -> Option<usize> {
    rows_text(ui).iter().position(|t| t.contains(needle))
}

#[test]
fn font_from_rom() {
    let mut ui = Ui::new();
    ui.text(0, 0, "A", 7, 0);
    for y in 0..8 {
        let bits = ROM[0x3D00 + (65 - 32) * 8 + y];
        let row = ui.compose_row(y);
        for (x, &px) in row[..8].iter().enumerate() {
            let want = if bits & (0x80 >> x) != 0 { 7 } else { 0 };
            assert_eq!(px, want, "y={y} x={x}");
        }
    }
    // colours and a high-bit char
    ui.text(1, 0, "\u{e9}", 14, 1);
    let row = ui.compose_row(3);
    assert!(row[8..16].iter().all(|&p| p == 14 || p == 1));
    assert_eq!(ui.row_text(0).chars().nth(1), Some('?'));
}

#[test]
fn dirty_rows() {
    let mut ui = Ui::new();
    let (n, _) = ui.dirty_rows();
    assert_eq!(n, 240, "everything is dirty after init");
    assert_eq!(ui.dirty_rows().0, 0);
    ui.text(3, 5, "X", 7, 0);
    let (n, bits) = ui.dirty_rows();
    assert_eq!(n, 8);
    for (i, b) in bits.iter().enumerate() {
        assert_eq!(*b, if i == 5 { 0xFF } else { 0 }, "byte {i}");
    }
    // rewriting the same text, or clearing and redrawing it, changes nothing
    ui.text(3, 5, "X", 7, 0);
    ui.clear(7, 0);
    ui.text(3, 5, "X", 7, 0);
    assert_eq!(ui.dirty_rows().0, 0);
}

#[test]
fn picker_wrap_and_start() {
    let mut m = Menu::new(3);
    assert_eq!(m.view(), view::PICKER);
    assert_eq!(m.event(ev::UP), (cmd::NONE, 0));
    assert_eq!(m.event(ev::OK), (cmd::START, 2));
    assert_eq!(m.view(), view::PICKER);
    assert_eq!(m.event(ev::DOWN).0, cmd::NONE);
    assert_eq!(m.event(ev::OK), (cmd::START, 0));
}

#[test]
fn picker_empty() {
    let mut ui = Ui::new();
    let mut m = Menu::new(0);
    ui.draw(&m, None);
    assert!(find_row(&ui, "No games").is_some());
    assert_eq!(m.event(ev::OK).0, cmd::NONE);
    assert_eq!(m.event(ev::UP).0, cmd::NONE);
    assert_eq!(m.event(ev::DOWN).0, cmd::NONE);
}

#[test]
fn picker_lists_and_scrolls() {
    let names: Vec<String> = (0..30).map(|i| format!("Game {i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pack = build_pack(&refs);
    let mut ui = Ui::new();
    let mut m = Menu::new(30);
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "FM-1 ZX").is_none());
    assert!(find_row(&ui, "Game 00").is_some());
    assert!(find_row(&ui, "Game 19").is_some());
    assert!(find_row(&ui, "Game 20").is_none());
    // the selected row has the bright cyan bar
    let r = find_row(&ui, "Game 00").unwrap();
    assert_eq!(ui.compose_row(r * 8 + 3)[239], 13);
    for _ in 0..26 {
        m.event(ev::DOWN);
    }
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "Game 00").is_none());
    let r = find_row(&ui, "Game 26").unwrap();
    assert_eq!(ui.compose_row(r * 8 + 3)[239], 13);
    // wrap from the top to the end
    let mut m = Menu::new(30);
    m.event(ev::UP);
    ui.draw(&m, Some(&pack));
    let r = find_row(&ui, "Game 29").unwrap();
    assert_eq!(ui.compose_row(r * 8 + 3)[239], 13);
}

fn open_menu() -> Menu {
    let mut m = Menu::new(2);
    m.set_view(view::GAME);
    assert_eq!(m.event(ev::HOME).0, cmd::NONE);
    assert_eq!(m.view(), view::MENU);
    m
}

#[test]
fn menu_items_and_divider_cycle() {
    let mut ui = Ui::new();
    let mut m = open_menu();
    ui.draw(&m, None);
    for item in ["Resume", "Reset game", "Keyboard", "Key map", "LCD speed: 4", "Sound: FM: engines", "Effects: FM", "Screen: Fit smooth", "Back to games"] {
        assert!(find_row(&ui, item).is_some(), "{item}");
    }
    for _ in 0..4 {
        m.event(ev::DOWN);
    }
    let mut args = vec![];
    for _ in 0..4 {
        let (c, a) = m.event(ev::OK);
        assert_eq!(c, cmd::DIVIDER);
        args.push(a);
    }
    assert_eq!(args, [2, 1, 0, 4]);
    assert_eq!(m.view(), view::MENU);
    m.set_divider(1);
    ui.draw(&m, None);
    assert!(find_row(&ui, "LCD speed: 1").is_some());
    assert_eq!(m.event(ev::OK), (cmd::DIVIDER, 0));
}

/// The HOME menu's Sound item: 5 items down.
const SOUND_ITEM: usize = 5;

/// Opens the HOME menu from the game and moves to the Sound item.
fn to_sound(m: &mut Menu) {
    m.set_view(view::GAME);
    assert_eq!(m.event(ev::HOME).0, cmd::NONE);
    for _ in 0..SOUND_ITEM {
        m.event(ev::DOWN);
    }
}

fn sound_shown(m: &Menu) -> String {
    let mut ui = Ui::new();
    ui.draw(m, None);
    rows_text(&ui).into_iter().find(|t| t.contains("Sound: ")).expect("no Sound item").trim().to_string()
}

#[test]
fn menu_sound_cycles_and_survives() {
    assert_eq!(SND_MODE_COUNT, 3, "Beeper, FM: sound and FM: engines");
    let mut m = Menu::new(2);
    to_sound(&mut m);
    assert_eq!(sound_shown(&m), "Sound: FM: engines", "FM: engines by default");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_BEEPER as u32));
    assert_eq!(m.view(), view::MENU, "the item stays in the menu");
    assert_eq!(sound_shown(&m), "Sound: Beeper");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_A as u32));
    assert_eq!(sound_shown(&m), "Sound: FM: sound");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_B as u32));
    assert_eq!(sound_shown(&m), "Sound: FM: engines");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_BEEPER as u32));
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_A as u32));
    // reset game: back in the game, the choice kept
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    assert_eq!(m.event(ev::OK).0, cmd::RESET);
    assert_eq!(m.view(), view::GAME);
    m.event(ev::HOME);
    assert_eq!(sound_shown(&m), "Sound: FM: sound", "after a reset");
    // back to the picker and another game started
    m.event(ev::UP); // wraps to "Back to games"
    assert_eq!(m.event(ev::OK).0, cmd::LEAVE);
    assert_eq!(m.view(), view::PICKER);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK), (cmd::START, 1));
    to_sound(&mut m);
    assert_eq!(sound_shown(&m), "Sound: FM: sound", "after starting another game");
    // and the next press cycles from the kept choice
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_B as u32));
}

/// The HOME menu's Screen item: after Sound and its Effects row.
/// (Sound is FM: engines by default, so the Effects row sits between them.)
const SCREEN_ITEM: usize = SOUND_ITEM + 2;

/// Opens the HOME menu from the game and moves to the Screen item.
fn to_screen(m: &mut Menu) {
    m.set_view(view::GAME);
    assert_eq!(m.event(ev::HOME).0, cmd::NONE);
    for _ in 0..SCREEN_ITEM {
        m.event(ev::DOWN);
    }
}

fn screen_shown(m: &Menu) -> String {
    let mut ui = Ui::new();
    ui.draw(m, None);
    let rows = rows_text(&ui);
    let i = rows.iter().position(|t| t.contains("Screen: ")).expect("no Screen item");
    let j = rows.iter().position(|t| t.contains("Sound: ")).expect("no Sound item");
    assert_eq!(i, j + 4, "the Screen item is not two lines after Sound (Effects between)");
    rows[i].trim().to_string()
}

#[test]
fn menu_screen_cycles_and_survives() {
    assert_eq!(PIX_MODE_COUNT, 4, "Crop, Fit, Fit smooth and Border");
    let mut m = Menu::new(2);
    to_screen(&mut m);
    assert_eq!(screen_shown(&m), "Screen: Fit smooth", "Fit smooth by default");
    for (mode, label) in [(PIX_BORDER, "Screen: Border"), (PIX_CROP, "Screen: Crop"), (PIX_FIT, "Screen: Fit"), (PIX_FIT_SMOOTH, "Screen: Fit smooth")] {
        assert_eq!(m.event(ev::OK), (cmd::SCREEN, mode));
        assert_eq!(m.view(), view::MENU, "the item stays in the menu");
        assert_eq!(screen_shown(&m), label);
    }
    assert_eq!(m.event(ev::OK), (cmd::SCREEN, PIX_BORDER));
    assert_eq!(m.event(ev::OK), (cmd::SCREEN, PIX_CROP));
    // reset game: back in the game, the choice kept
    for _ in 0..SCREEN_ITEM - 1 {
        m.event(ev::UP);
    }
    assert_eq!(m.event(ev::OK).0, cmd::RESET);
    assert_eq!(m.view(), view::GAME);
    m.event(ev::HOME);
    assert_eq!(screen_shown(&m), "Screen: Crop", "after a reset");
    // back to the picker and another game started
    m.event(ev::UP); // wraps to "Back to games"
    assert_eq!(m.event(ev::OK).0, cmd::LEAVE);
    assert_eq!(m.view(), view::PICKER);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK), (cmd::START, 1));
    to_screen(&mut m);
    assert_eq!(screen_shown(&m), "Screen: Crop", "after starting another game");
    // and the next press cycles from the kept choice; Sound is untouched
    assert_eq!(m.event(ev::OK), (cmd::SCREEN, PIX_FIT));
    assert_eq!(sound_shown(&m), "Sound: FM: engines");
}

fn row_with(m: &Menu, text: &str) -> Option<String> {
    let mut ui = Ui::new();
    ui.draw(m, None);
    rows_text(&ui).into_iter().find(|t| t.contains(text)).map(|t| t.trim().to_string())
}

fn rows_of(m: &Menu) -> Vec<String> {
    let mut ui = Ui::new();
    ui.draw(m, None);
    rows_text(&ui)
}

#[test]
fn menu_effects_row_only_in_fm_engines() {
    let mut m = Menu::new(2);
    to_sound(&mut m);
    let rows = rows_of(&m);
    let s = rows.iter().position(|t| t.contains("Sound: ")).unwrap();
    assert_eq!(rows[s + 2].trim(), "Effects: FM", "directly under Sound");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_BEEPER as u32));
    assert!(row_with(&m, "Effects:").is_none(), "hidden on Beeper");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_A as u32));
    assert!(row_with(&m, "Effects:").is_none(), "hidden on FM: sound");
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_FM_B as u32));
    assert_eq!(row_with(&m, "Effects:").as_deref(), Some("Effects: FM"));
    // a kept Beeper value comes back
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK), (cmd::EFFECTS, 0));
    m.event(ev::UP);
    m.event(ev::OK);
    m.event(ev::OK);
    m.event(ev::OK);
    assert_eq!(row_with(&m, "Effects:").as_deref(), Some("Effects: Beeper"), "value kept");
}

#[test]
fn menu_effects_toggles_and_survives() {
    let mut m = Menu::new(2);
    to_sound(&mut m);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK), (cmd::EFFECTS, 0));
    assert_eq!(m.view(), view::MENU);
    assert_eq!(row_with(&m, "Effects:").as_deref(), Some("Effects: Beeper"));
    assert_eq!(m.event(ev::OK), (cmd::EFFECTS, 1));
    assert_eq!(row_with(&m, "Effects:").as_deref(), Some("Effects: FM"));
    assert_eq!(m.event(ev::OK), (cmd::EFFECTS, 0));
    // back to the picker and another game started
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP);
    m.event(ev::UP); // wraps to "Back to games"
    assert_eq!(m.event(ev::OK).0, cmd::LEAVE);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK), (cmd::START, 1));
    to_sound(&mut m);
    m.event(ev::DOWN);
    assert_eq!(row_with(&m, "Effects:").as_deref(), Some("Effects: Beeper"), "after starting another game");
    assert_eq!(m.event(ev::OK), (cmd::EFFECTS, 1));
}

#[test]
fn menu_navigation_skips_hidden_effects() {
    let mut m = Menu::new(2);
    to_sound(&mut m);
    assert_eq!(m.event(ev::OK), (cmd::SOUND, SND_MODE_BEEPER as u32));
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK).0, cmd::SCREEN, "DOWN from Sound lands on Screen");
    m.event(ev::UP);
    assert_eq!(m.event(ev::OK).0, cmd::SOUND, "UP from Screen lands on Sound");
}

#[test]
fn menu_resume_reset_back() {
    let mut m = open_menu();
    assert_eq!(m.event(ev::OK).0, cmd::RESUME);
    assert_eq!(m.view(), view::GAME);
    m.event(ev::HOME);
    assert_eq!(m.event(ev::HOME).0, cmd::RESUME);
    assert_eq!(m.view(), view::GAME);
    m.event(ev::HOME);
    assert_eq!(m.event(ev::BACK).0, cmd::RESUME);
    m.event(ev::HOME);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK).0, cmd::RESET);
    assert_eq!(m.view(), view::GAME);
}

#[test]
fn menu_leave() {
    let mut m = open_menu();
    m.event(ev::UP); // wraps to the last item
    assert_eq!(m.event(ev::OK).0, cmd::LEAVE);
    assert_eq!(m.view(), view::PICKER);
}

#[test]
fn paused_and_message() {
    let mut ui = Ui::new();
    let mut m = Menu::new(1);
    m.set_view(view::GAME);
    assert_eq!(m.event(ev::PAUSE).0, cmd::NONE);
    assert_eq!(m.view(), view::PAUSED);
    ui.draw(&m, None);
    assert!((0..3).any(|r| ui.row_text(r).contains("PAUSED")));
    assert_eq!(m.event(ev::PAUSE).0, cmd::NONE);
    assert_eq!(m.view(), view::GAME);
    m.message("Load failed");
    assert_eq!(m.view(), view::MESSAGE);
    ui.draw(&m, None);
    let r = find_row(&ui, "Load failed").unwrap();
    let t = ui.row_text(r);
    let lead = t.len() - t.trim_start().len();
    assert!((9..=11).contains(&lead), "centred, lead {lead}");
    assert_eq!(m.event(ev::OK).0, cmd::NONE);
    assert_eq!(m.view(), view::PICKER);
    m.message("x");
    m.event(ev::BACK);
    assert_eq!(m.view(), view::PICKER);
}

#[test]
fn row_is_ui() {
    let mut m = Menu::new(1);
    let spans = |m: &Menu| -> Vec<bool> { (0..240).map(|r| m.row_is_ui(r)).collect() };
    for v in [view::PICKER, view::MENU, view::KEYS, view::MESSAGE] {
        m.set_view(v);
        assert!(spans(&m).iter().all(|&b| b), "view {v}");
    }
    m.set_view(view::GAME);
    assert!(spans(&m).iter().all(|&b| !b));
    m.set_view(view::PAUSED);
    assert!(spans(&m).iter().enumerate().all(|(r, &b)| b == (r < 24)));
    m.set_view(view::OSK);
    assert!(spans(&m).iter().enumerate().all(|(r, &b)| b == (r >= 200)));
}

#[test]
fn osk_navigation_and_shift() {
    let mut m = Menu::new(1);
    m.set_view(view::OSK);
    m.event(ev::SYM);
    m.event(ev::DOWN); // Q row
    m.event(ev::LEFT); // wraps to P
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 25 | (1 << 9)));
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 25), "shifts cleared after one key");
    m.event(ev::CAPS);
    m.event(ev::SYM);
    m.event(ev::CAPS); // toggles off again
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 25 | (1 << 9)));
    assert_eq!(m.view(), view::OSK);
    assert_eq!(m.event(ev::HOME).0, cmd::NONE);
    assert_eq!(m.view(), view::GAME);
}

#[test]
fn osk_layout_matches_spectrum() {
    // physical rows: 1-0 / Q-P / A-ENTER / CAPS Z-M SYM SPACE
    let want: [[u32; 10]; 4] = [
        [15, 16, 17, 18, 19, 24, 23, 22, 21, 20],
        [10, 11, 12, 13, 14, 29, 28, 27, 26, 25],
        [5, 6, 7, 8, 9, 34, 33, 32, 31, 30],
        [0, 1, 2, 3, 4, 39, 38, 37, 36, 35],
    ];
    let mut m = Menu::new(1);
    m.set_view(view::OSK);
    for row in want {
        for key in row {
            assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, key));
            m.event(ev::RIGHT);
        }
        m.event(ev::DOWN);
    }
    // vertical wrap
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 15));
    m.event(ev::UP);
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 0));
}

#[test]
fn osk_draws_bottom_rows() {
    let mut ui = Ui::new();
    let mut m = Menu::new(1);
    m.set_view(view::OSK);
    ui.draw(&m, None);
    assert!(ui.row_text(25).contains('1') && ui.row_text(25).contains('0'));
    assert!(ui.row_text(26).contains('Q'));
    assert!(ui.row_text(28).contains("CAP") && ui.row_text(28).contains("SPC"));
    // cursor at (0,0) is bright cyan, other keys white on blue
    assert_eq!(ui.compose_row(200)[0], 13);
    assert_eq!(ui.compose_row(200)[3 * 8], 1);
    m.event(ev::SYM);
    ui.draw(&m, None);
    assert!(ui.row_text(29).contains("SYMBOL"));
    assert_eq!(ui.compose_row(232)[12 * 8], 13, "SYM latched");
    assert_eq!(ui.compose_row(232)[0], 0, "CAPS not latched");
}

/// KEYS_TOP moves the on-screen keyboard to the top of the screen (grid rows 0-4, LCD rows 0-39),
/// KEYS_BOTTOM back; the place is kept when the keyboard closes and opens again.
#[test]
fn osk_moves_between_top_and_bottom() {
    let mut ui = Ui::new();
    let mut m = Menu::new(1);
    m.set_view(view::MENU);
    ui.draw(&m, None); // the menu art on, as the device has it when the keyboard opens
    m.set_view(view::OSK);
    let bottom = m.layout();
    assert_eq!(m.event(ev::KEYS_TOP).0, cmd::NONE);
    assert_eq!(m.view(), view::OSK);
    assert_ne!(m.layout(), bottom, "the layout changes with the place");
    assert!((0..240).all(|r| m.row_is_ui(r) == (r < 40)), "UI rows 0-39");
    ui.draw(&m, None);
    assert!(ui.row_text(0).contains('1') && ui.row_text(0).contains('0'));
    assert!(ui.row_text(1).contains('Q'));
    assert!(ui.row_text(3).contains("CAP") && ui.row_text(3).contains("SPC"));
    assert!(ui.row_text(4).contains("CAPS SHIFT"));
    assert_eq!(ui.compose_row(0)[0], 13, "cursor at the top");
    assert_eq!(m.event(ev::OK), (cmd::OSK_KEY, 15), "keys work at the top");
    m.event(ev::HOME);
    m.set_view(view::OSK);
    assert!(m.row_is_ui(0) && !m.row_is_ui(239), "still at the top after closing");
    m.event(ev::KEYS_BOTTOM);
    assert_eq!(m.layout(), bottom);
    assert!((0..240).all(|r| m.row_is_ui(r) == (r >= 200)), "UI rows 200-239");
}

/// The PAUSED bar after the HOME menu (whose art covers rows 0-63) shows PAUSED, not the art.
#[test]
fn paused_after_menu_has_no_art() {
    let mut ui = Ui::new();
    let mut m = Menu::new(1);
    m.set_view(view::MENU);
    ui.draw(&m, None);
    m.set_view(view::PAUSED);
    ui.draw(&m, None);
    assert!(ui.compose_row(12).iter().all(|&p| p < 16), "art colours on the PAUSED bar");
}

#[test]
fn keys_page_lists_default() {
    let pack = build_pack(&["Alpha", "Beta"]);
    let mut ui = Ui::new();
    let mut m = Menu::new(2);
    m.set_view(view::MENU);
    m.event(ev::DOWN);
    m.event(ev::DOWN);
    m.event(ev::DOWN);
    assert_eq!(m.event(ev::OK).0, cmd::NONE);
    assert_eq!(m.view(), view::KEYS);
    ui.draw(&m, Some(&pack));
    let rows = rows_text(&ui);
    let first: Vec<&String> = rows.iter().take(6).collect();
    assert!(first.iter().any(|r| r.contains("F3   KEMPSTON DOWN")), "{first:?}");
    assert!(rows[0].contains("Alpha"));
    // a second page, then back to the first
    m.event(ev::DOWN);
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "F3   KEMPSTON DOWN").is_none());
    m.event(ev::UP);
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "F3   KEMPSTON DOWN").is_some());
    assert_eq!(m.event(ev::BACK).0, cmd::NONE);
    assert_eq!(m.view(), view::MENU);
    m.set_view(view::KEYS);
    assert_eq!(m.event(ev::HOME).0, cmd::NONE);
    assert_eq!(m.view(), view::MENU);
}

#[test]
fn golden_picker_hash() {
    let pack = build_pack(&["Cave Runner", "Night Raid", "Moon Caves"]);
    let mut ui = Ui::new();
    let m = Menu::new(3);
    ui.draw(&m, Some(&pack));
    let mut all = Vec::new();
    for r in 0..240 {
        all.extend_from_slice(&ui.compose_row(r));
    }
    golden("picker_3_games", &format!("{:08x}", fnv1a(&all)));
}

fn draw_and_count(ui: &mut Ui, m: &Menu) -> u32 {
    ui.draw(m, None);
    ui.dirty_rows().0
}

#[test]
fn view_changes_repaint_everything() {
    let mut ui = Ui::new();
    let mut m = Menu::new(1);
    // menu: resume, HOME again
    m.set_view(view::MENU);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    assert_eq!(draw_and_count(&mut ui, &m), 0);
    m.event(ev::OK); // resume
    assert_eq!(m.view(), view::GAME);
    m.event(ev::HOME);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    // paused off and on
    m.event(ev::HOME); // resume
    assert_eq!(m.view(), view::GAME);
    m.event(ev::PAUSE);
    assert_eq!(m.view(), view::PAUSED);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    m.event(ev::PAUSE);
    m.event(ev::PAUSE);
    assert_eq!(m.view(), view::PAUSED);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    // OSK close and reopen
    m.set_view(view::OSK);
    draw_and_count(&mut ui, &m);
    m.event(ev::HOME);
    m.set_view(view::OSK);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    // leave back to the same picker state, and a message
    m.set_view(view::PICKER);
    draw_and_count(&mut ui, &m);
    m.set_view(view::GAME);
    m.set_view(view::PICKER);
    assert_eq!(draw_and_count(&mut ui, &m), 240);
    m.message("Load failed");
    assert_eq!(draw_and_count(&mut ui, &m), 240);
}

#[test]
fn runs_game_only_in_game_and_osk() {
    let mut m = Menu::new(1);
    for (v, runs) in [
        (view::PICKER, false),
        (view::GAME, true),
        (view::PAUSED, false),
        (view::MENU, false),
        (view::KEYS, false),
        (view::OSK, true),
        (view::MESSAGE, false),
    ] {
        m.set_view(v);
        assert_eq!(m.runs_game(), runs, "view {v}");
    }
}

#[test]
fn menu_calls_wait_for_the_ui() {
    // A Menu on another thread (another test) must not invalidate the grid while a Ui is held:
    // its set_view waits until the Ui is dropped.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut ui = Ui::new();
    assert_eq!(ui.dirty_rows().0, 240);
    let other = std::thread::spawn(move || {
        rx.recv().unwrap();
        Menu::new(1).set_view(view::MENU);
    });
    tx.send(()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(ui.dirty_rows().0, 0, "another thread's menu call invalidated the held grid");
    drop(ui);
    other.join().unwrap();
    // the same thread may use a Ui and a Menu together
    let _ui = Ui::new();
    Menu::new(1).set_view(view::MENU);
}

// ---- the UI palette, the flash packer and the menu art's band ----

#[test]
fn ui_pack_row_matches_pix_pack() {
    let mut idx = [0u8; 240];
    for (x, v) in idx.iter_mut().enumerate() {
        *v = ((x * 7 + x / 16) % 16) as u8;
    }
    for fmt in [PixFormat::Rgb444, PixFormat::Rgb565] {
        assert_eq!(zx_sys::ui_pack_row(fmt, &idx), fmt.pack(&idx));
    }
}

#[test]
fn ui_pack_row_rgb444_new_colours() {
    let mut idx = [0u8; 240];
    idx[0] = 16;
    idx[1] = 28;
    let out = zx_sys::ui_pack_row(PixFormat::Rgb444, &idx);
    // pixel 0 = c 9 d, pixel 1 = 1 9 d: bytes R0G0, B0R1, G1B1
    assert_eq!(&out[..3], &[0xC9, 0xD1, 0x9D]);
    assert_eq!(out.len(), 360);
}

#[test]
fn ui_pack_row_rgb565_new_colours() {
    let mut idx = [0u8; 240];
    idx[0] = 16;
    idx[1] = 28;
    let out = zx_sys::ui_pack_row(PixFormat::Rgb565, &idx);
    let c565 = |r: u32, g: u32, b: u32| (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16;
    assert_eq!(u16::from_be_bytes([out[0], out[1]]), c565(0xCC, 0x99, 0xDD));
    assert_eq!(u16::from_be_bytes([out[2], out[3]]), c565(0x11, 0x99, 0xDD));
    assert_eq!(out.len(), 480);
}

#[test]
fn art_band_matches_png() {
    // the band's colours in palette order: black 0, white 15, then 16-24
    let rgb_to_idx = |c: [u8; 3]| -> u8 {
        match c {
            [0, 0, 0] => 0,
            [255, 255, 255] => 15,
            [0xcc, 0x99, 0xdd] => 16,
            [0x88, 0x77, 0xdd] => 17,
            [0x66, 0x66, 0xbb] => 18,
            [0x22, 0x11, 0x66] => 19,
            [0x33, 0x33, 0x66] => 20,
            [0x99, 0x88, 0xdd] => 21,
            [0x88, 0x77, 0xcc] => 22,
            [0x77, 0x66, 0xbb] => 23,
            [0xdd, 0xee, 0xee] => 24,
            c => panic!("unexpected colour {c:02x?}"),
        }
    };
    let file = std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../app/art/menu-top.png")).unwrap();
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (240, 64));
    let ch = info.color_type.samples();
    for y in 0..64usize {
        let want: Vec<u8> = (0..240usize)
            .map(|x| {
                let o = (y * 240 + x) * ch;
                rgb_to_idx([buf[o], buf[o + 1], buf[o + 2]])
            })
            .collect();
        assert_eq!(zx_sys::ui_art_top_row(y as u32).to_vec(), want, "band row {y}");
    }
}

// ---- the menu art on the picker and the HOME menu ----

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("Game {i:02}")).collect()
}

fn composed(ui: &Ui) -> Vec<[u8; 240]> {
    (0..240).map(|r| ui.compose_row(r)).collect()
}

/// The first x of LCD row y where the stripe rule gives k == 0 (None if no stripe starts on the row).
fn stripe_start(y: i32) -> Option<usize> {
    (0..240).find(|&x| 100 * (x as i32 - 171 - 22) - 42 * (239 - y) >= 0)
}

#[test]
fn picker_layout_with_art() {
    let n = names(25);
    let refs: Vec<&str> = n.iter().map(String::as_str).collect();
    let pack = build_pack(&refs);
    let mut ui = Ui::new();
    let m = Menu::new(25);
    ui.draw(&m, Some(&pack));
    assert_eq!(find_row(&ui, "Game 00"), Some(8));
    assert_eq!(find_row(&ui, "Game 19"), Some(27));
    assert!((0..8).all(|r| ui.row_text(r).trim().is_empty()));
    assert_eq!(ui.row_text(29).trim_end(), " 1/25");
    assert!(find_row(&ui, "FM-1 ZX").is_none());
}

#[test]
fn home_layout_with_art() {
    let mut m = open_menu();
    let rows = rows_of(&m);
    assert!(rows[9].contains("Resume"));
    assert!(rows[11].contains("Reset game"));
    m.set_view(view::GAME);
    m.event(ev::HOME);
    to_sound(&mut m);
    assert!(rows_of(&m)[25].contains("Back to games"));
}

#[test]
fn art_on_picker_and_home() {
    let mut ui = Ui::new();
    for v in [view::PICKER, view::MENU] {
        let mut m = Menu::new(3);
        m.set_view(v);
        ui.draw(&m, None);
        let rows = composed(&ui);
        for y in 0..64 {
            assert_eq!(rows[y], zx_sys::ui_art_top_row(y as u32), "view {v} row {y}");
        }
        let x0 = stripe_start(239).unwrap();
        assert_eq!(rows[239][x0], 25);
        assert!(!(25..=28).contains(&rows[239][x0 - 1]));
        // all four stripes appear in order on the last row
        let seen: Vec<u8> = rows[239].iter().copied().filter(|p| (25..=28).contains(p)).fold(vec![], |mut a, p| {
            if a.last() != Some(&p) {
                a.push(p);
            }
            a
        });
        assert_eq!(seen, [25, 26, 27, 28]);
        // no stripe above row 130
        assert!(rows[129].iter().all(|p| !(25..=28).contains(p)));
    }
}

#[test]
fn art_only_on_picker_and_home() {
    let mut ui = Ui::new();
    let mut m = Menu::new(2);
    ui.draw(&m, None); // art on first, so a leak would show
    let check = |ui: &Ui, what: &str| {
        let rows = composed(ui);
        assert!(rows[..64].iter().all(|r| r.iter().all(|&p| p <= 15)), "{what}: band");
        assert!(rows[130..].iter().all(|r| r.iter().all(|&p| !(25..=28).contains(&p))), "{what}: stripes");
    };
    m.set_view(view::KEYS);
    ui.draw(&m, None);
    check(&ui, "keys");
    m.set_view(view::OSK);
    ui.draw(&m, None);
    check(&ui, "osk");
    m.set_view(view::PAUSED);
    ui.draw(&m, None);
    check(&ui, "paused");
    m.message("Load failed");
    ui.draw(&m, None);
    check(&ui, "message");
}

/// The selection bar is drawn over the corner stripes: where they cross it, its paper and ink stay.
#[test]
fn art_selection_over_stripes() {
    let n = names(25);
    let refs: Vec<&str> = n.iter().map(String::as_str).collect();
    let pack = build_pack(&refs);
    let mut ui = Ui::new();
    let mut m = Menu::new(25);
    for _ in 0..19 {
        m.event(ev::DOWN); // the 20th game, on grid row 27
    }
    ui.draw(&m, Some(&pack));
    assert_eq!(find_row(&ui, "Game 19"), Some(27));
    let row = ui.compose_row(220);
    let x0 = stripe_start(220).unwrap();
    assert!(row[x0 - 8..x0].iter().all(|&p| p == 13), "bar paper just left of the stripes (past the text)");
    for (x, &p) in row.iter().enumerate() {
        assert!(p == 13 || p == 0, "bar paper or ink at x={x}: {p}");
    }
    // a row of the same stripes off the bar still has them
    let other = ui.compose_row(212);
    let x1 = stripe_start(212).unwrap();
    assert_eq!(other[x1], 25, "stripes off the bar");
}

#[test]
fn art_with_no_games() {
    let mut ui = Ui::new();
    let m = Menu::new(0);
    ui.draw(&m, None);
    assert_eq!(find_row(&ui, "No games"), Some(14));
    assert_eq!(ui.compose_row(10), zx_sys::ui_art_top_row(10));
    assert!(ui.row_text(29).trim().is_empty());
}

#[test]
fn art_redraw_is_not_dirty_but_view_change_is() {
    let mut ui = Ui::new();
    let mut m = Menu::new(3);
    ui.draw(&m, None);
    ui.dirty_rows();
    ui.draw(&m, None);
    assert_eq!(ui.dirty_rows().0, 0, "the same picker again repaints nothing");
    m.set_view(view::KEYS);
    ui.draw(&m, None);
    assert_eq!(ui.dirty_rows().0, 240, "a view change repaints every LCD row");
}

fn rgb565(rgb: [u8; 3]) -> u16 {
    let (r, g, b) = (rgb[0] as u32, rgb[1] as u32, rgb[2] as u32);
    (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16
}

#[test]
fn band_colours_match_the_ui_palette() {
    // each BAND entry: the palette index packs to the colour the PNG has
    for &(rgb, i) in fmzx_pack::menu_art::BAND.iter() {
        let mut idx = [0u8; 240];
        idx[0] = i;
        let o = zx_sys::ui_pack_row(PixFormat::Rgb565, &idx);
        assert_eq!(u16::from_be_bytes([o[0], o[1]]), rgb565(rgb), "palette index {i} for #{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    }
    // and every pixel of the band as the LCD gets it equals the PNG's pixel
    let file = std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../app/art/menu-top.png")).unwrap();
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let ch = info.color_type.samples();
    for y in 0..64usize {
        let o = zx_sys::ui_pack_row(PixFormat::Rgb565, &zx_sys::ui_art_top_row(y as u32));
        for x in 0..240usize {
            let p = (y * 240 + x) * ch;
            let got = u16::from_be_bytes([o[x * 2], o[x * 2 + 1]]);
            assert_eq!(got, rgb565([buf[p], buf[p + 1], buf[p + 2]]), "band pixel ({x}, {y})");
        }
    }
}

#[test]
fn ui_pack_row_every_new_colour() {
    // the plan's colours 16-28 (#cc99dd, #8877dd, #6666bb, #221166, ...) as 4-bit channels (RGB888 is each nibble * 17)
    let pal: [(u32, u32, u32); 13] = [
        (0xc, 0x9, 0xd), (0x8, 0x7, 0xd), (0x6, 0x6, 0xb), (0x2, 0x1, 0x6), (0x3, 0x3, 0x6), (0x9, 0x8, 0xd), (0x8, 0x7, 0xc),
        (0x7, 0x6, 0xb), (0xd, 0xe, 0xe), (0xe, 0x2, 0x2), (0xf, 0xa, 0x1), (0x5, 0xb, 0x4), (0x1, 0x9, 0xd),
    ];
    for (i, &(r, g, b)) in pal.iter().enumerate() {
        let mut idx = [0u8; 240];
        idx[0] = 16 + i as u8;
        let o = zx_sys::ui_pack_row(PixFormat::Rgb444, &idx);
        assert_eq!((o[0], o[1] >> 4), (((r << 4) | g) as u8, b as u8), "444 colour {}", 16 + i);
        let o = zx_sys::ui_pack_row(PixFormat::Rgb565, &idx);
        let (r8, g8, b8) = (r * 17, g * 17, b * 17);
        let want = (((r8 >> 3) << 11) | ((g8 >> 2) << 5) | (b8 >> 3)) as u16;
        assert_eq!(u16::from_be_bytes([o[0], o[1]]), want, "565 colour {}", 16 + i);
    }
}
