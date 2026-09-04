#!/usr/bin/env python3

import argparse
import re
from pathlib import Path


ASSIGNMENT = re.compile(
    rb"^[ \t]*(?:export[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)[ \t]*="
)


def assignments(source: bytes) -> dict[bytes, bytes]:
    found: dict[bytes, bytes] = {}
    for line in source.splitlines():
        match = ASSIGNMENT.match(line)
        if match is not None:
            found.setdefault(match.group(1), line)
    return found


def reconcile(example_path: Path, env_path: Path) -> list[str]:
    example = example_path.read_bytes()
    current = env_path.read_bytes() if env_path.exists() else b""
    current_keys = set(assignments(current))
    missing = [
        (key, line)
        for key, line in assignments(example).items()
        if key not in current_keys
    ]
    if not missing:
        return []

    separator = b"" if not current or current.endswith((b"\n", b"\r")) else b"\n"
    addition = b"\n".join(line for _, line in missing) + b"\n"
    env_path.write_bytes(current + separator + addition)
    return [key.decode("ascii") for key, _ in missing]


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Append assignments missing from a local environment file."
    )
    parser.add_argument("example", type=Path)
    parser.add_argument("env", type=Path)
    args = parser.parse_args()
    for key in reconcile(args.example, args.env):
        print(f"added {key}")


if __name__ == "__main__":
    main()
