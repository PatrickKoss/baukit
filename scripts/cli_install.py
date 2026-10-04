#!/usr/bin/env python3
"""Check CLI install instructions and update their concrete release tags."""
from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "cargo install --git https://github.com/PatrickKoss/baukit --tag v"
SUFFIX = " --locked baukit-cli"
TAG = r"(?:\d+\.\d+\.\d+|X\.Y\.Z|{{ context\.template_version }})"
CANONICAL = re.compile(re.escape(PREFIX) + f"({TAG})" + re.escape(SUFFIX))
INSTALL = re.compile(r"cargo\s+install[^\n`]*")


def instruction_files(root: Path) -> list[Path]:
    result = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True, text=True, check=True)
    return [root / name for name in result.stdout.split("\0") if name and
            (name.endswith((".md", ".md.jinja")) or Path(name).name == "Makefile")]


def problems(source: str, version: str) -> list[str]:
    failures = []
    for match in INSTALL.finditer(source):
        command = match[0].strip()
        if not re.search(r"\bbaukit(?:-cli)?\b|--path\s+(?:\S*/)?cli\b", command):
            continue
        canonical = CANONICAL.fullmatch(command)
        if canonical is None:
            if command == PREFIX + "$(shell cat ../templates/VERSION)" + SUFFIX:
                continue
            failures.append(f"use the Git tag install command: {command}")
        elif canonical[1] not in (version, "X.Y.Z", "{{ context.template_version }}"):
            failures.append(f"stale CLI install tag v{canonical[1]}, expected v{version}")
    return failures


def update_tags(root: Path, version: str) -> None:
    for path in instruction_files(root):
        source = path.read_text()
        updated = CANONICAL.sub(lambda match: PREFIX + (version if re.fullmatch(r"\d+\.\d+\.\d+", match[1]) else match[1]) + SUFFIX, source)
        if updated != source:
            path.write_text(updated)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--update")
    arguments = parser.parse_args()
    version = arguments.update or (ROOT / "templates/VERSION").read_text().strip()
    if arguments.update:
        update_tags(ROOT, version)
    failures = [f"{path.relative_to(ROOT)}: {problem}" for path in instruction_files(ROOT) for problem in problems(path.read_text(), version)]
    if failures:
        raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    main()
