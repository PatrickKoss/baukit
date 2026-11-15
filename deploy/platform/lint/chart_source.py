"""Resolve the chart name and version a HelmRelease installs, from spec.chart or spec.chartRef."""

from __future__ import annotations

from collections.abc import Iterable, Mapping

OCI_KIND = "OCIRepository"
SourceKey = tuple[str, str]


class ChartSourceError(Exception):
    pass


def oci_sources(
    objects: Iterable[dict], default_namespace: str = "default"
) -> dict[SourceKey, dict]:
    sources = {}
    for obj in objects:
        if obj.get("kind") != OCI_KIND:
            continue
        metadata = obj.get("metadata") or {}
        namespace = metadata.get("namespace") or default_namespace
        sources[(namespace, str(metadata.get("name", "")))] = obj
    return sources


def oci_chart(source: dict) -> tuple[str, str]:
    spec = source.get("spec") or {}
    url = str(spec.get("url", ""))
    return url.rstrip("/").rsplit("/", 1)[-1], str((spec.get("ref") or {}).get("tag", ""))


def release_chart(
    release: dict, sources: Mapping[SourceKey, dict], default_namespace: str = "default"
) -> tuple[str, str]:
    spec = release.get("spec") or {}
    chart_ref = spec.get("chartRef")
    if not chart_ref:
        chart = (spec.get("chart") or {}).get("spec") or {}
        return str(chart.get("chart", "")), str(chart.get("version", ""))
    metadata = release.get("metadata") or {}
    name = metadata.get("name", "")
    if chart_ref.get("kind") != OCI_KIND:
        kind = chart_ref.get("kind")
        raise ChartSourceError(f"HelmRelease {name} chartRef kind {kind} is not {OCI_KIND}")
    namespace = chart_ref.get("namespace") or metadata.get("namespace") or default_namespace
    source = sources.get((namespace, str(chart_ref.get("name", ""))))
    if source is None:
        raise ChartSourceError(
            f"HelmRelease {name} references missing {OCI_KIND} {namespace}/{chart_ref.get('name')}"
        )
    return oci_chart(source)
