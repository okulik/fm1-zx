# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The checked text replacement the browser demo's patch scripts use (sourced, not run):
#   f=path/to/file
#   edit 'old text' 'new text'
# replaces the one occurrence of the old text in $f. Any other count (none, or several) fails
# with "patch: <file>: expected 1 match of: <old>, found N" and exit 1, so an fm1-emu revision
# that moves the code breaks the build instead of silently dropping a patch.

edit() {
    python3 -I -c '
import sys
path, old, new = sys.argv[1:4]
with open(path, encoding="utf-8", newline="") as fh:
    text = fh.read()
n = text.count(old)
if n != 1:
    sys.exit(f"patch: {path}: expected 1 match of: {old}, found {n}")
with open(path, "w", encoding="utf-8", newline="") as fh:
    fh.write(text.replace(old, new))
' "$f" "$1" "$2" || exit 1
}
