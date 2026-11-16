#!/usr/bin/env python3
"""Fail when a Flux Kustomization applies Prometheus Operator objects before their CRDs exist.

A Kustomization whose bases render a ServiceMonitor, PodMonitor or PrometheusRule (in validate.sh's
rendered output) must depend, directly or through dependsOn, on the Kustomization that composes
observability-stack, which installs those CRDs. Paths resolve through flux_sources, so a consumer
passes --source-root for its own GitRepository.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Mapping
from pathlib import Path

import helm_yaml
from check_exclusive_bases import PLATFORM, composed_bases
from flux_sources import SourceError, add_source_root_argument, kustomization_dirs

CRD_BASE = "observability-stack"
MONITOR_KIND = re.compile(r"^kind: (ServiceMonitor|PodMonitor|PrometheusRule)\s*$", re.M)
FLUX_KUSTOMIZATION = "kustomize.toolkit.fluxcd.io/"


def monitor_bases(rendered: Path, bases: set[str]) -> set[str]:
    found = set()
    for file in rendered.glob("*.yaml"):
        if not MONITOR_KIND.search(file.read_text()):
            continue
        base = rendered_base(file.stem, bases)
        if base:
            found.add(base)
    return found


def rendered_base(stem: str, bases: set[str]) -> str | None:
    for prefix in ("base-", "chart-"):
        if stem.startswith(prefix):
            name = stem.removeprefix(prefix)
            matches = [base for base in bases if name == base or name.startswith(f"{base}-")]
            return max(matches, key=len, default=None)
    return None


def kustomizations(reconciliation: Path) -> dict[str, dict]:
    text = reconciliation.read_text()
    documents = helm_yaml.parse_documents(helm_yaml.split_documents(text))
    return {
        doc["metadata"]["name"]: doc["spec"]
        for doc in documents
        if doc.get("kind") == "Kustomization" and FLUX_KUSTOMIZATION in doc.get("apiVersion", "")
    }


def reaches(name: str, targets: set[str], specs: dict[str, dict]) -> bool:
    pending, seen = [name], set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        for dependency in specs.get(current, {}).get("dependsOn", []):
            if dependency["name"] in targets:
                return True
            pending.append(dependency["name"])
    return False


def violations(
    reconciliation: Path,
    monitors: set[str],
    platform: Path = PLATFORM,
    roots: Mapping[str, Path] | None = None,
) -> list[str]:
    specs = kustomizations(reconciliation)
    try:
        paths = kustomization_dirs(specs, roots or {}, platform)
    except SourceError as error:
        return [f"{reconciliation}: {error}"]
    bases = {name: composed_bases(path, platform) for name, path in paths.items()}
    providers = {name for name, composed in bases.items() if CRD_BASE in composed}
    return [
        f"{reconciliation}: {name} renders Prometheus Operator objects but does not depend on "
        f"{' or '.join(sorted(providers)) or f'a Kustomization composing {CRD_BASE}'}"
        for name, composed in sorted(bases.items())
        if composed & monitors and name not in providers and not reaches(name, providers, specs)
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("reconciliations", nargs="+", type=Path)
    add_source_root_argument(parser)
    args = parser.parse_args(argv)
    roots = dict(args.source_roots)
    platform_bases = {path.parent.name for path in PLATFORM.glob("*/kustomization.yaml")}
    monitors = monitor_bases(args.rendered, platform_bases)
    found = [
        m for file in args.reconciliations for m in violations(file, monitors, PLATFORM, roots)
    ]
    for message in found:
        print(f"ERROR: {message}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
