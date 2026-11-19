// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 Leo Kuroshita (@kurogedelic), Hügelton Instruments

//! The M-UPGRADE host's side of the USB-MIDI update protocol, against a booted firmware in
//! fm1-emu (`Machine::soc.usb`): a Rust port of felucca-bsp's `tools/fm1_install.py`. Frames are
//! `F0 pack7(00 59 cmd len24 body chk) F7`. Port of the commit
//! 2a5af3dfc8bccb92e37d48a3e7e0bf9a5827e51f, step 1 (the running firmware's reads of the package).

use crate::fw::Machine;

/// The installers' handshake: the device answers its name, '_' and three digits.
pub const HS_QUERY: [u8; 10] = [0xF0, 0x00, 0x32, 0x45, 0x00, 0x00, 0x00, 0x40, 0x7F, 0xF7];
/// The handshake with the one-byte body 01: a firmware on felucca-bsp's `chip/ota.c` answers its
/// whole identity.
pub const HS_FULL: [u8; 12] = [0xF0, 0x00, 0x32, 0x45, 0x08, 0x00, 0x00, 0x40, 0x00, 0x7E, 0x01, 0xF7];

/// The upgrade command that starts the firmware's update session.
pub const UPGRADE: [u8; 6] = [0xF0, 0x22, 0x24, 0x35, 0x7F, 0xF7];
/// The command that restarts the device into its boot loader ("soft uboot").
pub const SOFT_UBOOT: [u8; 6] = [0xF0, 0x22, 0x24, 0x35, 0x7D, 0xF7];
/// The request address that ends step 1 (the package is checked).
pub const FINISH_CHECK: u32 = 0xE000_0000;
/// The most data one reply carries.
pub const MAXDATA: usize = 512;
const BLOCKS: usize = 20;
const BLK: usize = 0x30;
const KEEP: usize = 0x2F;

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

/// The image the device reads during an update: the package without its 20 marker bytes (one after
/// each of the first 20 blocks of 0x30 bytes).
pub fn logical_image(fwsc: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fwsc.len());
    for i in 0..BLOCKS {
        out.extend_from_slice(fwsc.get(i * BLK..i * BLK + KEEP).unwrap_or_default());
    }
    out.extend_from_slice(fwsc.get(BLOCKS * BLK..).unwrap_or_default());
    out
}

/// A read reply: `00 59 30 len24 flags addr32 len24 data chk`, framed.
fn response(addr: u32, data: &[u8], flags: u8) -> Vec<u8> {
    let n = data.len();
    let mut body = vec![0x00, 0x59, 0x30, ((n + 8) & 0xFF) as u8, (((n + 8) >> 8) & 0xFF) as u8, 0, flags];
    body.extend_from_slice(&addr.to_le_bytes());
    body.extend_from_slice(&[(n & 0xFF) as u8, ((n >> 8) & 0xFF) as u8, 0]);
    body.extend_from_slice(data);
    let chk = !body[6..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    body.push(chk);
    let mut msg = vec![0xF0];
    msg.extend(pack7(&body));
    msg.push(0xF7);
    msg
}

/// A read request (`00 59 30`, length 8: flags, address, length): `(flags, addr, len)`.
fn parse_request(msg: &[u8]) -> Option<(u8, u32, u32)> {
    if msg.len() < 2 || msg[0] != 0xF0 || msg[msg.len() - 1] != 0xF7 {
        return None;
    }
    let u = unpack7(&msg[1..msg.len() - 1]);
    if u.len() != 15 || u[..3] != [0x00, 0x59, 0x30] || !u[6..14].iter().fold(0u8, |s, &b| s.wrapping_add(b)) != u[14] {
        return None;
    }
    let addr = u32::from_le_bytes([u[7], u[8], u[9], u[10]]);
    Some((u[6], addr, u32::from_le_bytes([u[11], u[12], u[13], 0])))
}

/// How step 1 ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Serve {
    /// The firmware asked for `FINISH_CHECK` and was answered.
    Success,
    /// The host stopped answering (`ServeOpts::stop_after`).
    Abandoned,
    /// The device asked for something outside the package, or went quiet.
    Refused(String),
}

