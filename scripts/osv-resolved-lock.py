#!/usr/bin/env python3
"""Writes a Cargo.lock holding only the packages cargo can build for a workspace.

Cargo.lock lists optional dependencies that no enabled feature pulls in, so a
scanner reports advisories for code that never compiles. `cargo tree` with all
features and all targets lists what the resolver can actually reach.

usage: osv-resolved-lock.py MANIFEST_PATH OUTPUT_DIRECTORY
"""

from __future__ import annotations

import re
import subprocess
import sys
import tomllib
from pathlib import Path

TREE_LINE = re.compile(r"^(?P<name>[A-Za-z0-9_\-]+) v(?P<version>\S+)")
LOCK_VERSION = 4


def parse_tree_packages(tree: str) -> set[tuple[str, str]]:
    packages = set()
    for line in tree.splitlines():
        match = TREE_LINE.match(line)
        if match:
            packages.add((match["name"], match["version"]))
    return packages


def resolved_packages(manifest: Path) -> set[tuple[str, str]]:
    tree = subprocess.run(
        [
            "cargo",
            "tree",
            "--manifest-path",
            str(manifest),
            "--workspace",
            "--all-features",
            "--target",
            "all",
            "--edges",
            "normal,build,dev",
            "--prefix",
            "none",
            "--locked",
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return parse_tree_packages(tree)


def pruned_lock(lock_text: str, keep: set[tuple[str, str]]) -> str:
    packages = tomllib.loads(lock_text).get("package", [])
    lines = [f"version = {LOCK_VERSION}"]
    for package in packages:
        if (package["name"], package["version"]) not in keep:
            continue
        lines += [
            "",
            "[[package]]",
            f'name = "{package["name"]}"',
            f'version = "{package["version"]}"',
        ]
        if "source" in package:
            lines.append(f'source = "{package["source"]}"')
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    manifest, output = Path(argv[1]), Path(argv[2])
    lock = manifest.with_name("Cargo.lock")
    pruned = pruned_lock(lock.read_text(encoding="utf-8"), resolved_packages(manifest))
    output.mkdir(parents=True, exist_ok=True)
    (output / "Cargo.lock").write_text(pruned, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
