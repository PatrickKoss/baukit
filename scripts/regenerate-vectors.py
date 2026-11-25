#!/usr/bin/env python3
"""Regenerate every fixtures/**/generated-v1.json from scripts/vector-generators.toml.

`--check` also fails when fixtures/ differs from git after regenerating.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "scripts" / "vector-generators.toml"
GIT_DIFF = ["git", "diff", "--exit-code", "--stat", "--", "fixtures"]
GENERATED_GLOB = "fixtures/**/generated-v1.json"


def registry_errors(generators: list[dict], root: Path) -> list[str]:
    outputs = [generator["output"] for generator in generators]
    on_disk = {path.relative_to(root).as_posix() for path in root.glob(GENERATED_GLOB)}
    errors = [
        f"{output} has more than one generator"
        for output in set(outputs)
        if outputs.count(output) > 1
    ]
    errors += [
        f"{output} has no generator in {REGISTRY.name}" for output in sorted(on_disk - set(outputs))
    ]
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check", action="store_true", help="fail when regenerating changes fixtures/"
    )
    args = parser.parse_args(argv)
    generators = tomllib.loads(REGISTRY.read_text(encoding="utf-8")).get("generator", [])
    errors = registry_errors(generators, ROOT)
    for generator in generators:
        print(f"regenerate-vectors: {generator['output']}", flush=True)
        command = generator["command"]
        status = subprocess.run(command, cwd=ROOT, check=False)  # noqa: S603 -- checked-in argv
        if status.returncode != 0:
            errors.append(f"generator for {generator['output']} failed")
    if not errors and args.check:
        diff = subprocess.run(  # noqa: S603 -- fixed git argv, no shell
            GIT_DIFF,  # noqa: S607 -- git comes from PATH
            cwd=ROOT,
            check=False,
        )
        if diff.returncode != 0:
            errors.append("generated vectors differ from the committed fixtures")
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(f"regenerate-vectors: {len(generators)} generators")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
