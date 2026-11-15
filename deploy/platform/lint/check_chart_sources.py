#!/usr/bin/env python3
"""Check every HelmRelease chart source against the chart sources table in deploy/platform/README.md.

A chart the table lists as OCI must come from an OCIRepository through spec.chartRef, at the URL in
the table, and the OCIRepository is named after the chart. A chart the table lists with cosign
verification must carry spec.verify with that exact OIDC identity. A HelmRepository row needs a
reason, and every row must match a HelmRelease.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

import chart_source
import helm_yaml

LINT_DIR = Path(__file__).resolve().parent
README = LINT_DIR.parent / "README.md"
TABLE_HEADING = "## Chart sources"
COLUMNS = ("Chart", "Source", "Signed", "Flux verify", "Identity", "Reason")
CODE_SPAN = re.compile(r"`([^`]+)`")
FLUX_KINDS = re.compile(r"^kind: (HelmRelease|HelmRepository|OCIRepository)$", re.MULTILINE)
OCI = "OCI"
HELM_REPOSITORY = "HelmRepository"
COSIGN = "cosign"
NO_VERIFY = "none"


class TableError(Exception):
    pass


@dataclass(frozen=True)
class Row:
    chart: str
    source: str
    url: str
    verify: str
    identity: tuple[str, ...]
    reason: str


def cells(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def table_lines(text: str) -> list[str]:
    _, found, rest = text.partition(f"\n{TABLE_HEADING}\n")
    if not found:
        raise TableError(f"no '{TABLE_HEADING}' section")
    lines = []
    for line in rest.splitlines():
        if line.startswith("|"):
            lines.append(line)
        elif lines or line.startswith("#"):
            break
    if len(lines) < 3:
        raise TableError(f"'{TABLE_HEADING}' has no table rows")
    return lines


def parse_row(header: list[str], line: str) -> Row:
    values = dict(zip(header, cells(line), strict=True))
    source, *spans = values["Source"].split(" ", 1)
    url = CODE_SPAN.findall(spans[0])[0] if spans and CODE_SPAN.search(spans[0]) else ""
    row = Row(
        chart=values["Chart"].strip("`"),
        source=source,
        url=url,
        verify=values["Flux verify"],
        identity=tuple(CODE_SPAN.findall(values["Identity"])),
        reason=values["Reason"],
    )
    if row.source not in (OCI, HELM_REPOSITORY) or not row.url:
        raise TableError(f"{row.chart}: Source must be 'OCI `oci://...`' or 'HelmRepository `https://...`'")
    if row.verify not in (COSIGN, NO_VERIFY):
        raise TableError(f"{row.chart}: Flux verify must be '{COSIGN}' or '{NO_VERIFY}'")
    if row.verify == COSIGN and len(row.identity) != 2:
        raise TableError(f"{row.chart}: cosign rows name the issuer and subject as two code spans")
    return row


def parse_table(text: str) -> dict[str, Row]:
    lines = table_lines(text)
    header = cells(lines[0])
    if tuple(header) != COLUMNS:
        raise TableError(f"'{TABLE_HEADING}' columns must be {' | '.join(COLUMNS)}")
    rows = [parse_row(header, line) for line in lines[2:]]
    table = {row.chart: row for row in rows}
    if len(table) != len(rows):
        raise TableError("a chart is listed twice")
    return table


def flux_objects(rendered: Path) -> list[dict]:
    documents = []
    for base in sorted(rendered.glob("base-*.yaml")):
        documents.extend(
            document
            for document in helm_yaml.split_documents(base.read_text())
            if FLUX_KINDS.search(document)
        )
    return helm_yaml.parse_documents(documents)


def verify_errors(name: str, source: dict, row: Row) -> list[str]:
    verify = (source.get("spec") or {}).get("verify")
    if row.verify == NO_VERIFY:
        return [f"{name}: README marks {row.chart} unverified, but its source sets verify"] if verify else []
    if not verify or verify.get("provider") != COSIGN:
        return [f"{name}: {row.chart} is cosign-signed, but its OCIRepository has no cosign verify"]
    identities = {
        (str(item.get("issuer", "")), str(item.get("subject", "")))
        for item in verify.get("matchOIDCIdentity") or []
    }
    if identities != {row.identity}:
        return [f"{name}: verify.matchOIDCIdentity is not the README identity {row.identity}"]
    return []


def release_errors(release: dict, sources: dict, row: Row | None, chart: str) -> list[str]:
    metadata = release.get("metadata") or {}
    name = f"HelmRelease {metadata.get('namespace')}/{metadata.get('name')}"
    if row is None:
        return [f"{name}: chart {chart} has no row in the README chart sources table"]
    spec = release.get("spec") or {}
    if row.source == HELM_REPOSITORY:
        if spec.get("chartRef"):
            return [f"{name}: README lists {chart} as HelmRepository, but the release uses chartRef"]
        return [] if row.reason else [f"{name}: README gives no reason why {chart} is not OCI"]
    if "chartRef" not in spec:
        return [f"{name}: {chart} is published as OCI, use spec.chartRef to an OCIRepository"]
    ref = spec["chartRef"]
    source = sources[(ref.get("namespace") or metadata.get("namespace") or "default", ref["name"])]
    errors = []
    if ref["name"] != chart:
        errors.append(f"{name}: OCIRepository {ref['name']} must be named after its chart {chart}")
    if (source.get("spec") or {}).get("url") != row.url:
        errors.append(f"{name}: OCIRepository url is not the README url {row.url}")
    return errors + verify_errors(name, source, row)


def check(objects: list[dict], table: dict[str, Row]) -> list[str]:
    sources = chart_source.oci_sources(objects)
    errors, used = [], set()
    for release in objects:
        if release.get("kind") != "HelmRelease":
            continue
        try:
            chart, _ = chart_source.release_chart(release, sources)
        except chart_source.ChartSourceError as error:
            errors.append(str(error))
            continue
        if chart.startswith("./"):
            continue
        used.add(chart)
        errors.extend(release_errors(release, sources, table.get(chart), chart))
    errors.extend(f"README chart sources row {chart} matches no HelmRelease" for chart in sorted(set(table) - used))
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--readme", type=Path, default=README)
    args = parser.parse_args(argv)
    try:
        errors = check(flux_objects(args.rendered), parse_table(args.readme.read_text()))
    except (TableError, helm_yaml.YamlError, OSError, KeyError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    if not errors:
        print("Chart sources match the README table.")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
