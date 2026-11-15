#!/usr/bin/env python3
"""Fail when a Flux Kustomization creates workloads before the Kyverno policies can admit them.

Each Kustomization path is rendered with kustomize. One that renders Pods, a Pod controller, a
HelmRelease or a CNPG Cluster must depend, directly or through dependsOn, on the Kustomization
composing kyverno-policies. Otherwise the API server admits its Pods without the Deny policies.
A reconciliation file without kyverno-policies has nothing to order against and passes.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Callable
from pathlib import Path

import helm_yaml
from check_exclusive_bases import PLATFORM, composed_bases
from check_monitor_order import PLATFORM_PREFIX, kustomizations, reaches

POLICY_BASE = "kyverno-policies"
WORKLOAD_KIND = re.compile(
    r"^kind: (Pod|Deployment|StatefulSet|DaemonSet|ReplicaSet|Job|CronJob|HelmRelease|Cluster)\s*$",
    re.M,
)
EXEMPT_SUFFIXES = {
    "-foundation": "rustfs storage that Kyverno does not need, runs as non-root with probes and "
    "is checked by the Kyverno CLI in ci-policies; holding it back would delay Kyverno for nothing",
    "-kyverno": "the admission engine itself cannot be admitted by policies it installs",
}


class RenderError(RuntimeError):
    pass


Render = Callable[[Path], str]


def exemption(name: str) -> str | None:
    return next(
        (reason for suffix, reason in EXEMPT_SUFFIXES.items() if name.endswith(suffix)), None
    )


def kustomize_render(kustomize: str) -> Render:
    def render(path: Path) -> str:
        result = helm_yaml.process_runner.run_process(
            [kustomize, "build", str(path)], capture_output=True, text=True
        )
        if result.returncode != 0:
            raise RenderError(f"kustomize build {path} failed:\n{result.stderr.strip()}")
        return result.stdout

    return render


def violations(reconciliation: Path, render: Render, platform: Path = PLATFORM) -> list[str]:
    specs = kustomizations(reconciliation)
    paths = {
        name: platform / spec["path"].removeprefix(PLATFORM_PREFIX) for name, spec in specs.items()
    }
    providers = {
        name for name, path in paths.items() if POLICY_BASE in composed_bases(path, platform)
    }
    if not providers:
        return []
    return [
        f"{reconciliation}: {name} creates workloads but does not depend on "
        f"{' or '.join(sorted(providers))}"
        for name, path in sorted(paths.items())
        if name not in providers
        and exemption(name) is None
        and WORKLOAD_KIND.search(render(path))
        and not reaches(name, providers, specs)
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kustomize", default="kustomize")
    parser.add_argument("reconciliations", nargs="+", type=Path)
    args = parser.parse_args(argv)
    render = kustomize_render(args.kustomize)
    try:
        found = [m for file in args.reconciliations for m in violations(file, render)]
    except RenderError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for message in found:
        print(f"ERROR: {message}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
