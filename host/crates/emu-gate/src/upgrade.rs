// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 Leo Kuroshita (@kurogedelic), Hügelton Instruments

//! The M-UPGRADE host's side of the USB-MIDI update protocol, against a booted firmware in
//! fm1-emu (`Machine::soc.usb`): a Rust port of felucca-bsp's `tools/fm1_install.py`. Frames are
//! `F0 pack7(00 59 cmd len24 body chk) F7`.

use crate::fw::Machine;

/// The installers' handshake: the device answers its name, '_' and three digits.
pub const HS_QUERY: [u8; 10] = [0xF0, 0x00, 0x32, 0x45, 0x00, 0x00, 0x00, 0x40, 0x7F, 0xF7];
/// The handshake with the one-byte body 01: a firmware on felucca-bsp's `chip/ota.c` answers its
/// whole identity.
pub const HS_FULL: [u8; 12] = [0xF0, 0x00, 0x32, 0x45, 0x08, 0x00, 0x00, 0x40, 0x00, 0x7E, 0x01, 0xF7];

/// 8-to-7 bit packing, LSB first.
pub fn pack7(d: &[u8]) -> Vec<u8> {
    let (mut out, mut acc, mut nb) = (Vec::with_capacity(d.len() * 8 / 7 + 1), 0u32, 0u32);
    for &b in d {
        acc |= u32::from(b) << nb;
        nb += 8;
        while nb >= 7 {
            out.push((acc & 0x7F) as u8);
            acc >>= 7;
            nb -= 7;
        }
    }
    if nb > 0 {
        out.push((acc & 0x7F) as u8);
    }
    out
}

/// The inverse of [`pack7`] (a partial last byte dropped).
pub fn unpack7(d: &[u8]) -> Vec<u8> {
    let (mut out, mut acc, mut nb) = (Vec::with_capacity(d.len() * 7 / 8), 0u32, 0u32);
    for &b in d {
        acc |= u32::from(b & 0x7F) << nb;
        nb += 7;
        while nb >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            nb -= 8;
        }
    }
    out
}

/// A handshake's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The identity text, up to its first zero.
    pub text: String,
    /// The flash's JEDEC ID (bytes 24..26 of the identity field); `None` when all zero (not read).
    pub jedec: Option<[u8; 3]>,
}

/// An identity reply (`00 59 11`, length 27, the identity field, its checksum), or `None`.
fn parse_identity(msg: &[u8]) -> Option<Identity> {
    if msg.len() < 2 || msg[0] != 0xF0 || msg[msg.len() - 1] != 0xF7 {
        return None;
    }
    let d = unpack7(&msg[1..msg.len() - 1]);
    if d.len() != 34 || d[..6] != [0x00, 0x59, 0x11, 27, 0, 0] {
        return None;
    }
    let field = &d[6..33];
    if !field.iter().fold(0u8, |s, &b| s.wrapping_add(b)) != d[33] {
        return None;
    }
    let text = field.split(|&b| b == 0).next().unwrap_or_default();
    let jedec = [field[24], field[25], field[26]];
    Some(Identity { text: String::from_utf8_lossy(text).into_owned(), jedec: (jedec != [0; 3]).then_some(jedec) })
}

/// Sends the handshake (`full`: [`HS_FULL`], else [`HS_QUERY`]) and runs the machine until the
/// identity reply comes back, for at most `max_ms` emulated milliseconds.
pub fn handshake(m: &mut Machine, full: bool, max_ms: u64) -> Option<Identity> {
    let _ = m.soc.usb.midi_take();
    m.soc.usb.midi_send(if full { &HS_FULL[..] } else { &HS_QUERY[..] });
    let end = m.ms() + max_ms;
    while m.ms() < end {
        m.run_ms(1);
        if let Some(id) = m.soc.usb.midi_take().iter().find_map(|msg| parse_identity(msg)) {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack7_round_trip() {
        let d: Vec<u8> = (0..=255u8).cycle().take(700).collect();
        for n in [0, 1, 6, 7, 8, 27, 34, 640, 700] {
            let p = pack7(&d[..n]);
            assert!(p.iter().all(|&b| b < 0x80));
            assert_eq!(unpack7(&p), &d[..n], "length {n}");
        }
    }

    /// The reply `ota_reply_identity` sends, with `edit` applied to its decoded bytes.
    fn reply(edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut d = vec![0x00, 0x59, 0x11, 27, 0, 0];
        let mut field = [0u8; 27];
        field[..8].copy_from_slice(b"FM-1_048");
        field[24..].copy_from_slice(&[0x85, 0x60, 0x14]);
        d.extend_from_slice(&field);
        d.push(!field.iter().fold(0u8, |s, &b| s.wrapping_add(b)));
        edit(&mut d);
        let mut msg = vec![0xF0];
        msg.extend(pack7(&d));
        msg.push(0xF7);
        msg
    }

    #[test]
    fn identity_reply_parses() {
        assert_eq!(parse_identity(&reply(|_| {})), Some(Identity { text: "FM-1_048".into(), jedec: Some([0x85, 0x60, 0x14]) }));
    }

    #[test]
    fn identity_reply_checked() {
        assert_eq!(parse_identity(&reply(|d| d[33] ^= 1)), None, "a wrong checksum");
        assert_eq!(parse_identity(&reply(|d| d[3] = 26)), None, "a wrong length");
    }
}
