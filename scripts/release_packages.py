"""Read the TypeScript package inventory for the release train."""

from __future__ import annotations

import argparse
import json
import re
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


def cut_package_changelog(source: str, version: str) -> str:
    sections = re.split(r"^## ", source, flags=re.MULTILINE)
    pending = [section for section in sections[1:]
               if section.partition("\n")[0] in ("Unreleased", "[Unreleased]")]
    releases = [section for section in sections[1:]
                if section.partition("\n")[0] == version]
    if len(pending) > 1 or len(releases) != 1 or any(
        not section.partition("\n")[2].startswith("\n") for section in pending
    ):
        raise ValueError("still has uncut Unreleased entries or is missing the new release heading")

    notes = pending[0].partition("\n")[2].strip() if pending else ""
    history = [section for section in sections[1:] if section not in pending]
    if notes:
        index = history.index(releases[0])
        heading, _, body = history[index].partition("\n")
        history[index] = f"{heading}\n\n{notes}\n\n{body.lstrip()}"
    unreleased = pending[0].partition("\n")[0] if pending else "Unreleased"
    return sections[0].rstrip() + f"\n\n## {unreleased}\n\n" + "\n\n".join(
        "## " + section.rstrip() for section in history
    ) + "\n"


def cut_package_changelogs(root: Path, version: str) -> None:
    for manifest in typescript_manifests(root).values():
        path = manifest.with_name("CHANGELOG.md")
        try:
            source = cut_package_changelog(path.read_text(), version)
        except ValueError as error:
            raise ValueError(f"{path}: {error}") from error
        path.write_text(source)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--cut-changelogs", metavar="VERSION")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if args.cut_changelogs:
        try:
            cut_package_changelogs(root, args.cut_changelogs)
        except (OSError, ValueError) as error:
            parser.exit(1, f"{error}\n")
    else:
        for package in typescript_manifests(root):
            print(package)
