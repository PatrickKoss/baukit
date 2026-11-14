#!/usr/bin/env python3
"""Fail when a Prometheus retentionSize is missing or above 90% of its PVC.

Builds every platform base and overlay with kustomize and reads the
kube-prometheus-stack HelmRelease values each one ends up with.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Iterable
from decimal import Decimal
from pathlib import Path

import helm_yaml

PLATFORM = Path(__file__).resolve().parents[1]
KUSTOMIZE = PLATFORM.parents[1] / ".tools/bin/kustomize"
RELEASE = "kube-prometheus-stack"
MAX_RATIO = Decimal("0.9")
KUBE_SUFFIXES = {
    "": 1,
    "Ki": 2**10,
    "Mi": 2**20,
    "Gi": 2**30,
    "Ti": 2**40,
    "Pi": 2**50,
    "Ei": 2**60,
    "k": 10**3,
    "M": 10**6,
    "G": 10**9,
    "T": 10**12,
    "P": 10**15,
    "E": 10**18,
}
KUBE_QUANTITY = re.compile(r"^([0-9]+(?:\.[0-9]+)?)(Ki|Mi|Gi|Ti|Pi|Ei|k|M|G|T|P|E)?$")
# Prometheus parses retention.size as base-2 bytes, so KB and KiB are both 1024.
PROMETHEUS_SIZE = re.compile(r"^([0-9]+(?:\.[0-9]+)?)(?:([KMGTPE])i?)?B$")
PROMETHEUS_UNITS = {None: 1, "K": 2**10, "M": 2**20, "G": 2**30, "T": 2**40, "P": 2**50, "E": 2**60}


class RetentionError(Exception):
    pass


def kube_bytes(quantity: str) -> Decimal:
    match = KUBE_QUANTITY.match(quantity)
    if match is None:
        raise RetentionError(f"cannot parse PVC size {quantity!r}")
    return Decimal(match.group(1)) * KUBE_SUFFIXES[match.group(2) or ""]


def prometheus_bytes(size: str) -> Decimal:
    match = PROMETHEUS_SIZE.match(size)
    if match is None:
        raise RetentionError(f"cannot parse retentionSize {size!r}")
    return Decimal(match.group(1)) * PROMETHEUS_UNITS[match.group(2)]


def dig(mapping: object, *keys: str) -> object:
    for key in keys:
        if not isinstance(mapping, dict):
            return None
        mapping = mapping.get(key)
    return mapping


def release_problems(source: str, release: dict) -> list[str]:
    spec = dig(release, "spec", "values", "prometheus", "prometheusSpec")
    claim = dig(spec, "storageSpec", "volumeClaimTemplate", "spec")
    pvc = dig(claim, "resources", "requests", "storage")
    size = dig(spec, "retentionSize")
    if not isinstance(pvc, str):
        return [f"{source}: {RELEASE} sets no Prometheus PVC size"]
    if not isinstance(size, str):
        return [f"{source}: {RELEASE} sets no retentionSize for its {pvc} PVC"]
    try:
        ratio = prometheus_bytes(size) / kube_bytes(pvc)
    except RetentionError as error:
        return [f"{source}: {error}"]
    if ratio > MAX_RATIO:
        return [f"{source}: retentionSize {size} is {ratio:.0%} of the {pvc} PVC, above 90%"]
    return []


def problems_in(source: str, rendered: str) -> list[str]:
    documents = helm_yaml.parse_documents(helm_yaml.split_documents(rendered))
    releases = [
        document
        for document in documents
        if document.get("kind") == "HelmRelease" and dig(document, "metadata", "name") == RELEASE
    ]
    return [problem for release in releases for problem in release_problems(source, release)]


def kustomizations(platform: Path) -> list[Path]:
    patterns = ("*/kustomization.yaml", "overlays/*/kustomization.yaml")
    patterns += ("overlays/*/components/*/kustomization.yaml",)
    return sorted({path.parent for pattern in patterns for path in platform.glob(pattern)})


def build(kustomize: Path, directory: Path) -> str:
    result = helm_yaml.process_runner.run_process(
        [str(kustomize), "build", str(directory)], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RetentionError(f"kustomize build {directory} failed:\n{result.stderr.strip()}")
    return result.stdout


def check(kustomize: Path, directories: Iterable[Path]) -> tuple[list[str], int]:
    problems: list[str] = []
    checked = 0
    for directory in directories:
        rendered = build(kustomize, directory)
        if f"name: {RELEASE}" not in rendered:
            continue
        checked += 1
        problems.extend(problems_in(str(directory), rendered))
    return problems, checked


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--kustomize", type=Path, default=KUSTOMIZE)
    parser.add_argument("directories", type=Path, nargs="*")
    args = parser.parse_args(argv)
    directories = args.directories or kustomizations(PLATFORM)
    try:
        problems, checked = check(args.kustomize, directories)
    except (RetentionError, helm_yaml.YamlError, OSError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for problem in problems:
        print(f"ERROR: {problem}", file=sys.stderr)
    if checked == 0:
        print(f"ERROR: no {RELEASE} HelmRelease found", file=sys.stderr)
        return 1
    print(f"retentionSize: {checked} renders with {RELEASE} checked")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
