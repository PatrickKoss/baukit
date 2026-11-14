from __future__ import annotations

import http.client
import importlib.util
import shutil
import subprocess
import tempfile
import threading
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from http.server import ThreadingHTTPServer
from pathlib import Path

from process_runner import run_process

SERVER_PATH = Path(__file__).resolve().parent.parent / "deploy/platform/local-git-http.py"
_spec = importlib.util.spec_from_file_location("local_git_http", SERVER_PATH)
if _spec is None or _spec.loader is None:
    raise ImportError(f"cannot load {SERVER_PATH}")
local_git_http = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(local_git_http)


class RequestLengthTest(unittest.TestCase):
    def test_get_without_length_has_no_body(self) -> None:
        self.assertEqual(local_git_http.request_length("GET", None, None), 0)

    def test_valid_length_is_returned(self) -> None:
        self.assertEqual(local_git_http.request_length("POST", "42", None), 42)

    def test_invalid_lengths_are_rejected_with_their_status(self) -> None:
        too_large = str(local_git_http.MAX_REQUEST_BYTES + 1)
        cases = [
            ("POST", None, None, 411),
            ("POST", "-1", None, 400),
            ("POST", "12abc", None, 400),
            ("POST", "", None, 400),
            ("POST", "١", None, 400),
            ("POST", too_large, None, 413),
            ("POST", "5", "chunked", 411),
        ]
        for method, length, encoding, status in cases:
            with self.subTest(length=length, encoding=encoding):
                with self.assertRaises(local_git_http.RequestLengthError) as raised:
                    local_git_http.request_length(method, length, encoding)
                self.assertEqual(raised.exception.status, status)


class QuietHandler(local_git_http.GitHandler):
    def log_message(self, message: str, *args: object) -> None:
        del message, args


class GitServer(ThreadingHTTPServer):
    git_root: Path


@contextmanager
def serve(root: Path) -> Iterator[int]:
    server = GitServer(("127.0.0.1", 0), QuietHandler)
    server.git_root = root
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server.server_address[1]
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


class GitHttpServerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        work = Path(self.directory.name) / "work"
        self.root = Path(self.directory.name) / "git"
        git = shutil.which("git")
        self.assertIsNotNone(git)
        self.git = str(git)
        for argv in (
            [self.git, "init", "-q", "-b", "main", str(work)],
            [
                self.git,
                "-C",
                str(work),
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "snapshot",
            ],
            [self.git, "clone", "-q", "--bare", str(work), str(self.root / "baukit.git")],
        ):
            run_process(argv, check=True, capture_output=True, stdin=subprocess.DEVNULL)

    def post(self, port: int, headers: dict[str, str], body: bytes) -> int:
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
        try:
            connection.putrequest("POST", "/baukit.git/git-upload-pack", skip_accept_encoding=True)
            for name, value in headers.items():
                connection.putheader(name, value)
            connection.endheaders(body)
            return connection.getresponse().status
        finally:
            connection.close()

    def test_clone_works_through_the_server(self) -> None:
        with serve(self.root) as port:
            result = run_process(
                [self.git, "ls-remote", f"http://127.0.0.1:{port}/baukit.git", "HEAD"],
                check=False,
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("HEAD", result.stdout)

    def test_bad_content_length_is_rejected_before_git_runs(self) -> None:
        cases = [
            ({"Content-Length": "abc"}, b"", 400),
            ({"Content-Length": "-5"}, b"", 400),
            ({"Content-Length": str(local_git_http.MAX_REQUEST_BYTES + 1)}, b"", 413),
            ({}, b"", 411),
            ({"Transfer-Encoding": "chunked"}, b"0\r\n\r\n", 411),
        ]
        with serve(self.root) as port:
            for headers, body, status in cases:
                with self.subTest(headers=headers):
                    self.assertEqual(self.post(port, headers, body), status)


if __name__ == "__main__":
    unittest.main()
