# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The browser demo's local server: serves DIR on 127.0.0.1:PORT only, with
# Cache-Control: no-store on every response (a reload after a rebuild gets the new files) and
# the module and wasm types the page needs.
#   python3 web/serve.py build/web 8642
import functools
import http.server
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".mjs": "text/javascript",
        ".js": "text/javascript",
    }

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: serve.py DIR PORT")
    directory, port = sys.argv[1], int(sys.argv[2])
    handler = functools.partial(Handler, directory=directory)
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as server:
        print(f"serving {directory} at http://127.0.0.1:{port}/ (Ctrl-C stops)", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
