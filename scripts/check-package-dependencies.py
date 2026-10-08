#!/usr/bin/env python3
"""Reject local dependency specifiers in the npm archives we publish."""

from __future__ import annotations

import argparse
import json
import subprocess
import tarfile
import tempfile
from pathlib import Path

from release_packages import typescript_manifests

ROOT = Path(__file__).resolve().parents[1]
DEPENDENCY_GROUPS = ("dependencies", "peerDependencies", "optionalDependencies")
LOCAL_PROTOCOLS = ("workspace:", "link:", "file:")


def check_package(directory: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="baukit-npm-pack-") as temporary:
        result = subprocess.run(
            ["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", temporary],
            cwd=directory, check=True, capture_output=True, text=True,
        )
        packed = json.loads(result.stdout)
        if len(packed) != 1:
            raise ValueError(f"{directory}: expected one npm archive")
        with tarfile.open(Path(temporary) / packed[0]["filename"], "r:gz") as archive:
            manifest = archive.extractfile("package/package.json")
            if manifest is None:
                raise ValueError(f"{directory}: npm archive has no package.json")
            package = json.load(manifest)
        problems = [
            f"{package['name']} {group}: {name} requires {specifier}"
            for group in DEPENDENCY_GROUPS
            for name, specifier in package.get(group, {}).items()
            if specifier.startswith(LOCAL_PROTOCOLS)
        ]
        if problems:
            raise ValueError("\n".join(problems))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="*", type=Path)
    args = parser.parse_args()
    directories = args.packages or [path.parent for path in typescript_manifests(ROOT).values()]
    for directory in directories:
        try:
            check_package(directory.resolve())
        except (OSError, ValueError, subprocess.CalledProcessError, tarfile.TarError) as error:
            parser.exit(1, f"npm dependency check failed: {error}\n")
    print(f"Checked npm dependency specifiers in {len(directories)} packed packages")


if __name__ == "__main__":
    main()
