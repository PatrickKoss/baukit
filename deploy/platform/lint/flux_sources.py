"""Map Flux Kustomization paths onto local checkouts of their GitRepository sources.

sourceRef GitRepository/baukit resolves to the baukit checkout these checkers run from, so its
paths must start with ./deploy/platform/. Every other source needs a --source-root NAME=DIR, for
example flux-system=<consumer repo root>. A Kustomization whose source or path does not resolve
is an error, never skipped.
"""

from __future__ import annotations

import argparse
from collections.abc import Mapping
from pathlib import Path

BAUKIT_SOURCE = "baukit"
PLATFORM_PREFIX = "./deploy/platform/"
SOURCE_KIND = "GitRepository"


class SourceError(ValueError):
    pass


def source_root(value: str) -> tuple[str, Path]:
    name, separator, directory = value.partition("=")
    if not separator or not name or not directory:
        raise argparse.ArgumentTypeError(f"expected NAME=DIR, got {value!r}")
    if name == BAUKIT_SOURCE:
        raise argparse.ArgumentTypeError(
            "GitRepository/baukit always resolves to the checkout that runs the checker"
        )
    root = Path(directory)
    if not root.is_dir():
        raise argparse.ArgumentTypeError(f"source root {directory} is not a directory")
    return name, root.resolve()


def add_source_root_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--source-root",
        dest="source_roots",
        action="append",
        type=source_root,
        default=[],
        metavar="NAME=DIR",
        help="local checkout of GitRepository NAME, such as flux-system=<consumer repo root>",
    )


def kustomization_dir(name: str, spec: dict, roots: Mapping[str, Path], platform: Path) -> Path:
    reference = spec.get("sourceRef") or {}
    source = reference.get("name")
    kind = reference.get("kind", SOURCE_KIND)
    path = spec.get("path", "./")
    if kind != SOURCE_KIND or not source:
        raise SourceError(f"{name}: sourceRef must name a {SOURCE_KIND}")
    if source == BAUKIT_SOURCE:
        if not path.startswith(PLATFORM_PREFIX):
            raise SourceError(f"{name}: baukit path {path} is outside {PLATFORM_PREFIX}")
        directory = platform / path.removeprefix(PLATFORM_PREFIX)
    elif source in roots:
        directory = roots[source] / path
    else:
        raise SourceError(
            f"{name}: no checkout for {SOURCE_KIND}/{source}; pass --source-root {source}=DIR"
        )
    if not directory.is_dir():
        raise SourceError(f"{name}: path {path} of {SOURCE_KIND}/{source} is not at {directory}")
    return directory


def kustomization_dirs(
    specs: Mapping[str, dict], roots: Mapping[str, Path], platform: Path
) -> dict[str, Path]:
    resolved, errors = {}, []
    for name, spec in specs.items():
        try:
            resolved[name] = kustomization_dir(name, spec, roots, platform)
        except SourceError as error:
            errors.append(str(error))
    if errors:
        raise SourceError("; ".join(errors))
    return resolved
