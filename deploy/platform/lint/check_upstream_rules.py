#!/usr/bin/env python3
"""Fail when an upstream rule copy that a promtool test loads differs from the rendered chart.

Each copy in deploy/observability-platform/upstream-rules/ names its source PrometheusRule in
UPSTREAM_RULES. The copy's groups must equal that PrometheusRule's spec.groups in the
rendered chart, so the test exercises the expression Prometheus really loads.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import helm_yaml

REPO_ROOT = Path(__file__).resolve().parents[3]
UPSTREAM_DIR = REPO_ROOT / "deploy/observability-platform/upstream-rules"
UPSTREAM_RULES = {
    "kubernetes-storage.rules.yml": (
        "chart-observability-stack-kube-prometheus-stack.yaml",
        "kube-prometheus-stack-kubernetes-storage",
    ),
}


def rendered_groups(rendered: Path, rule_name: str) -> list | None:
    candidates = [
        document
        for document in helm_yaml.split_documents(rendered.read_text())
        if "kind: PrometheusRule" in document and rule_name in document
    ]
    documents = helm_yaml.parse_documents(candidates)
    for document in documents:
        metadata = document.get("metadata") or {}
        if document.get("kind") == "PrometheusRule" and metadata.get("name") == rule_name:
            return (document.get("spec") or {}).get("groups")
    return None


def check(rendered_dir: Path, upstream_dir: Path, rules: dict[str, tuple[str, str]]) -> list[str]:
    problems = []
    for copy_name, (rendered_name, rule_name) in sorted(rules.items()):
        copy = upstream_dir / copy_name
        [parsed_copy] = helm_yaml.parse_documents([copy.read_text()])
        groups = rendered_groups(rendered_dir / rendered_name, rule_name)
        if groups is None:
            problems.append(
                f"{rendered_name} has no PrometheusRule {rule_name}; "
                f"the rules {copy} tests are not deployed"
            )
        elif parsed_copy.get("groups") != groups:
            problems.append(
                f"{copy} differs from PrometheusRule {rule_name} in {rendered_name}; "
                "copy its spec.groups from the render and rerun the promtool tests"
            )
    unlisted = sorted(path.name for path in upstream_dir.glob("*.y*ml") if path.name not in rules)
    problems.extend(f"{upstream_dir / name} has no source in UPSTREAM_RULES" for name in unlisted)
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--upstream-dir", type=Path, default=UPSTREAM_DIR)
    args = parser.parse_args(argv)
    try:
        problems = check(args.rendered, args.upstream_dir, UPSTREAM_RULES)
    except (helm_yaml.YamlError, OSError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for problem in problems:
        print(f"ERROR: {problem}", file=sys.stderr)
    if not problems:
        print(f"upstream rules: {len(UPSTREAM_RULES)} copies match the rendered charts")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
