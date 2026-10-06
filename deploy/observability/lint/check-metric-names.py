#!/usr/bin/env python3
"""Lint observability expressions against Baukit's telemetry contract."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Sequence
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
OBSERVABILITY = ROOT / "deploy" / "observability"

# Source of truth: docs/platform/telemetry-spec.md, section 2.
SPEC_METRICS = (
    "http_requests_total",
    "mcp_requests_total",
    "mcp_request_duration_seconds",
    "http_request_duration_seconds",
    "http_requests_in_flight",
    "http_rate_limit_decisions_total",
    "fixed_window_amount_budget_decisions_total",
    "build_info",
    "db_pool_connections_max",
    "db_pool_connections_idle",
    "db_pool_connections_in_use",
    "db_pool_acquire_duration_seconds",
    "db_pool_acquire_timeouts_total",
    "worker_job_runs_total",
    "worker_job_duration_seconds",
    "worker_queue_oldest_age_seconds",
)

PROMETHEUS_BUILTINS = {"up"}
HISTOGRAM_SUFFIXES = ("_bucket", "_count", "_sum")
HISTOGRAM_MARKER = "histogram"
METRIC_NAME = re.compile(r"^[A-Za-z_:][A-Za-z0-9_:]*$")
HISTOGRAM_METRICS = {
    "mcp_request_duration_seconds",
    "http_request_duration_seconds",
    "db_pool_acquire_duration_seconds",
    "worker_job_duration_seconds",
}
METRIC_SELECTOR = re.compile(
    r"(?<![A-Za-z0-9_:])([A-Za-z_:][A-Za-z0-9_:]*)\s*(?=\{|\[)"
)
IDENTIFIER = re.compile(r"(?<![A-Za-z0-9_:])([A-Za-z_:][A-Za-z0-9_:]*)")
RECORD_NAME = re.compile(r"^\s*-?\s*record:\s*([A-Za-z_:][A-Za-z0-9_:]*)\s*$", re.MULTILINE)
FORBIDDEN_PLURAL = re.compile(r"\bhttp_requests_duration_seconds(?:_[a-z]+)?\b")
LOKI_SERVICE_NAME = re.compile(r"\{[^}\n]*\bservice_name\s*(?:=|!=|=~|!~)")
STATUS_CLASS = re.compile(
    r"\bstatus\s*(?:=|!=|=~|!~)\s*[\"'](?:[1-5][xX]{2})[\"']"
)
PROMQL_KEYWORDS = {
    "and",
    "bool",
    "end",
    "group_left",
    "group_right",
    "ignoring",
    "json",  # LogQL parser stage used by the dashboard log panel.
    "offset",
    "on",
    "or",
    "start",
    "unless",
}


def dashboard_expressions(path: Path) -> list[tuple[str, str]]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: invalid dashboard JSON: {error}") from error

    expressions: list[tuple[str, str]] = []

    def visit(value: object, location: str) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                child_location = f"{location}.{key}"
                if key == "expr" and isinstance(child, str):
                    expressions.append((child_location, child))
                visit(child, child_location)
        elif isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, f"{location}[{index}]")

    visit(document, display_path(path))
    return expressions


def display_path(path: Path) -> str:
    """Show a path relative to Baukit or the working directory when possible."""
    resolved = path.resolve()
    for base in (ROOT, Path.cwd().resolve()):
        if resolved.is_relative_to(base):
            return str(resolved.relative_to(base))
    return str(resolved)


def lint_expression(
    location: str,
    expression: str,
    allowed_metrics: set[str],
    problems: list[str],
) -> None:
    if FORBIDDEN_PLURAL.search(expression):
        problems.append(f"{location}: forbidden plural HTTP duration metric")
    if LOKI_SERVICE_NAME.search(expression):
        problems.append(f"{location}: service_name is forbidden in Loki label selectors")
    if STATUS_CLASS.search(expression):
        problems.append(f"{location}: status classes are forbidden metric label values")

    references = metric_references(expression)
    if any("worker_job_" in metric for metric in references):
        unquoted = re.sub(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'', " ", expression)
        for labels in re.findall(
            r"\b(?:by|without|on|ignoring|group_left|group_right)\s*\(([^)]*)\)",
            unquoted,
        ):
            if {label.strip() for label in labels.split(",")} & {"job", "exported_job"}:
                problems.append(f"{location}: worker handler types must use job_kind, not job")

    for metric in sorted(references):
        if metric not in allowed_metrics:
            problems.append(f"{location}: unknown metric {metric!r}")


def metric_references(expression: str) -> set[str]:
    """Return metric identifiers while excluding functions and PromQL labels."""
    references = set(METRIC_SELECTOR.findall(expression))
    sanitized = re.sub(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'', " ", expression)
    sanitized = re.sub(r"\{[^}]*\}", " ", sanitized)
    sanitized = re.sub(r"\[[^]]*\]", " ", sanitized)
    sanitized = re.sub(
        r"\b(?:by|without|on|ignoring|group_left|group_right)\s*\([^)]*\)",
        " ",
        sanitized,
    )

    for match in IDENTIFIER.finditer(sanitized):
        identifier = match.group(1)
        if identifier in PROMQL_KEYWORDS:
            continue
        following = sanitized[match.end() :].lstrip()
        if following.startswith("("):
            continue
        references.add(identifier)
    return references


def rule_expressions(content: str) -> list[tuple[int, str]]:
    """Extract inline and block-scalar PromQL expressions from rule YAML."""
    lines = content.splitlines()
    expressions: list[tuple[int, str]] = []
    index = 0
    while index < len(lines):
        match = re.match(r"^(\s*)expr:\s*(.*)$", lines[index])
        if match is None:
            index += 1
            continue

        indentation = len(match.group(1))
        remainder = match.group(2).strip()
        line_number = index + 1
        if remainder not in {"", "|", ">", "|-", ">-", "|+", ">+"}:
            if len(remainder) >= 2 and remainder[0] == remainder[-1] and remainder[0] in "\"'":
                remainder = remainder[1:-1]
            expressions.append((line_number, remainder))
            index += 1
            continue

        block: list[str] = []
        index += 1
        while index < len(lines):
            line = lines[index]
            if line.strip() and len(line) - len(line.lstrip()) <= indentation:
                break
            if line.strip():
                block.append(line.strip())
            index += 1
        expressions.append((line_number, "\n".join(block)))
    return expressions


class ConfigurationError(Exception):
    """Invalid command-line input that prevents linting."""


def parse_arguments(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Lint dashboards, alerts, and recording rules against metric names."
    )
    parser.add_argument(
        "--observability-root",
        type=Path,
        default=OBSERVABILITY,
        metavar="DIR",
        help="directory holding dashboards/, alerts/, and recording-rules/ "
        "(default: Baukit's deploy/observability)",
    )
    parser.add_argument(
        "--allowlist",
        type=Path,
        metavar="FILE",
        help="product metric names, one per line; append 'histogram' to also allow "
        "the _bucket, _count, and _sum series; # starts a comment",
    )
    parser.add_argument(
        "--rules",
        type=Path,
        action="append",
        default=[],
        metavar="FILE",
        help="extra Prometheus rule file outside the observability root; repeatable",
    )
    return parser.parse_args(list(argv))


def read_allowlist(path: Path) -> tuple[set[str], set[str]]:
    """Return product metric names and the subset that are histograms."""
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise ConfigurationError(f"{path}: cannot read allowlist: {error}") from error

    names: set[str] = set()
    histograms: set[str] = set()
    for line_number, line in enumerate(lines, start=1):
        fields = line.split("#", 1)[0].split()
        if not fields:
            continue
        location = f"{display_path(path)}:{line_number}"
        name, kind = fields[0], fields[1:]
        if METRIC_NAME.fullmatch(name) is None:
            raise ConfigurationError(f"{location}: invalid metric name {name!r}")
        if kind not in ([], [HISTOGRAM_MARKER]):
            raise ConfigurationError(
                f"{location}: expected a metric name and optional {HISTOGRAM_MARKER!r}"
            )
        if name in names:
            raise ConfigurationError(f"{location}: duplicate metric name {name!r}")
        names.add(name)
        if kind:
            histograms.add(name)
    return names, histograms


def rule_paths(observability: Path, extra_rules: Sequence[Path]) -> list[Path]:
    paths = [
        path
        for directory in ("recording-rules", "alerts")
        for pattern in ("*.yml", "*.yaml")
        for path in (observability / directory).glob(pattern)
    ]
    return sorted(paths + list(extra_rules))


def read_rule_documents(
    paths: Sequence[Path], problems: list[str]
) -> list[tuple[Path, str]]:
    documents: list[tuple[Path, str]] = []
    for path in paths:
        try:
            documents.append((path, path.read_text(encoding="utf-8")))
        except OSError as error:
            problems.append(f"{path}: cannot read rule file: {error}")
    return documents


def allowed_metric_names(
    product_metrics: set[str], product_histograms: set[str], local_recordings: set[str]
) -> set[str]:
    exposed_metrics = set(SPEC_METRICS) | product_metrics
    for histogram in HISTOGRAM_METRICS | product_histograms:
        exposed_metrics.update(f"{histogram}{suffix}" for suffix in HISTOGRAM_SUFFIXES)
    return exposed_metrics | PROMETHEUS_BUILTINS | local_recordings


def lint_dashboards(
    paths: Sequence[Path], allowed_metrics: set[str], problems: list[str]
) -> None:
    for path in paths:
        try:
            expressions = dashboard_expressions(path)
        except ValueError as error:
            problems.append(str(error))
            continue
        for location, expression in expressions:
            lint_expression(location, expression, allowed_metrics, problems)


def lint_rules(
    documents: Sequence[tuple[Path, str]], allowed_metrics: set[str], problems: list[str]
) -> None:
    for path, content in documents:
        location = display_path(path)
        for line_number, expression in rule_expressions(content):
            lint_expression(
                f"{location}:{line_number}", expression, allowed_metrics, problems
            )


def load_product_metrics(allowlist: Path | None) -> tuple[set[str], set[str]]:
    if allowlist is None:
        return set(), set()
    return read_allowlist(allowlist)


def main(argv: Sequence[str] = ()) -> int:
    arguments = parse_arguments(argv)
    observability = arguments.observability_root
    try:
        if not observability.is_dir():
            raise ConfigurationError(f"{observability}: observability root is not a directory")
        product_metrics, product_histograms = load_product_metrics(arguments.allowlist)
    except ConfigurationError as error:
        print(f"Observability metric-name lint cannot run: {error}", file=sys.stderr)
        return 2

    problems: list[str] = []
    paths = rule_paths(observability, arguments.rules)
    rule_documents = read_rule_documents(paths, problems)
    local_recordings = {
        name for _, content in rule_documents for name in RECORD_NAME.findall(content)
    }
    allowed_metrics = allowed_metric_names(
        product_metrics, product_histograms, local_recordings
    )

    dashboard_paths = sorted((observability / "dashboards").glob("*.json"))
    lint_dashboards(dashboard_paths, allowed_metrics, problems)
    lint_rules(rule_documents, allowed_metrics, problems)

    if problems:
        print("Observability metric-name lint failed:", file=sys.stderr)
        for problem in problems:
            print(f"- {problem}", file=sys.stderr)
        return 1

    print(
        f"Observability metric-name lint passed: "
        f"{len(dashboard_paths)} dashboard(s), {len(paths)} rule file(s), "
        f"{len(local_recordings)} local recording rule(s)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
