#!/usr/bin/env python3
"""Serve read-only local Git repositories through git-http-backend."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

MAX_REQUEST_BYTES = 16 * 1024 * 1024


class RequestLengthError(Exception):
    def __init__(self, status: int, reason: str) -> None:
        super().__init__(reason)
        self.status = status
        self.reason = reason


def request_length(method: str, content_length: str | None, transfer_encoding: str | None) -> int:
    if transfer_encoding is not None:
        raise RequestLengthError(411, "Transfer-Encoding is not supported; send Content-Length")
    if content_length is None:
        if method == "POST":
            raise RequestLengthError(411, "Content-Length is required")
        return 0
    if not content_length.isascii() or not content_length.isdigit():
        raise RequestLengthError(400, "Content-Length must be a non-negative integer")
    length = int(content_length)
    if length > MAX_REQUEST_BYTES:
        raise RequestLengthError(413, f"Content-Length exceeds {MAX_REQUEST_BYTES} bytes")
    return length


class GitHandler(BaseHTTPRequestHandler):
    server_version = "baukit-local-git/1"

    def do_GET(self) -> None:
        self._serve()

    def do_POST(self) -> None:
        self._serve()

    def _serve(self) -> None:
        parsed = urlsplit(self.path)
        try:
            length = request_length(
                self.command,
                self.headers.get("Content-Length"),
                self.headers.get("Transfer-Encoding"),
            )
        except RequestLengthError as error:
            self.close_connection = True
            self.send_error(error.status, error.reason)
            return
        body = self.rfile.read(length) if length else b""
        if len(body) != length:
            self.close_connection = True
            self.send_error(400, "request body is shorter than Content-Length")
            return
        environment = os.environ.copy()
        environment.update(
            {
                "GIT_PROJECT_ROOT": str(self.server.git_root),  # type: ignore[attr-defined]  # HTTPServer has no git_root; main sets it for the handler
                "GIT_HTTP_EXPORT_ALL": "1",
                "PATH_INFO": parsed.path,
                "QUERY_STRING": parsed.query,
                "REQUEST_METHOD": self.command,
                "CONTENT_TYPE": self.headers.get("Content-Type", ""),
                "CONTENT_LENGTH": str(length),
                "REMOTE_ADDR": self.client_address[0],
            }
        )
        git = shutil.which("git")
        if git is None:
            self.send_error(500, "git is not on PATH")
            return
        result = subprocess.run(  # noqa: S603 -- argv is the git found by shutil.which plus the fixed "http-backend", no shell
            [git, "http-backend"],
            input=body,
            capture_output=True,
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
    parser.add_argument("--host", required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), GitHandler)
    server.git_root = args.root.resolve()  # type: ignore[attr-defined]  # HTTPServer has no git_root; main sets it for the handler
    server.serve_forever()


if __name__ == "__main__":
    main()
