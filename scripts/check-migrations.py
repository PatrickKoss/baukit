#!/usr/bin/env python3
"""Check baukit's reference migrations against rust/migrations.toml.

order         every crate with migrations is listed once, after the crates it names
lint          squawk over every migration that is not in the shipped baseline
double-apply  apply all migrations in order on a fresh Postgres, then again: new files must
              rerun cleanly and leave the schema unchanged
serve         apply all migrations, print the DATABASE_URL and keep the container until stdin closes
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
import tomllib
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "rust" / "migrations.toml"
NO_TRANSACTION = "-- no-transaction"
PG_VERSION = "18"
READY_ATTEMPTS = 60
READY_POLL_SECONDS = 1.0
CONTAINER_LABEL = "baukit.check-migrations=1"


@dataclass(frozen=True)
class Plan:
    crates: list[dict]
    shipped: frozenset[str]
    image: str

    def files(self) -> list[Path]:
        ordered = []
        for crate in self.crates:
            ordered.extend(
                sorted((ROOT / "rust" / "crates" / crate["name"] / "migrations").glob("*.sql"))
            )
        return ordered

    def is_shipped(self, path: Path) -> bool:
        return path.relative_to(ROOT).as_posix() in self.shipped


def load_plan(manifest: Path = MANIFEST) -> Plan:
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    return Plan(data["crate"], frozenset(data["shipped"]["files"]), data["postgres_image"])


def order_errors(plan: Plan, crates_dir: Path) -> list[str]:
    names = [crate["name"] for crate in plan.crates]
    errors = [f"{name} is listed twice" for name in set(names) if names.count(name) > 1]
    on_disk = {path.parent.parent.name for path in crates_dir.glob("*/migrations/*.sql")}
    errors += [f"{name} has migrations but is not listed" for name in sorted(on_disk - set(names))]
    errors += [f"{name} is listed but has no migrations" for name in sorted(set(names) - on_disk)]
    for index, crate in enumerate(plan.crates):
        for parent in crate.get("after", []):
            if parent not in names[:index]:
                errors.append(f"{crate['name']} must come after {parent}")
    missing = [file for file in sorted(plan.shipped) if not (ROOT / file).is_file()]
    errors += [f"shipped baseline names a missing file: {file}" for file in missing]
    return errors


def lint(plan: Plan) -> int:
    new_files = [str(path.relative_to(ROOT)) for path in plan.files() if not plan.is_shipped(path)]
    if not new_files:
        print("check-migrations: no migrations newer than the shipped baseline")
        return 0
    command = ["squawk", f"--pg-version={PG_VERSION}", *new_files]
    return subprocess.run(command, cwd=ROOT, check=False).returncode  # noqa: S603 -- argv list from a checked-in manifest, no shell


def docker(*args: str, stdin: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(  # noqa: S603 -- fixed docker argv, no shell
        ["docker", *args],  # noqa: S607 -- docker comes from PATH like every gate tool
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    )


@contextmanager
def postgres(image: str) -> Iterator[str]:
    started = docker(
        "run",
        "-d",
        "--rm",
        "--label",
        CONTAINER_LABEL,
        "-e",
        "POSTGRES_HOST_AUTH_METHOD=trust",
        "-p",
        "127.0.0.1::5432",
        image,
    )
    if started.returncode != 0:
        raise SystemExit(f"check-migrations: docker run failed: {started.stderr}")
    container = started.stdout.strip()
    try:
        for _ in range(READY_ATTEMPTS):
            if (
                docker(
                    "exec", container, "pg_isready", "-U", "postgres", "-h", "127.0.0.1"
                ).returncode
                == 0
            ):
                break
            time.sleep(READY_POLL_SECONDS)
        else:
            raise SystemExit("check-migrations: postgres did not become ready")
        yield container
    finally:
        docker("rm", "-f", container)


def apply(container: str, path: Path) -> subprocess.CompletedProcess[str]:
    sql = path.read_text(encoding="utf-8")
    single = [] if sql.lstrip().startswith(NO_TRANSACTION) else ["--single-transaction"]
    return docker(
        "exec",
        "-i",
        container,
        "psql",
        "-U",
        "postgres",
        "-q",
        "-v",
        "ON_ERROR_STOP=1",
        *single,
        "-f",
        "-",
        stdin=sql,
    )


def schema(container: str) -> str:
    return docker("exec", container, "pg_dump", "-U", "postgres", "--schema-only").stdout


def apply_all(container: str, files: list[Path]) -> list[str]:
    errors = []
    for path in files:
        result = apply(container, path)
        if result.returncode != 0:
            errors.append(f"{path.relative_to(ROOT)} failed on a fresh database:\n{result.stderr}")
    return errors


def double_apply(plan: Plan) -> int:
    files = plan.files()
    with postgres(plan.image) as container:
        errors = apply_all(container, files)
        before = schema(container)
        for path in files:
            result = apply(container, path)
            if result.returncode == 0 or plan.is_shipped(path):
                continue
            errors.append(f"{path.relative_to(ROOT)} is not re-runnable:\n{result.stderr}")
        if schema(container) != before:
            errors.append("the second run changed the schema")
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(f"check-migrations: applied {len(files)} migrations twice")
    return 1 if errors else 0


def serve(plan: Plan) -> int:
    with postgres(plan.image) as container:
        errors = apply_all(container, plan.files())
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        port = docker("port", container, "5432/tcp").stdout.split(":")[-1].strip()
        print(f"postgres://postgres@127.0.0.1:{port}/postgres", flush=True)
        sys.stdin.read()
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("command", choices=["order", "lint", "double-apply", "serve"])
    args = parser.parse_args(argv)
    plan = load_plan()
    errors = order_errors(plan, ROOT / "rust" / "crates")
    if errors:
        print("\n".join(f"ERROR: {error}" for error in errors), file=sys.stderr)
        return 1
    commands = {"order": lambda _: 0, "lint": lint, "double-apply": double_apply, "serve": serve}
    return commands[args.command](plan)


if __name__ == "__main__":
    sys.exit(main())
