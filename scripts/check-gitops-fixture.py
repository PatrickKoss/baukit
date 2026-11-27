#!/usr/bin/env python3
"""Renders a generated product's deploy/gitops overlays and the chart with their HelmRelease values.

Usage: scripts/check-gitops-fixture.py <product root>
Needs kustomize and helm on PATH.
"""

from __future__ import annotations

import sys
import tempfile
import tomllib
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy" / "chart" / "baukit-app"
ENVIRONMENTS = ("testing", "production")
# Production runs two API and worker pods so a node drain keeps one serving.
# Every other Deployment runs one.
SCALED_PROCESSES = ("api", "worker")
SCALED_REPLICAS = {"testing": 1, "production": 2}
ARGUMENT_COUNT = 2
USAGE_EXIT = 2


def output_of(command: list[str]) -> str:
    return run_process(command, check=True, capture_output=True, text=True).stdout


def kustomize(directory: Path) -> list[dict]:
    output = output_of(["kustomize", "build", str(directory)])
    return [document for document in yaml.safe_load_all(output) if document]


def helm_template(app: str, values: dict) -> list[dict]:
    with tempfile.NamedTemporaryFile("w", suffix=".yaml") as handle:
        yaml.safe_dump(values, handle)
        handle.flush()
        output_of(["helm", "lint", "--strict", str(CHART), "-f", handle.name])
        output = output_of(["helm", "template", app, str(CHART), "-f", handle.name])
    return [document for document in yaml.safe_load_all(output) if document]


def host(app: str, environment: str, domain: str) -> str:
    return f"{app}-testing.{domain}" if environment == "testing" else f"{app}.{domain}"


def check_route_tls(route: dict, environment: str, domain: str) -> list[str]:
    expected = [{"main": domain, "sans": [f"*.{domain}"]}]
    tls = route["spec"].get("tls")
    if not tls:
        return [f"{environment}: tls is empty; Flux server-side apply nulls it"]
    if tls.get("domains") != expected:
        return [f"{environment}: IngressRoute tls.domains is {tls.get('domains')}, want {expected}"]
    return []


def check_environment(root: Path, manifest: dict, environment: str) -> list[str]:
    app = manifest["app"]["name"]
    domain = manifest["gitops"]["domain"]
    expected_host = host(app, environment, domain)
    documents = kustomize(root / "deploy" / "gitops" / environment)
    kinds = {document["kind"] for document in documents}
    errors: list[str] = []
    if "Certificate" in kinds:
        errors.append(f"{environment}: renders a Certificate; TLS comes from the default TLSStore")
    for document in documents:
        if document["metadata"].get("namespace") != f"{app}-{environment}":
            errors.append(f"{environment}: {document['kind']} is outside {app}-{environment}")
    route = next(document for document in documents if document["kind"] == "IngressRoute")
    if route["spec"]["routes"][0]["match"] != f"Host(`{expected_host}`)":
        errors.append(f"{environment}: route matches {route['spec']['routes'][0]['match']}")
    errors.extend(check_route_tls(route, environment, domain))
    release = next(document for document in documents if document["kind"] == "HelmRelease")
    values = release["spec"]["values"]
    if values["deploymentEnvironment"] != environment:
        errors.append(f"{environment}: deploymentEnvironment is {values['deploymentEnvironment']}")
    mcp = manifest["capabilities"].get("mcp", False)
    if values.get("mcp", {}).get("enabled", False) != mcp:
        errors.append(f"{environment}: mcp.enabled differs from capabilities.mcp")
    if mcp and values["mcp"]["resourceUrl"] != f"https://{expected_host}/mcp":
        errors.append(f"{environment}: mcp.resourceUrl is {values['mcp']['resourceUrl']}")
    service = route["spec"]["routes"][0]["services"][0]["name"]
    rendered = helm_template(app, values)
    services = {
        document["metadata"]["name"] for document in rendered if document["kind"] == "Service"
    }
    if service not in services:
        errors.append(
            f"{environment}: route targets Service {service}, chart renders {sorted(services)}"
        )
    errors.extend(check_replicas(app, rendered, environment))
    return errors


def expected_replicas(app: str, name: str, environment: str) -> int:
    scaled = {f"{app}-{process}" for process in SCALED_PROCESSES}
    return SCALED_REPLICAS[environment] if name in scaled else 1


def check_replicas(app: str, rendered: list[dict], environment: str) -> list[str]:
    errors = []
    budgets = {
        document["metadata"]["name"]
        for document in rendered
        if document["kind"] == "PodDisruptionBudget"
    }
    for document in rendered:
        if document["kind"] == "HorizontalPodAutoscaler":
            errors.append(f"{environment}: renders an HPA")
        if document["kind"] != "Deployment":
            continue
        name = document["metadata"]["name"]
        replicas = document["spec"].get("replicas", 1)
        expected = expected_replicas(app, name, environment)
        if replicas != expected:
            errors.append(f"{environment}: {name} runs {replicas} replicas, expected {expected}")
        if expected > 1 and name not in budgets:
            errors.append(f"{environment}: {name} runs {replicas} replicas without a PDB")
    return errors


def main() -> int:
    if len(sys.argv) != ARGUMENT_COUNT:
        print(__doc__, file=sys.stderr)
        return USAGE_EXIT
    root = Path(sys.argv[1])
    manifest = tomllib.loads((root / "baukit.toml").read_text(encoding="utf-8"))
    errors = [
        error
        for environment in ENVIRONMENTS
        for error in check_environment(root, manifest, environment)
    ]
    for error in errors:
        print(f"gitops fixture: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
