#!/usr/bin/env python3
"""Fail when one overlay composes two platform bases that own the same objects.

An overlay composes a base through kustomize resources or components, or through a Flux
Kustomization whose spec.path points at ./deploy/platform/<base>.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

PLATFORM = Path(__file__).resolve().parent.parent
EXCLUSIVE_GROUPS = (frozenset({"redis", "redis-ha"}),)
REFERENCE = re.compile(r"^\s+- (\S+)\s*$", re.M)
FLUX_PATH = re.compile(r"^\s+path: \./deploy/platform/(\S+?)/?\s*$", re.M)


def composed_bases(overlay: Path, platform: Path = PLATFORM) -> set[str]:
    platform = platform.resolve()
    pending = [overlay.resolve()]
    seen: set[Path] = set()
    while pending:
        directory = pending.pop()
        if directory in seen:
            continue
        seen.add(directory)
        for manifest in directory.glob("*.yaml"):
            text = manifest.read_text()
            targets = [directory / ref for ref in REFERENCE.findall(text)]
            targets += [platform / path for path in FLUX_PATH.findall(text)]
            pending += [t.resolve() for t in targets if (t / "kustomization.yaml").is_file()]
    return {d.name for d in seen if d.parent == platform}


def violations(overlay: Path, platform: Path = PLATFORM) -> list[str]:
    bases = composed_bases(overlay, platform)
    return [
        f"{overlay}: composes {' and '.join(sorted(group))}, which own the same objects"
        for group in EXCLUSIVE_GROUPS
        if group <= bases
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("overlays", nargs="+", type=Path)
    args = parser.parse_args(argv)
    found = [message for overlay in args.overlays for message in violations(overlay)]
    for message in found:
        print(f"ERROR: {message}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
