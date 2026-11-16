#!/bin/sh
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Patches the fetched fm1-emu sources (build/fm1-emu; never cargo's checkout) for the browser
# demo. Every edit is exact text that must occur once (web/patch-edit.sh). Run on pristine
# sources (tools/web-demo restores them first):
#   web/patch-emu.sh build/fm1-emu
set -eu

if [ $# -ne 1 ]; then
    echo "usage: web/patch-emu.sh <fm1-emu dir>" >&2
    exit 2
fi
emu=$1
. "$(dirname "$0")/patch-edit.sh"

# The audio PLL as the FM-1's boot leaves it (emu-gate fw.rs AUDIO_PLL_STOCK): on the package
# path the page leaves it unset, and our firmware's audio init refuses to start without it.
f="$emu/crates/fm1-web/src/lib.rs"
edit '        soc.periph.skip_idle = true;
' '        soc.periph.skip_idle = true;
        soc.write32(0x000119A0, 0x45400203);
        soc.write32(0x000119A4, 0x3F503026);
'

# The LCD honours COLMOD (0x3A): with 0x03 the pixel stream is 12-bit, three bytes per two pixels
# (R0G0, B0R1, G1B1: app/pixels.c PAIR), each channel widened by bit replication to the
# framebuffer's RGB565. Any other argument, or none, keeps the two-byte RGB565 path; a COLMOD
# (or a RAMWR) drops a half-assembled pixel.
f="$emu/crates/fm1-soc/src/lcd.rs"
edit '    pixel_hi: Option<u8>,
' '    pixel_hi: Option<u8>,
    colmod: u8,
    pix12: [u8; 2],
    pix12_n: usize,
'
edit '            pixel_hi: None,
' '            pixel_hi: None,
            colmod: 0,
            pix12: [0; 2],
            pix12_n: 0,
'
edit '            0x36 | 0x3A => self.phase = Phase::Arg { need: 1 },' '            0x36 => self.phase = Phase::Arg { need: 1 },
            0x3A => {
                self.phase = Phase::Arg { need: 1 };
                self.colmod = 0;
                self.pixel_hi = None;
                self.pix12_n = 0;
            }'
edit '                self.pixel_hi = None;
            }
            0x21' '                self.pixel_hi = None;
                self.pix12_n = 0;
            }
            0x21'
edit '                    .min(H as u16 - 1);
            }
            _ => {}' '                    .min(H as u16 - 1);
            }
            0x3A => self.colmod = self.arg[0],
            _ => {}'
edit '    fn pixel_byte(&mut self, byte: u8) {
' '    fn pixel_byte(&mut self, byte: u8) {
        if self.colmod == 0x03 {
            if self.pix12_n < 2 {
                self.pix12[self.pix12_n] = byte;
                self.pix12_n += 1;
                return;
            }
            self.pix12_n = 0;
            let [a, b] = self.pix12;
            let widen = |r: u8, g: u8, bl: u8| -> u16 {
                let (r, g, bl) = (r as u16 & 15, g as u16 & 15, bl as u16 & 15);
                ((r << 1 | r >> 3) << 11) | ((g << 2 | g >> 2) << 5) | (bl << 1 | bl >> 3)
            };
            self.push_pixel(widen(a >> 4, a & 15, b >> 4));
            self.push_pixel(widen(b & 15, byte >> 4, byte & 15));
            return;
        }
'

# The running game for the page (web/fmzx.js): a word of RAM, so the worklet can read the
# firmware's fmzx_stats (view, game) at the address tools/web-demo found in the ELF
f="$emu/crates/fm1-web/src/lib.rs"
edit '#[no_mangle]
pub extern "C" fn fm1_enc(' '/// A word of the machine'"'"'s RAM at `addr` (0 outside it).
#[no_mangle]
pub extern "C" fn fm1_peek32(addr: u32) -> u32 {
    machine().map_or(0, |m| {
        let o = addr.wrapping_sub(fm1_soc::RAM_BASE);
        if (o as usize).saturating_add(4) <= m.soc.ram.len() { m.soc.ram.read32(o) } else { 0 }
    })
}

#[no_mangle]
pub extern "C" fn fm1_enc('

# the worklet: the boot message's fmzxStats (the address of fmzx_stats, 0 when unknown), and each
# frame message carries [view, game] read from it (null without it)
f="$emu/web/worklet.js"
edit '    this.seq = m.seq;
' '    this.seq = m.seq;
    this.fmzxStats = (m.fmzxStats >>> 0) || 0;
'
edit 'frames: this.lastTiming?.frames ?? n }, transfer);' 'frames: this.lastTiming?.frames ?? n,
        fmzx: this.fmzxStats && x.fm1_peek32 ? [x.fm1_peek32(this.fmzxStats + 12), x.fm1_peek32(this.fmzxStats + 16)] : null }, transfer);'
