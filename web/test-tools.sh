#!/bin/sh
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Tests of the browser demo's helpers: web/patch-edit.sh's `edit` and web/serve.py.
#   sh web/test-tools.sh
# Exits non-zero at the first failure.
set -eu

web=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
pid=
trap 'if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

# runs `edit "$1" "$2"` on $tmp/f in a subshell, its output in $tmp/out; prints the exit status
run_edit() {
    set +e
    (f="$tmp/f"; . "$web/patch-edit.sh"; edit "$1" "$2") >"$tmp/out" 2>&1
    s=$?
    set -e
    echo "$s"
}

# (a) one match: replaced
printf 'alpha\nbeta;\ngamma\n' >"$tmp/f"
[ "$(run_edit 'beta;' 'beta;
delta;')" = 0 ] || fail "edit with one match: $(cat "$tmp/out")"
[ "$(cat "$tmp/f")" = "alpha
beta;
delta;
gamma" ] || fail "edit with one match: got $(cat "$tmp/f")"
echo "ok: edit replaces the one match"

# (b) no match: exit 1 and says so
printf 'alpha\n' >"$tmp/f"
[ "$(run_edit 'omega' 'x')" = 1 ] || fail "edit with no match did not exit 1"
grep -q 'expected 1 match' "$tmp/out" || fail "edit with no match printed: $(cat "$tmp/out")"
[ "$(cat "$tmp/f")" = alpha ] || fail "edit with no match changed the file"
echo "ok: edit with no match exits 1"

# (c) two matches: exit 1
printf 'beta\nbeta\n' >"$tmp/f"
[ "$(run_edit 'beta' 'x')" = 1 ] || fail "edit with two matches did not exit 1"
grep -q 'found 2' "$tmp/out" || fail "edit with two matches printed: $(cat "$tmp/out")"
echo "ok: edit with two matches exits 1"

# (d) serve.py: wasm type and no caching
mkdir "$tmp/site"
printf '\0asm' >"$tmp/site/x.wasm"
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])')
python3 "$web/serve.py" "$tmp/site" "$port" >"$tmp/serve.log" 2>&1 &
pid=$!
n=0
until curl -sI "http://127.0.0.1:$port/x.wasm" >"$tmp/head" 2>/dev/null; do
    n=$((n + 1))
    [ "$n" -lt 50 ] || fail "serve.py did not answer: $(cat "$tmp/serve.log")"
    sleep 0.1
done
tr -d '\r' <"$tmp/head" >"$tmp/h"
grep -qi '^Content-Type: application/wasm$' "$tmp/h" || fail "serve.py headers: $(cat "$tmp/h")"
grep -qi '^Cache-Control: no-store$' "$tmp/h" || fail "serve.py headers: $(cat "$tmp/h")"
kill "$pid"
wait "$pid" 2>/dev/null || true
pid=
echo "ok: serve.py serves .wasm as application/wasm with Cache-Control: no-store"

echo "web/test-tools.sh: all passed"
