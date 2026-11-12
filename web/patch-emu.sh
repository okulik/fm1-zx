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
