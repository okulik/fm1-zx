// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use zx_tests::*;

fn drain(s: &mut Sched) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    while let Some(b) = s.next_band() {
        v.push(b);
    }
    v
}

#[test]
fn all_pending_fit() {
    let mut s = Sched::new(PixFormat::Rgb444);
    s.mark_all(1);
    s.begin_frame(1, 150_000);
    let bands = drain(&mut s);
    assert!(bands.iter().all(|&(_, n)| (1..=8).contains(&n)));
    let mut next = 0;
    for &(r, n) in &bands {
        assert_eq!(r, next);
        next += n;
    }
    assert_eq!(next, 240);
    let st = s.stats();
    assert_eq!(st.bands as usize, bands.len());
    assert_eq!(st.bytes as usize, 240 * 360 + bands.len() * 11);
    assert_eq!(s.pending(), 0);
}

#[test]
fn budget_limits_and_carries() {
    let mut s = Sched::new(PixFormat::Rgb444);
    for r in 24..216 {
        s.mark(r, 1);
    }
    s.begin_frame(1, 30_000);
    let bands = drain(&mut s);
    let sent: usize = bands.iter().map(|b| b.1).sum();
    let bytes = s.stats().bytes as usize;
    assert_eq!(bytes, sent * 360 + bands.len() * 11);
    assert!(bytes <= 30_000);
    assert!(30_000 - bytes < 360 + 11, "a further row would still fit");
    // lowest-numbered rows went first
    assert_eq!(bands[0].0, 24);
    let mut next = 24;
    for &(r, n) in &bands {
        assert_eq!(r, next);
        next += n;
    }
    assert_eq!(s.pending(), 192 - sent);
    for r in 24..216 {
        if r < 24 + sent {
            assert_eq!(s.pending_since(r), None);
        } else {
            assert_eq!(s.pending_since(r), Some(1));
        }
    }
}

#[test]
fn oldest_first() {
    let mut s = Sched::new(PixFormat::Rgb444);
    for r in 100..108 {
        s.mark(r, 1);
    }
    s.begin_frame(1, 0);
    assert_eq!(s.next_band(), None);
    for r in 0..8 {
        s.mark(r, 2);
    }
    s.begin_frame(2, 8 * 360 + 11);
    assert_eq!(s.next_band(), Some((100, 8)));
    assert_eq!(s.next_band(), None);
    assert_eq!(s.pending(), 8);
}

#[test]
fn remark_keeps_age() {
    let mut s = Sched::new(PixFormat::Rgb444);
    s.mark(5, 3);
    s.mark(5, 9);
    assert_eq!(s.pending_since(5), Some(3));
    s.begin_frame(9, 150_000);
    assert_eq!(s.next_band(), Some((5, 1)));
    s.mark(5, 10);
    assert_eq!(s.pending_since(5), Some(10));
}

#[test]
fn scroller_steady_fps() {
    let mut s = Sched::new(PixFormat::Rgb444);
    let budget = lcd_budget_bytes(4);
    for frame in 1..=500u32 {
        for r in 24..216 {
            s.mark(r, frame);
        }
        s.begin_frame(frame, budget);
        while s.next_band().is_some() {}
        for r in 24..216 {
            if let Some(since) = s.pending_since(r) {
                assert!(frame - since <= 4, "row {r} pending since {since} at frame {frame}");
            }
        }
    }
    let st = s.stats();
    let fps = 50.0 * st.row_sends as f64 / st.pending_frames_sum as f64;
    assert!((20.0..=23.0).contains(&fps), "fps {fps}");
}

#[test]
fn static_screen_fps() {
    let mut s = Sched::new(PixFormat::Rgb444);
    let mut marks = 0u32;
    for frame in 1..=100u32 {
        if frame % 10 == 0 {
            s.mark(77, frame);
            marks += 1;
        }
        s.begin_frame(frame, 30_000);
        while s.next_band().is_some() {}
    }
    let st = s.stats();
    assert_eq!(st.row_sends, marks);
    assert_eq!(st.pending_frames_sum, marks);
    assert_eq!(st.bytes, marks * (360 + 11));
    assert_eq!(50 * st.row_sends / st.pending_frames_sum, 50);
}

#[test]
fn budget_values() {
    assert_eq!(
        [4, 2, 1, 0].map(lcd_budget_bytes),
        [30_000, 50_000, 75_000, 150_000]
    );
}
