#!/usr/bin/env python3
"""Serve read-only local Git repositories through git-http-backend."""

from __future__ import annotations

import argparse
import os
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


class GitHandler(BaseHTTPRequestHandler):
    server_version = "baukit-local-git/1"

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        self._serve()

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        self._serve()

    def _serve(self) -> None:
        parsed = urlsplit(self.path)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b""
        environment = os.environ.copy()
        environment.update(
            {
                "GIT_PROJECT_ROOT": str(self.server.git_root),  # type: ignore[attr-defined]
                "GIT_HTTP_EXPORT_ALL": "1",
                "PATH_INFO": parsed.path,
                "QUERY_STRING": parsed.query,
                "REQUEST_METHOD": self.command,
                "CONTENT_TYPE": self.headers.get("Content-Type", ""),
                "CONTENT_LENGTH": str(length),
                "REMOTE_ADDR": self.client_address[0],
            }
        )
        result = subprocess.run(
            ["git", "http-backend"],
            input=body,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )
        header_block, separator, response_body = result.stdout.partition(b"\r\n\r\n")
        if not separator:
            self.send_error(500, result.stderr.decode(errors="replace"))
            return
        status = 200
        headers: list[tuple[str, str]] = []
        for raw_header in header_block.decode("latin-1").split("\r\n"):
            name, _, value = raw_header.partition(":")
            if name.lower() == "status":
                status = int(value.strip().split(" ", 1)[0])
            elif name:
                headers.append((name, value.strip()))
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(response_body)

    def log_message(self, message: str, *args: object) -> None:
        print(f"{self.address_string()} - {message % args}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("0.0.0.0", args.port), GitHandler)
    server.git_root = args.root.resolve()  # type: ignore[attr-defined]
    server.serve_forever()


if __name__ == "__main__":
    main()