/// What the host does.
#[derive(Clone, Debug)]
pub struct ServeOpts {
    /// Stop answering after this many requests (the host gone).
    pub stop_after: Option<u32>,
    /// Flip one data byte in this reply (1-based).
    pub corrupt_at: Option<u32>,
    /// Emulated milliseconds without a request that end the session.
    pub max_ms: u64,
    /// MIDI messages (a note-on, then clock bytes) the computer sends every emulated millisecond
    /// while it serves, as a DAW sending to the port would.
    pub midi_noise: u32,
}

/// Sends `UPGRADE` and answers the firmware's read requests from the logical image of `fwsc` until
/// it asks for `FINISH_CHECK`. Step 1 reads parts of the package, each once: a device that asks
/// for more reads than the package has [`MAXDATA`]-byte blocks (and 64) ends it as `Refused`.
pub fn upgrade_step1(m: &mut Machine, fwsc: &[u8], o: &ServeOpts) -> Serve {
    let image = logical_image(fwsc);
    let cap = (image.len() / MAXDATA) as u32 + 64;
    let _ = m.soc.usb.midi_take();
    m.soc.usb.midi_send(&UPGRADE);
    let (mut served, mut asked, mut last) = (0u32, 0u32, m.ms());
    while m.ms() < last + o.max_ms {
        for i in 0..o.midi_noise {
            if i == 0 {
                m.soc.usb.midi_send(&[0x90, 36 + (m.ms() % 40) as u8, 100]);
            } else {
                m.soc.usb.midi_send(&[0xF8]);
            }
        }
        m.run_ms(1);
        for msg in m.soc.usb.midi_take() {
            let Some((fl, addr, n)) = parse_request(&msg) else { continue };
            last = m.ms();
            asked += 1;
            if asked > cap {
                return Serve::Refused(format!("more than {cap} requests"));
            }
            if addr == FINISH_CHECK {
                m.soc.usb.midi_send(&response(addr, b"success\0", 0));
                return Serve::Success;
            }
            let (a, n) = (addr as usize, n as usize);
            if n > MAXDATA || a.checked_add(n).is_none_or(|e| e > image.len()) {
                return Serve::Refused(format!("a read of {addr:#x}+{n}, outside the package"));
            }
            if o.stop_after.is_some_and(|k| served >= k) {
                return Serve::Abandoned;
            }
            served += 1;
            let mut data = image[a..a + n].to_vec();
            if o.corrupt_at == Some(served) && !data.is_empty() {
                data[0] ^= 0xFF;
            }
            m.soc.usb.midi_send(&response(addr, &data, fl));
        }
    }
    Serve::Refused(format!("no request for {} ms", o.max_ms))
}

/// Sends the soft-uboot command.
pub fn soft_uboot(m: &mut Machine) {
    m.soc.usb.midi_send(&SOFT_UBOOT);
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
    fn logical_image_length() {
        let Ok(raw) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../build/fm1/fmzx.fwsc")) else {
            eprintln!("skipped: build/fm1/fmzx.fwsc is absent (tools/make-packages)");
            return;
        };
        let img = logical_image(&raw);
        // what fm1_install.py's logical_image gives for that package: the 20 marker bytes dropped
        assert_eq!(img.len(), 616079);
        assert_eq!(img.len(), raw.len() - BLOCKS);
        assert_eq!(&img[..KEEP], &raw[..KEEP]);
        assert_eq!(&img[KEEP..2 * KEEP], &raw[BLK..BLK + KEEP]);
    }

    #[test]
    fn request_and_response_frames() {
        let body = [0x00, 0x59, 0x30, 8, 0, 0, 0x02, 0x00, 0x10, 0x00, 0x00, 0x00, 0x02, 0x00];
        let mut u = body.to_vec();
        u.push(!u[6..].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
        let mut msg = vec![0xF0];
        msg.extend(pack7(&u));
        msg.push(0xF7);
        assert_eq!(parse_request(&msg), Some((2, 0x1000, 512)));
        let r = response(0x1000, &[1, 2, 3], 2);
        let d = unpack7(&r[1..r.len() - 1]);
        assert_eq!(&d[..7], &[0x00, 0x59, 0x30, 11, 0, 0, 2]);
        assert_eq!(&d[7..11], &0x1000u32.to_le_bytes());
        assert_eq!(&d[11..17], &[3, 0, 0, 1, 2, 3]);
        assert_eq!(d[17], !d[6..17].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
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
