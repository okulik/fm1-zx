#!/bin/sh
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Patches the built page (main.js, index.html in the build directory) for the browser demo.
# Every edit is exact text that must occur once (web/patch-edit.sh). tools/web-demo runs it
# after copying our files over upstream's build:
#   web/patch-page.sh build/web
set -eu

if [ $# -ne 1 ]; then
    echo "usage: web/patch-page.sh <build dir>" >&2
    exit 2
fi
. "$(dirname "$0")/patch-edit.sh"

# main.js: the input hooks for our panel (web/fmzx.js), the fresh package, the one clock
f="$1/main.js"
edit 'function setPot(v) {' 'window.fmzxInput = { setKey, turn };
function setPot(v) {'
edit '  if (e.metaKey || e.ctrlKey || e.target instanceof HTMLSelectElement) return;
' '  if (e.metaKey || e.ctrlKey || e.target instanceof HTMLSelectElement) return;
  if (window.fmzxKey?.(e, true)) return;
'
edit 'addEventListener("keyup", (e) => {
' 'addEventListener("keyup", (e) => {
  if (window.fmzxKey?.(e, false)) return;
'
edit 'const kept = await keptEntry(f.id);' 'const kept = f.fresh ? null : await keptEntry(f.id);'
# the draw loop read the memory buffer before asking for the panel pointer, and the call can grow
# (detach) the memory: take the pointer first
edit '  const mem = () => ui.memory.buffer;
' '  const mem = () => ui.memory.buffer;
  const view = (T, ptr, n) => new T(mem(), ptr, n);
'
edit 'new Uint16Array(mem(), ui.panel_lcd(), 240 * 240).set(frame.lcd);' 'view(Uint16Array, ui.panel_lcd(), 240 * 240).set(frame.lcd);'
edit 'new Float32Array(mem(), ui.panel_leds(), 41).set(frame.leds);' 'view(Float32Array, ui.panel_leds(), 41).set(frame.leds);'
# no SMK-37 catalog next to this page (we drop smk37/): skip the twin lookup
edit '    for (const s of (await (await fetch("smk37/firmwares.json")).json()).firmwares) if (s.fm1) twins.set(s.fm1, s.id);
' ''
# the fresh package is never saved either: no snapshot of it goes to the IndexedDB store
edit 'persistedSectors: record?.sectors || [], entry: DEV.key + f.id,' 'fresh: !!f.fresh, persistedSectors: record?.sectors || [], entry: DEV.key + f.id,'
edit 'function saveSnapshot(session, flash, flashRevision) {
' 'function saveSnapshot(session, flash, flashRevision) {
  if (session.fresh) return Promise.resolve();
'
edit 'let mhz = Number(params.get("mhz") || stored("fm1.mhz") || 100);' 'let mhz = 240;'

# index.html: our stylesheet and script
f="$1/index.html"
edit '</head>' '<link rel="stylesheet" href="fmzx.css">
</head>'
edit '</body>' '<script type="module" src="fmzx.js"></script>
</body>'
