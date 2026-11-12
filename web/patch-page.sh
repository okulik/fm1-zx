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
edit 'let mhz = Number(params.get("mhz") || stored("fm1.mhz") || 100);' 'let mhz = 240;'

# index.html: our stylesheet and script
f="$1/index.html"
edit '</head>' '<link rel="stylesheet" href="fmzx.css">
</head>'
edit '</body>' '<script type="module" src="fmzx.js"></script>
</body>'
