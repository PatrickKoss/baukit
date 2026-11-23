#!/usr/bin/env python3
"""Renders deploy/gitops overlays and the baukit-app chart with their HelmRelease values.

Usage: scripts/check-deploy-values.py <product root> <baukit-app chart directory>
Needs kustomize and helm on PATH. The product gate's deploy-values step runs it.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

import yaml
from process_runner import run_process

CHART_NAME = "baukit-app"
SKIPPED_OVERLAYS = {"base"}
WORKLOAD_KINDS = {"Deployment", "StatefulSet", "DaemonSet", "Job"}
ARGUMENT_COUNT = 3
USAGE_EXIT = 2


class RenderError(Exception):
    pass


def output_of(command: list[str]) -> str:
    result = run_process(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise RenderError(f"{' '.join(command[:2])} failed: {result.stderr.strip()}")
    return result.stdout


def documents_of(text: str) -> list[dict]:
    return [document for document in yaml.safe_load_all(text) if document]


def overlays(root: Path) -> list[Path]:
    gitops = root / "deploy" / "gitops"
    if not gitops.is_dir():
        return []
    return [
        directory
        for directory in sorted(gitops.iterdir())
        if directory.name not in SKIPPED_OVERLAYS
        and (directory / "kustomization.yaml").is_file()
        and yaml.safe_load((directory / "kustomization.yaml").read_text()).get("kind")
        != "Component"
    ]


def chart_of(release: dict) -> str:
    return release.get("spec", {}).get("chart", {}).get("spec", {}).get("chart", "")


def chart_releases(documents: list[dict]) -> list[dict]:
    return [
        document
        for document in documents
        if document.get("kind") == "HelmRelease" and Path(chart_of(document)).name == CHART_NAME
    ]


def pod_spec(document: dict) -> dict | None:
    if document.get("kind") not in WORKLOAD_KINDS:
        return None
    return document["spec"]["template"]["spec"]


def duplicate_env(documents: list[dict]) -> list[str]:
    errors = []
    for document in documents:
        spec = pod_spec(document)
        if spec is None:
            continue
        workload = f"{document['kind']} {document['metadata']['name']}"
        for container in spec.get("initContainers", []) + spec.get("containers", []):
            counts = Counter(entry["name"] for entry in container.get("env", []))
            for name, count in sorted(counts.items()):
                if count > 1:
                    errors.append(
                        f"{workload} container {container['name']} sets {name} {count} times; "
                        "server-side apply rejects duplicate env entries"
                    )
    return errors


def render_release(chart: Path, release: dict) -> list[dict]:
    namespace = release["metadata"].get("namespace", "default")
    name = release["spec"].get("releaseName", release["metadata"]["name"])
    with tempfile.NamedTemporaryFile("w", suffix=".yaml") as values:
        yaml.safe_dump(release["spec"].get("values", {}), values)
        values.flush()
        output_of(["helm", "lint", "--strict", str(chart), "-f", values.name])
        rendered = output_of(
            ["helm", "template", name, str(chart), "--namespace", namespace, "-f", values.name]
        )
    return documents_of(rendered)


def check_overlay(chart: Path, overlay: Path) -> list[str]:
    try:
        releases = chart_releases(documents_of(output_of(["kustomize", "build", str(overlay)])))
        if not releases:
            return [f"renders no HelmRelease of the {CHART_NAME} chart"]
        return [
            f"HelmRelease {release['metadata']['name']}: {error}"
            for release in releases
            for error in duplicate_env(render_release(chart, release))
        ]
    except RenderError as error:
        return [str(error)]


def check(root: Path, chart: Path) -> list[str]:
    found = overlays(root)
    if not found:
        return ["deploy/gitops has no overlay with a kustomization.yaml"]
    return [
        f"{overlay.relative_to(root)}: {error}"
        for overlay in found
        for error in check_overlay(chart, overlay)
    ]


def main(argv: list[str]) -> int:
    if len(argv) != ARGUMENT_COUNT:
        print(__doc__, file=sys.stderr)
        return USAGE_EXIT
    root, chart = Path(argv[1]).resolve(), Path(argv[2]).resolve()
    try:
        errors = check(root, chart)
    except (OSError, subprocess.SubprocessError, yaml.YAMLError) as error:
        errors = [str(error)]
    for error in errors:
        print(f"deploy values: {error}", file=sys.stderr)
    if not errors:
        print(f"deploy values: {len(overlays(root))} overlays render with {chart}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
