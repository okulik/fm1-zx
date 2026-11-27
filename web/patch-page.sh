#!/bin/sh
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Patches the built page (main.js, index.html in the build directory) for the browser demo.
# Every edit is exact text that must occur once (web/patch-edit.sh). tools/web-demo runs it
# after copying our files over upstream's build:
#   web/patch-page.sh build/web [build name]
set -eu

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: web/patch-page.sh <build dir> [build name]" >&2
    exit 2
fi
v=$(printf %s "${2:-0}" | tr -c 'A-Za-z0-9._-' '-')
. "$(dirname "$0")/patch-edit.sh"

# main.js: the input hooks for our panel (web/fmzx.js), the fresh package, the one clock
f="$1/main.js"
edit 'function setPot(v) {' 'window.fmzxInput = { setKey, turn, hit: (x, y) => ui.panel_hit(x, y), board: () => board };
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
# upstream's draw loop reads the memory buffer before asking for the panel pointer, and that call
# can grow (detach) the memory: take the pointer first
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
# the running game (web/fmzx.js): the address of the firmware's fmzx_stats goes to the worklet at
# boot, each frame (with [view, game] read from it) goes to our layer, and our labels are drawn
# over the synth each time it is drawn
edit 'mhz: clock, jit: params.get("jit") !== "0",' 'fmzxStats: window.fmzxStats || 0, mhz: clock, jit: params.get("jit") !== "0",'
edit '  frame = m;
' '  frame = m;
  window.fmzxFrame?.(m);
'
edit '  ctx.putImageData(img, 0, 0);
' '  ctx.putImageData(img, 0, 0);
  window.fmzxDraw?.(ctx, W, H);
'
edit 'let mhz = Number(params.get("mhz") || stored("fm1.mhz") || 100);' 'let mhz = 240;'
# the catalog and the package are asked of the server each time (it answers "not changed" when
# they are the ones the browser holds), so a page published again never runs a package it kept
edit 'await fetch("firmwares.json")' 'await fetch("firmwares.json", { cache: "no-cache" })'
edit 'const got = await fetchPackage(f);' 'const got = await fetchPackage(f, (u) => fetch(u, { cache: "no-cache" }));'
edit 'const got = await fetchPackage(selected);' 'const got = await fetchPackage(selected, (u) => fetch(u, { cache: "no-cache" }));'

# index.html: our stylesheet and script, named with the build ($2) so a browser that holds an
# earlier build's files asks for these
f="$1/index.html"
edit '</head>' "<link rel=\"stylesheet\" href=\"fmzx.css?v=$v\">
</head>"
edit '</body>' "<script type=\"module\" src=\"fmzx.js?v=$v\"></script>
</body>"
