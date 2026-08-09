#!/usr/bin/env python3
"""Resolve observability metric references against one or more live scrapes."""

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
from pathlib import Path
from types import ModuleType


SAMPLE_NAME = re.compile(r"^([A-Za-z_:][A-Za-z0-9_:]*)")


def arguments() -> argparse.Namespace:
    default_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--observability-root", type=Path, default=default_root)
    parser.add_argument(
        "--metrics",
        action="append",
        required=True,
        metavar="PROCESS=PATH",
        help="Prometheus text scrape; repeat for every process",
    )
    parser.add_argument(
        "--known-gap",
        action="append",
        default=[],
        metavar="METRIC",
        help="Metric name expected to remain unresolved; repeat as needed",
    )
    parser.add_argument(
        "--known-gap-file",
        action="append",
        default=[],
        type=Path,
        metavar="PATH",
        help="Newline-delimited expected unresolved metrics; # starts a comment",
    )
    return parser.parse_args()


def load_linter(path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location("baukit_metric_linter", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load Baukit metric linter from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def metric_argument(value: str) -> tuple[str, Path]:
    process, separator, raw_path = value.partition("=")
    if not separator or not process or not raw_path:
        raise ValueError(f"invalid --metrics value {value!r}; expected PROCESS=PATH")
    return process, Path(raw_path)


def exported_names(path: Path) -> set[str]:
    names: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith("# HELP ") or stripped.startswith("# TYPE "):
            parts = stripped.split()
            if len(parts) >= 3:
                names.add(parts[2])
            continue
        if stripped.startswith("#"):
            continue
        match = SAMPLE_NAME.match(stripped)
        if match is not None:
            names.add(match.group(1))
    return names


def known_gaps(direct: list[str], paths: list[Path]) -> set[str]:
    gaps = set(direct)
    for path in paths:
        for line in path.read_text(encoding="utf-8").splitlines():
            value = line.split("#", 1)[0].strip()
            if value:
                gaps.add(value)
    return gaps


def main() -> int:
    args = arguments()
    observability = args.observability_root.resolve()
    linter = load_linter(observability / "lint" / "check-metric-names.py")

    scrapes: dict[str, set[str]] = {}
    try:
        for value in args.metrics:
            process, path = metric_argument(value)
            if process in scrapes:
                raise ValueError(f"duplicate process name in --metrics: {process}")
            scrapes[process] = exported_names(path)
    except (OSError, ValueError) as error:
        print(f"cannot read metrics: {error}", file=sys.stderr)
        return 2

    application_names = set().union(*scrapes.values())
    rule_paths = sorted(
        list((observability / "recording-rules").glob("*.yml"))
        + list((observability / "recording-rules").glob("*.yaml"))
        + list((observability / "alerts").glob("*.yml"))
        + list((observability / "alerts").glob("*.yaml"))
    )
    local_recordings: set[str] = set()
    rule_documents: list[tuple[Path, str]] = []
    for path in rule_paths:
        content = path.read_text(encoding="utf-8")
        rule_documents.append((path, content))
        local_recordings.update(linter.RECORD_NAME.findall(content))

    available = application_names | local_recordings | set(linter.PROMETHEUS_BUILTINS)
    unresolved: dict[str, set[str]] = {}

    def check(location: str, expression: str) -> None:
        missing = linter.metric_references(expression) - available
        if missing:
            unresolved[location] = missing

    dashboard_paths = sorted((observability / "dashboards").glob("*.json"))
    for path in dashboard_paths:
        for location, expression in linter.dashboard_expressions(path):
            check(location, expression)
    for path, content in rule_documents:
        relative = path.relative_to(observability)
        for line_number, expression in linter.rule_expressions(content):
            check(f"{relative}:{line_number}", expression)

    for process, names in sorted(scrapes.items()):
        print(f"{process} exported metric names ({len(names)}): {', '.join(sorted(names))}")
    print(
        f"Checked {len(dashboard_paths)} dashboard(s), {len(rule_paths)} rule file(s), "
        f"and {len(local_recordings)} recording rule(s)."
    )

    if unresolved:
        print("Unresolved observability metric names:", file=sys.stderr)
        for location, names in sorted(unresolved.items()):
            print(f"- {location}: {', '.join(sorted(names))}", file=sys.stderr)
    else:
        print("Unresolved observability metric names: none")

    actual_names = set().union(*unresolved.values()) if unresolved else set()
    try:
        expected_names = known_gaps(args.known_gap, args.known_gap_file)
    except OSError as error:
        print(f"cannot read known-gap allowlist: {error}", file=sys.stderr)
        return 2
    if actual_names != expected_names:
        print(
            "Known-gap allowlist mismatch: "
            f"expected {sorted(expected_names)}, found {sorted(actual_names)}",
            file=sys.stderr,
        )
        return 1
    if expected_names:
        print(f"Known-gap allowlist confirmed: {', '.join(sorted(expected_names))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
