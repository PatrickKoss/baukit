#!/usr/bin/env python3
"""Run a generated backend against its compose services and an rmcp client."""

from __future__ import annotations

import argparse
import base64
import http.cookiejar
import importlib.util
import json
import os
import signal
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def wait_ready(url: str, process: subprocess.Popen) -> None:
    deadline = time.monotonic() + 60
    last_error = "readiness returned a non-200 status"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"backend exited with {process.returncode}")
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if response.status == 200:
                    return
        except (urllib.error.URLError, TimeoutError) as error:
            last_error = str(error)
        time.sleep(0.2)
    raise RuntimeError(f"backend did not become ready: {last_error}")


def run_backend(fixture: Path, repository: Path) -> None:
    name = fixture.name
    prefix = name.upper().replace("-", "_")
    environment = os.environ.copy()
    environment[f"{prefix}__DATABASE__URL"] = f"postgres://postgres:postgres@postgres:5432/{name.replace('-', '_')}"
    environment[f"{prefix}__RATE_LIMIT__REDIS_URL"] = "redis://redis:6379/"
    backend = None
    with tempfile.TemporaryFile(mode="w+") as output:
        try:
            subprocess.run([str(fixture / "backend/target/debug/migrate")], env=environment, check=True)
            resource = "http://localhost:8080/mcp"
            environment.update({
                f"{prefix}__MCP__ENABLED": "true",
                f"{prefix}__MCP__RESOURCE_URL": resource,
                f"{prefix}__MCP__ISSUER": f"http://localhost:8081/realms/{name}",
                f"{prefix}__MCP__ALLOWED_HOSTS": '["localhost:8080"]',
                f"{prefix}__MCP__ALLOWED_ORIGINS": "[]",
            })
            backend = subprocess.Popen([str(fixture / "backend/target/debug/api")], env=environment, stdout=output, stderr=subprocess.STDOUT)
            wait_ready("http://localhost:9090/readyz", backend)
            module_path = fixture / "scripts/pkce-login.py"
            spec = importlib.util.spec_from_file_location("mcp_pkce", module_path)
            if spec is None or spec.loader is None:
                raise RuntimeError("cannot load generated PKCE helper")
            pkce = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(pkce)
            issuer = f"http://localhost:8081/realms/{name}"
            opener = urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar(policy=pkce.LocalDevelopmentCookiePolicy())),
                pkce.CallbackRedirectHandler("http://localhost:18888/callback"),
            )
            token = pkce.login(opener, pkce.discover(opener, issuer), username="test", password="development-password", client_id=f"{name}-mcp", redirect_uri="http://localhost:18888/callback", scope="openid items:read", resource=resource)
            payload = token.split(".")[1]
            claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
            audience = claims.get("aud", [])
            if isinstance(audience, str):
                audience = [audience]
            if claims.get("iss") != issuer or resource not in audience or not claims.get("sub"):
                raise RuntimeError("issued token lacks the expected issuer, resource audience, or subject")
            if "items:read" not in claims.get("scope", "").split():
                raise RuntimeError("issued token lacks items:read")
            web_opener = urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar(policy=pkce.LocalDevelopmentCookiePolicy())),
                pkce.CallbackRedirectHandler("http://localhost:5173/auth/callback"),
            )
            web_token = pkce.login(web_opener, pkce.discover(web_opener, issuer), username="test", password="development-password", client_id=f"{name}-web", redirect_uri="http://localhost:5173/auth/callback", scope="openid profile email offline_access")
            item_name = "MCP smoke item"
            request = urllib.request.Request(
                "http://localhost:8080/items",
                data=json.dumps({"name": item_name}).encode(),
                headers={"Content-Type": "application/json", "Authorization": f"Bearer {web_token}"},
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=5) as response:
                item = json.load(response)
                if response.status != 201 or item["name"] != item_name:
                    raise RuntimeError("REST did not create the smoke item")
            print("REST: created MCP smoke item with a backend-audience web token", flush=True)
            print("OAuth: authorization code + PKCE S256, scope items:read, resource http://localhost:8080/mcp", flush=True)
            with urllib.request.urlopen("http://localhost:8080/.well-known/oauth-protected-resource", timeout=5) as response:
                metadata = json.load(response)
            if metadata["resource"] != resource or metadata["authorization_servers"] != [issuer]:
                raise RuntimeError(f"incorrect resource metadata: {metadata}")
            print("metadata: " + json.dumps(metadata, sort_keys=True), flush=True)
            client_environment = os.environ.copy()
            client_environment.update({"MCP_RESOURCE_URL": resource, "MCP_ACCESS_TOKEN": token, "MCP_EXPECT_ITEM_NAME": item_name})
            subprocess.run([str(repository / "rust/target/debug/examples/connect")], env=client_environment, check=True)
        except BaseException:
            output.seek(0)
            print(output.read())
            raise
        finally:
            if backend is not None:
                backend.terminate()
                try:
                    backend.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    backend.kill()
                    backend.wait()


def run(fixture: Path, repository: Path) -> None:
    subprocess.run(["cargo", "build", "--manifest-path", str(repository / "rust/Cargo.toml"), "-p", "baukit-mcp", "--example", "connect"], check=True)
    project = f"baukit-mcp-smoke-{os.getpid()}"
    with tempfile.TemporaryDirectory(prefix="baukit-mcp-compose-") as temporary:
        override = Path(temporary) / "compose.yaml"
        mounts = [f"{repository}:{repository}:ro", f"{fixture}:{fixture}:ro"]
        override.write_text(f"""services:
  postgres:
    ports: !reset []
  redis:
    ports: !reset []
  keycloak:
    ports: !reset []
    network_mode: service:smoke
    depends_on:
      smoke:
        condition: service_started
    environment:
      KC_HTTP_PORT: \"8081\"
      JAVA_OPTS_KC_HEAP: \"-Xms128m -Xmx512m\"
      JAVA_OPTS_APPEND: \"-XX:ActiveProcessorCount=2\"
    cpus: 2
    mem_limit: 1g
  smoke:
    image: mcr.microsoft.com/playwright:v1.63.0-noble
    command: [sleep, infinity]
    user: \"{os.getuid()}:{os.getgid()}\"
    working_dir: {json.dumps(str(repository))}
    environment:
      PYTHONDONTWRITEBYTECODE: \"1\"
    volumes: {json.dumps(mounts)}
    cpus: 2
    mem_limit: 1g
""")
        compose = ["docker", "compose", "--project-name", project, "--file", str(fixture / "compose.yaml"), "--file", str(override)]
        try:
            subprocess.run([*compose, "up", "--detach", "--wait", "postgres", "keycloak", "redis"], check=True)
            subprocess.run([*compose, "exec", "--no-TTY", "smoke", "python3", str(Path(__file__).resolve()), str(fixture), "--inside"], check=True)
        finally:
            subprocess.run([*compose, "down", "--volumes"], check=True)


def stop_on_signal(signum: int, _frame) -> None:
    raise SystemExit(128 + signum)


def main() -> None:
    signal.signal(signal.SIGTERM, stop_on_signal)
    parser = argparse.ArgumentParser()
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--inside", action="store_true")
    args = parser.parse_args()
    runner = run_backend if args.inside else run
    runner(args.fixture.resolve(), Path(__file__).resolve().parents[1])


if __name__ == "__main__":
    main()
