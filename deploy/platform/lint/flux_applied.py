"""Print the directories Flux applies for one rendered overlay, NUL-separated and deduplicated."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import flux_sources
import helm_yaml

FLUX_KUSTOMIZATION_API = "kustomize.toolkit.fluxcd.io/v1"


def applied_specs(objects: list[dict]) -> dict[str, dict]:
    return {
        (obj.get("metadata") or {}).get("name", ""): obj.get("spec") or {}
        for obj in objects
        if obj.get("apiVersion") == FLUX_KUSTOMIZATION_API and obj.get("kind") == "Kustomization"
    }


def applied_dirs(objects: list[dict], platform: Path) -> list[Path]:
    dirs = flux_sources.kustomization_dirs(applied_specs(objects), {}, platform)
    return sorted({directory.resolve() for directory in dirs.values()})


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rendered", type=Path)
    parser.add_argument("--platform", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args(argv)
    documents = helm_yaml.split_documents(args.rendered.read_text())
    objects = [obj for obj in helm_yaml.parse_documents(documents) if obj]
    try:
        dirs = applied_dirs(objects, args.platform.resolve())
    except flux_sources.SourceError as error:
        print(f"ERROR: {args.rendered.name}: {error}", file=sys.stderr)
        return 1
    sys.stdout.write("".join(f"{directory}\0" for directory in dirs))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
