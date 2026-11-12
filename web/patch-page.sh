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
