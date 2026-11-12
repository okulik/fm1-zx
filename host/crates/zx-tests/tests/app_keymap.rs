// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

const ENC0: usize = 41;

fn note(n: u32) -> u32 {
    1 << n
}

#[test]
fn default_kempston() {
    let mut km = KeyMapper::new();
    let (rows, k) = km.apply(&KmInput { notes: note(0) | note(3), ..Default::default() });
    assert_eq!(k, 0b1010);
    assert_eq!(rows, [0; 8]);
}

#[test]
fn default_enter_digits() {
    let mut km = KeyMapper::new();
    // C4 = note 7, OCT- = button 0
    let (rows, k) = km.apply(&KmInput { notes: note(7), buttons: 1, ..Default::default() });
    assert_eq!(k, 0);
    let mut want = [0u8; 8];
    want[6] = 1; // ENTER
    want[3] = 1; // 1
    assert_eq!(rows, want);
}

#[test]
fn two_actions() {
    let mut km = KeyMapper::new();
    let mut map = km_default();
    map[2] = [0, 20]; // FX: CAPS SHIFT (row 0 bit 0) + 0 (row 4 bit 0)
    km.set_map(&map);
    let (rows, _) = km.apply(&KmInput { buttons: 1 << 2, ..Default::default() });
    let mut want = [0u8; 8];
    want[0] = 1;
    want[4] = 1;
    assert_eq!(rows, want);
}

fn pulse_map(km: &mut KeyMapper) {
    let mut map = [[KM_NONE; 2]; KM_INPUTS];
    map[ENC0 + 1] = [5 * 5 + 0, KM_NONE]; // SELECT cw -> P (row 5 bit 0)
    km.set_map(&map);
}

fn p_calls(km: &mut KeyMapper, first: KmInput) -> usize {
    let mut n = 0;
    let mut inp = first;
    for _ in 0..30 {
        let (rows, _) = km.apply(&inp);
        inp = KmInput::default();
        if rows[5] & 1 != 0 {
            n += 1;
        } else if n > 0 {
            break;
        }
    }
    n
}

#[test]
fn encoder_pulse() {
    let mut km = KeyMapper::new();
    pulse_map(&mut km);
    let mut e = [0i8; 7];
    e[0] = 1;
    assert_eq!(p_calls(&mut km, KmInput { enc: e, ..Default::default() }), KM_PULSE_FRAMES as usize);
    e[0] = 3;
    assert_eq!(p_calls(&mut km, KmInput { enc: e, ..Default::default() }), 9);
    // the ccw direction is not mapped
    e[0] = -1;
    assert_eq!(p_calls(&mut km, KmInput { enc: e, ..Default::default() }), 0);
}

#[test]
fn release_all() {
    let mut km = KeyMapper::new();
    km.apply(&KmInput { buttons: 0x3FFF, notes: 0x7FF_FFFF, enc: [1; 7] });
    km.reset();
    let (rows, k) = km.apply(&KmInput::default());
    assert_eq!(rows, [0; 8]);
    assert_eq!(k, 0);
}

#[test]
fn home_play_ignored() {
    let mut km = KeyMapper::new();
    let mut map = [[0u8; 2]; KM_INPUTS]; // everything maps to key 0 (CAPS SHIFT), even HOME and PLAY
    map[8] = [1, 0x44];
    map[12] = [1, 0x44];
    map[0] = [KM_NONE; 2];
    km.set_map(&map);
    let (rows, k) = km.apply(&KmInput { buttons: (1 << 8) | (1 << 12), ..Default::default() });
    assert_eq!(rows, [0; 8]);
    assert_eq!(k, 0);
}
