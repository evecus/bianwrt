#!/usr/bin/env python3
"""demo-proxy 插件自带的 HTTP 后端（反向代理模式示例）。

仅监听 127.0.0.1；面板把 /plugins/demo-proxy/<path> 转发到本服务的 /<path>。
静态资源使用相对路径（./bianwrt-sdk.js），浏览器会自动带上 /plugins/demo-proxy/ 前缀。
"""
import argparse
import json
import mimetypes
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "ui"))


class Handler(BaseHTTPRequestHandler):
    server_version = "BianwrtDemoProxy/1.0"

    def log_message(self, fmt, *args):
        print("%s - %s" % (self.address_string(), fmt % args), flush=True)

    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _json(self, obj, code=200):
        self._send(code, json.dumps(obj, ensure_ascii=False).encode("utf-8"),
                   "application/json; charset=utf-8")

    def _static(self, path):
        rel = path.lstrip("/") or "index.html"
        if ".." in rel or rel.startswith("/"):
            return self._send(403, b"forbidden", "text/plain; charset=utf-8")
        fp = os.path.join(ROOT, rel)
        if not os.path.isfile(fp):
            fp = os.path.join(ROOT, "index.html")  # SPA fallback
        if not os.path.isfile(fp):
            return self._send(404, b"not found", "text/plain; charset=utf-8")
        ctype = mimetypes.guess_type(fp)[0] or "application/octet-stream"
        if ctype.startswith("text/") or ctype in ("application/javascript", "application/json"):
            ctype += "; charset=utf-8"
        with open(fp, "rb") as f:
            self._send(200, f.read(), ctype)

    def do_GET(self):
        path = urlparse(self.path).path
        if path == "/api/state":
            return self._json({
                "plugin": "demo-proxy",
                "mode": "proxy",
                "time": time.strftime("%Y-%m-%d %H:%M:%S"),
                "pid": os.getpid(),
                "message": "来自插件自身后端的响应（经面板反向代理）",
            })
        return self._static(path)

    def do_POST(self):
        path = urlparse(self.path).path
        if path == "/api/echo":
            n = int(self.headers.get("Content-Length", "0") or 0)
            raw = self.rfile.read(min(n, 65536)) if n else b"{}"
            try:
                data = json.loads(raw or b"{}")
            except ValueError:
                data = {}
            return self._json({"echo": data, "forwarded_plugin": self.headers.get("X-Bianwrt-Plugin")})
        return self._json({"error": "not found"}, 404)

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Content-Length", "0")
        self.end_headers()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=19090)
    ap.add_argument("--host", default="127.0.0.1")
    a = ap.parse_args()
    srv = ThreadingHTTPServer((a.host, a.port), Handler)
    print("demo-proxy backend listening on %s:%d" % (a.host, a.port), flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
