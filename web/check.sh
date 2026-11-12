#!/bin/sh
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs the browser demo's headless smoke test (web/check.mjs) on a built page directory, with the
# preview package as the colour reference:
#   web/check.sh build/web PREVIEW.fwsc
set -eu

if [ $# -ne 2 ]; then
    echo "usage: web/check.sh DIR PREVIEW.fwsc" >&2
    exit 2
fi
if ! command -v node >/dev/null 2>&1; then
    echo "web/check.sh: node not found; the smoke test needs Node.js (https://nodejs.org)" >&2
    exit 1
fi
exec node "$(dirname "$0")/check.mjs" "$1" "$2"
