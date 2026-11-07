#!/usr/bin/env python3
"""Run a generated web client's `e2e/stack` specs against Keycloak and its API."""

from __future__ import annotations

import argparse
import http.client
import json
import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path

PLAYWRIGHT_IMAGE = "mcr.microsoft.com/playwright:v1.63.0-noble"
READY_TIMEOUT_SECONDS = 120
STOP_TIMEOUT_SECONDS = 30
WEB_PORT = 5173
OPS_PORT = 9090


def get_status(port: int, path: str) -> int:
    connection = http.client.HTTPConnection("localhost", port, timeout=2)
    try:
        connection.request("GET", path)
        return connection.getresponse().status
    finally:
        connection.close()


def wait_ready(port: int, path: str, process: subprocess.Popen) -> None:
    deadline = time.monotonic() + READY_TIMEOUT_SECONDS
    last_error = "readiness returned a non-200 status"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"{process.args[0]} exited with {process.returncode}")
        try:
            if get_status(port, path) == 200:
                return
        except (OSError, http.client.HTTPException) as error:
            last_error = str(error)
        time.sleep(0.2)
    raise RuntimeError(f"localhost:{port}{path} did not become ready: {last_error}")


def stop(process: subprocess.Popen) -> None:
    process.terminate()
    try:
        process.wait(timeout=STOP_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def backend_environment(name: str) -> dict[str, str]:
    prefix = name.upper().replace("-", "_")
    environment = os.environ.copy()
    environment.update(
        {
            f"{prefix}__DATABASE__URL": (
                f"postgres://postgres:postgres@postgres:5432/{name.replace('-', '_')}"
            ),
            f"{prefix}__RATE_LIMIT__REDIS_URL": "redis://redis:6379/",
            # Clerk and WorkOS fixtures need any admin secret to start; sign-in never uses it.
            f"{prefix}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET": "stack-test-placeholder",
        }
    )
    return environment


def binary_directory(fixture: Path) -> Path:
    target = os.environ.get("CARGO_TARGET_DIR") or fixture / "backend/target"
    return Path(target).resolve() / "debug"


def run_inside(fixture: Path, binaries: Path) -> None:
    environment = backend_environment(fixture.name)
    web = fixture / "web"
    with tempfile.TemporaryFile(mode="w+") as output:
        started: list[subprocess.Popen] = []
        try:
            subprocess.run(
                [str(binaries / "migrate")],
                cwd=fixture,
                env=environment,
                check=True,
            )
            api = subprocess.Popen(
                [str(binaries / "api")],
                cwd=fixture,
                env=environment,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
            started.append(api)
            wait_ready(OPS_PORT, "/readyz", api)
            vite = subprocess.Popen(
                ["node", "node_modules/vite/bin/vite.js", "--port", str(WEB_PORT), "--strictPort"],
                cwd=web,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
            started.append(vite)
            wait_ready(WEB_PORT, "/", vite)
            subprocess.run(
                [
                    "node",
                    "node_modules/@playwright/test/cli.js",
                    "test",
                    "--config",
                    "e2e/playwright.stack.config.ts",
                ],
                cwd=web,
                env={**os.environ, "E2E_WEB_PORT": str(WEB_PORT)},
                check=True,
            )
        except BaseException:
            output.seek(0)
            print(output.read(), flush=True)
            raise
        finally:
            for process in reversed(started):
                stop(process)


def compose_override(repository: Path, fixture: Path, binaries: Path) -> str:
    mounts = [
        f"{repository}:{repository}:ro",
        f"{fixture}:{fixture}",
        f"{binaries}:{binaries}:ro",
    ]
    return f"""services:
  postgres:
    ports: !reset []
  redis:
    ports: !reset []
  keycloak:
    ports: !reset []
    network_mode: service:stack
    depends_on:
      stack:
        condition: service_started
    environment:
      KC_HTTP_PORT: \"8081\"
      JAVA_OPTS_KC_HEAP: \"-Xms128m -Xmx512m\"
      JAVA_OPTS_APPEND: \"-XX:ActiveProcessorCount=2\"
    cpus: 2
    mem_limit: 1g
  stack:
    image: {PLAYWRIGHT_IMAGE}
    command: [sleep, infinity]
    user: \"{os.getuid()}:{os.getgid()}\"
    working_dir: {json.dumps(str(fixture))}
    environment:
      CI: \"true\"
      HOME: /tmp
      PYTHONDONTWRITEBYTECODE: \"1\"
    volumes: {json.dumps(mounts)}
    cpus: 2
    mem_limit: 2g
"""


def run(fixture: Path, repository: Path) -> None:
    binaries = binary_directory(fixture)
    project = f"baukit-web-session-{os.getpid()}"
    with tempfile.TemporaryDirectory(prefix="baukit-web-session-") as temporary:
        override = Path(temporary) / "compose.yaml"
        override.write_text(compose_override(repository, fixture, binaries))
        compose = [
            "docker",
            "compose",
            "--project-name",
            project,
            "--file",
            str(fixture / "compose.yaml"),
            "--file",
            str(override),
        ]
        try:
            subprocess.run(
                [*compose, "up", "--detach", "--wait", "postgres", "keycloak", "redis"], check=True
            )
            subprocess.run(
                [
                    *compose,
                    "exec",
                    "--no-TTY",
                    "stack",
                    "python3",
                    str(Path(__file__).resolve()),
                    str(fixture),
                    "--inside",
                    str(binaries),
                ],
                check=True,
            )
        finally:
            subprocess.run([*compose, "down", "--volumes"], check=True)


def stop_on_signal(signum: int, _frame) -> None:
    raise SystemExit(128 + signum)


def main() -> None:
    signal.signal(signal.SIGTERM, stop_on_signal)
    parser = argparse.ArgumentParser()
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--inside", type=Path, metavar="BINARIES")
    args = parser.parse_args()
    fixture = args.fixture.resolve()
    if args.inside is not None:
        run_inside(fixture, args.inside)
    else:
        run(fixture, Path(__file__).resolve().parents[1])


if __name__ == "__main__":
    main()
