// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

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
    assert!(ui.row_text(0).contains("FM-1 ZX"));
    assert!(find_row(&ui, "Game 00").is_some());
    assert!(find_row(&ui, "Game 23").is_some());
    assert!(find_row(&ui, "Game 24").is_none());
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
    for item in ["Resume", "Reset game", "Keyboard", "Key map", "LCD speed: 4", "Back to games"] {
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
    assert!(first.iter().any(|r| r.contains("F3   KEMPSTON LEFT")), "{first:?}");
    assert!(rows[0].contains("Alpha"));
    // a second page, then back to the first
    m.event(ev::DOWN);
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "F3   KEMPSTON LEFT").is_none());
    m.event(ev::UP);
    ui.draw(&m, Some(&pack));
    assert!(find_row(&ui, "F3   KEMPSTON LEFT").is_some());
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
