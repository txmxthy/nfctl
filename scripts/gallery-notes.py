#!/usr/bin/env python3
"""Tiny sink for gallery annotations: the page (opened as a file) PUTs its
notes and POSTs exported PNGs here, so they land on disk instead of in the
browser. Stdlib only.

    gallery-notes.py DIR [PORT]     # writes DIR/notes.json and DIR/png/*.png
"""
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

DIR = Path(sys.argv[1] if len(sys.argv) > 1 else "target/gallery")
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8767
NOTES = DIR / "notes.json"
PNG = DIR / "png"


class H(BaseHTTPRequestHandler):
    def _cors(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, PUT, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")

    def _reply(self, code=200, body=b"", ctype="application/json"):
        self.send_response(code)
        self._cors()
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self):
        self._reply(204)

    def do_GET(self):
        if self.path == "/notes.json":
            self._reply(200, NOTES.read_bytes() if NOTES.exists() else b"{}")
        elif self.path == "/build":
            page = DIR / "index.html"
            stamp = str(page.stat().st_mtime_ns) if page.exists() else "0"
            self._reply(200, stamp.encode())
        else:
            self._reply(404, b"{}")

    def _body(self):
        return self.rfile.read(int(self.headers.get("Content-Length", 0)))

    def do_PUT(self):
        if self.path != "/notes.json":
            return self._reply(404, b"{}")
        data = json.loads(self._body() or b"{}")
        NOTES.parent.mkdir(parents=True, exist_ok=True)
        NOTES.write_text(json.dumps(data, indent=1, sort_keys=True))
        self._reply(200, b'{"ok":true}')

    def do_POST(self):
        m = re.fullmatch(r"/png/([A-Za-z0-9._-]+)\.png", self.path)
        if not m:
            return self._reply(404, b"{}")
        PNG.mkdir(parents=True, exist_ok=True)
        (PNG / f"{m.group(1)}.png").write_bytes(self._body())
        self._reply(200, b'{"ok":true}')

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", PORT), H).serve_forever()
