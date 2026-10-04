#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path


def linked_examples(root: Path) -> list[Path]:
    examples = root / "examples"
    packages = (root / "typescript/packages").resolve()
    directories = set()
    for directory, children, files in os.walk(examples):
        children[:] = sorted(
            name for name in children if name not in {"node_modules", ".git"}
        )
        if "package.json" not in files:
            continue
        manifest_dir = Path(directory)
        manifest = json.loads((manifest_dir / "package.json").read_text())
        requirements = (
            requirement
            for group in (
                "dependencies", "devDependencies", "optionalDependencies", "peerDependencies"
            )
            for requirement in manifest.get(group, {}).values()
        )
        if not any(
            requirement.startswith(("file:", "link:"))
            and (manifest_dir / requirement.split(":", 1)[1])
            .resolve()
            .is_relative_to(packages)
            for requirement in requirements
        ):
            continue
        lock_dir = manifest_dir
        while not (lock_dir / "pnpm-lock.yaml").is_file():
            if lock_dir == examples:
                raise ValueError(
                    f"Missing pnpm-lock.yaml for {manifest_dir.relative_to(root)}"
                )
            lock_dir = lock_dir.parent
        directories.add(lock_dir)
    return sorted(directories)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Check lockfiles for examples linked to local TypeScript packages."
    )
    parser.add_argument(
        "--refresh", action="store_true",
        help="Refresh lockfiles after changing local packages.",
    )
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    for directory in linked_examples(root):
        action = "Refreshing" if arguments.refresh else "Checking"
        print(f"{action} {directory.relative_to(root)}/pnpm-lock.yaml", flush=True)
        subprocess.run(
            [
                "corepack", "pnpm", "install", "--lockfile-only", "--ignore-scripts",
                "--no-frozen-lockfile" if arguments.refresh else "--frozen-lockfile",
            ],
            cwd=directory,
            check=True,
        )


if __name__ == "__main__":
    main()
