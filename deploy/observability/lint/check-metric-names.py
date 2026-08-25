#!/usr/bin/env python3
"""Lint observability expressions against Baukit's telemetry contract."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
OBSERVABILITY = ROOT / "deploy" / "observability"

# Source of truth: docs/platform/telemetry-spec.md, section 2.
SPEC_METRICS = (
    "http_requests_total",
    "http_request_duration_seconds",
    "http_requests_in_flight",
    "http_rate_limit_decisions_total",
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
HISTOGRAM_METRICS = {
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

    visit(document, str(path.relative_to(ROOT)))
    return expressions


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

    for metric in sorted(metric_references(expression)):
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


def main() -> int:
    problems: list[str] = []
    rule_paths = sorted(
        list((OBSERVABILITY / "recording-rules").glob("*.yml"))
        + list((OBSERVABILITY / "recording-rules").glob("*.yaml"))
        + list((OBSERVABILITY / "alerts").glob("*.yml"))
        + list((OBSERVABILITY / "alerts").glob("*.yaml"))
    )
    rule_documents: list[tuple[Path, str]] = []
    local_recordings: set[str] = set()
    for path in rule_paths:
        try:
            content = path.read_text(encoding="utf-8")
        except OSError as error:
            problems.append(f"{path}: cannot read rule file: {error}")
            continue
        rule_documents.append((path, content))
        local_recordings.update(RECORD_NAME.findall(content))

    exposed_metrics = set(SPEC_METRICS)
    for histogram in HISTOGRAM_METRICS:
        exposed_metrics.update(
            {f"{histogram}_bucket", f"{histogram}_count", f"{histogram}_sum"}
        )
    allowed_metrics = exposed_metrics | PROMETHEUS_BUILTINS | local_recordings

    dashboard_paths = sorted((OBSERVABILITY / "dashboards").glob("*.json"))
    for path in dashboard_paths:
        try:
            expressions = dashboard_expressions(path)
        except ValueError as error:
            problems.append(str(error))
            continue
        for location, expression in expressions:
            lint_expression(location, expression, allowed_metrics, problems)

    for path, content in rule_documents:
        relative_path = str(path.relative_to(ROOT))
        for line_number, expression in rule_expressions(content):
            lint_expression(
                f"{relative_path}:{line_number}", expression, allowed_metrics, problems
            )

    if problems:
        print("Observability metric-name lint failed:", file=sys.stderr)
        for problem in problems:
            print(f"- {problem}", file=sys.stderr)
        return 1

    print(
        f"Observability metric-name lint passed: "
        f"{len(dashboard_paths)} dashboard(s), {len(rule_paths)} rule file(s), "
        f"{len(local_recordings)} local recording rule(s)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
