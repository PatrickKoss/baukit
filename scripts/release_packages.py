"""Read the TypeScript package inventory for the release train."""

from __future__ import annotations

import json
from pathlib import Path


def typescript_manifests(root: Path) -> dict[str, Path]:
    manifests = {}
    for directory in sorted((root / "typescript/packages").iterdir()):
        if not directory.is_dir():
            continue
        path = directory / "package.json"
        name = json.loads(path.read_text())["name"]
        if not name.startswith("@baukit/"):
            raise ValueError(f"{path.relative_to(root)} must name an @baukit/* package")
        if name in manifests:
            raise ValueError(f"duplicate TypeScript package name: {name}")
        manifests[name] = path
    return manifests


if __name__ == "__main__":
    for package in typescript_manifests(Path(__file__).resolve().parents[1]):
        print(package)
