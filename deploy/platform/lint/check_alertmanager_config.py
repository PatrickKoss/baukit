#!/usr/bin/env python3
"""Fail when Alertmanager routing can drop alerts silently.

The kube-prometheus-stack chart render must read its config from the routing Secret and mount that
Secret, so a cluster without it never starts Alertmanager. The base must not ship the Secret. Every
routing config (the base example and each overlay Secret) must route Watchdog first to a receiver
with an integration, send nothing else to a receiver without one (InfoInhibitor excepted), and carry
the critical-over-warning and InfoInhibitor inhibit rules. The Flux error Alert must cover every
namespace that holds a Kustomization, HelmRelease or GitRepository.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import re
import sys
from collections.abc import Iterable, Iterator
from dataclasses import dataclass
from pathlib import Path

import helm_yaml

PLATFORM_DIR = Path(__file__).resolve().parents[1]
EXAMPLE = PLATFORM_DIR / "observability-stack/alertmanager/alertmanager.example.yaml"
SECRET_NAME = "alertmanager-routing"
SECRET_NAMESPACE = "observability"
CONFIG_KEY = "alertmanager.yaml"
CHART_RENDER = "chart-observability-stack-kube-prometheus-stack.yaml"
BASE_RENDER = "base-observability-stack.yaml"
REQUIRED_OVERLAYS = ("overlays/local/components/observability",)
KUSTOMIZE = PLATFORM_DIR.parents[1] / ".tools/bin/kustomize"
CHART_CONFIG_SECRET = "alertmanager-kube-prometheus-stack-alertmanager"
FLUX_ALERT = ("flux-system", "flux-errors")
FLUX_KINDS = ("Kustomization", "HelmRelease", "GitRepository")
WATCHDOG = ("alertname", "=", "Watchdog")
INFO_INHIBITOR = ("alertname", "=", "InfoInhibitor")
MATCHER = re.compile(r'^\s*([A-Za-z_][A-Za-z0-9_]*)\s*(=~|!~|!=|=)\s*"?(.*?)"?\s*$')
RELEVANT_KINDS = re.compile(r"^kind: (Alertmanager|Secret|Alert|Kustomization|HelmRelease|GitRepository)$", re.M)

Matcher = tuple[str, str, str]


@dataclass(frozen=True)
class RenderedObject:
    kind: str
    namespace: str
    name: str
    body: dict


def parse_matchers(entries: Iterable[str]) -> set[Matcher]:
    matchers = set()
    for entry in entries:
        match = MATCHER.match(str(entry))
        if match is None:
            raise ValueError(f"cannot parse matcher {entry!r}")
        matchers.add((match.group(1), match.group(2), match.group(3)))
    return matchers


def route_matchers(route: dict) -> set[Matcher]:
    matchers = parse_matchers(route.get("matchers") or [])
    matchers.update((label, "=", str(value)) for label, value in (route.get("match") or {}).items())
    matchers.update((label, "=~", str(value)) for label, value in (route.get("match_re") or {}).items())
    return matchers


def selects(matchers: set[Matcher], label: str, value: str) -> bool:
    for name, operator, pattern in matchers:
        if name != label:
            continue
        if operator == "=" and pattern == value:
            return True
        if operator == "=~" and re.fullmatch(pattern, value):
            return True
    return False


def delivering_receivers(config: dict) -> tuple[set[str], set[str]]:
    receivers = config.get("receivers") or []
    names = {str(receiver.get("name")) for receiver in receivers}
    delivering = {
        str(receiver.get("name"))
        for receiver in receivers
        if any(key.endswith("_configs") and value for key, value in receiver.items())
    }
    return names, delivering


def walk_routes(route: dict, inherited: str, path: str) -> Iterator[tuple[str, str, dict]]:
    receiver = str(route.get("receiver") or inherited)
    yield path, receiver, route
    for index, child in enumerate(route.get("routes") or []):
        yield from walk_routes(child, receiver, f"{path}.routes[{index}]")


def route_problems(config: dict, source: str) -> list[str]:
    root = config.get("route")
    if not isinstance(root, dict):
        return [f"{source}: has no route"]
    names, delivering = delivering_receivers(config)
    problems = []
    for path, receiver, route in walk_routes(root, "", "route"):
        if receiver not in names:
            problems.append(f"{source}: {path} uses undefined receiver {receiver!r}")
        elif receiver not in delivering and route_matchers(route) != {INFO_INHIBITOR}:
            problems.append(
                f"{source}: {path} sends alerts to receiver {receiver!r}, which has no integration"
            )
    routes = root.get("routes") or []
    first = routes[0] if routes else {}
    watchdog_receiver = str(first.get("receiver") or root.get("receiver"))
    if WATCHDOG not in route_matchers(first):
        problems.append(f'{source}: route.routes[0] must match alertname = "Watchdog"')
    elif watchdog_receiver not in delivering:
        problems.append(f"{source}: Watchdog goes to {watchdog_receiver!r}, which has no integration")
    return problems


def inhibits(rule: dict, source: tuple[str, str], target: tuple[str, str], equal: set[str]) -> bool:
    source_matchers = parse_matchers(rule.get("source_matchers") or [])
    target_matchers = parse_matchers(rule.get("target_matchers") or [])
    return (
        selects(source_matchers, *source)
        and selects(target_matchers, *target)
        and equal <= set(rule.get("equal") or [])
    )


def inhibit_problems(config: dict, source: str) -> list[str]:
    rules = config.get("inhibit_rules") or []
    problems = []
    wanted = (
        (("severity", "critical"), ("severity", "warning"), {"alertname", "namespace"},
         "critical inhibits warning for the same alertname and namespace"),
        (("alertname", "InfoInhibitor"), ("severity", "info"), {"namespace"},
         "InfoInhibitor inhibits info for the same namespace"),
    )
    for source_label, target_label, equal, description in wanted:
        if not any(inhibits(rule, source_label, target_label, equal) for rule in rules):
            problems.append(f"{source}: no inhibit rule where {description}")
    return problems


def config_problems(config: dict, source: str) -> list[str]:
    try:
        return route_problems(config, source) + inhibit_problems(config, source)
    except ValueError as error:
        return [f"{source}: {error}"]


def rendered_objects(path: Path) -> list[RenderedObject]:
    return parse_objects(path.read_text())


def parse_objects(text: str) -> list[RenderedObject]:
    candidates = [doc for doc in helm_yaml.split_documents(text) if RELEVANT_KINDS.search(doc)]
    objects = []
    for document in helm_yaml.parse_documents(candidates):
        metadata = document.get("metadata") or {}
        objects.append(
            RenderedObject(
                kind=str(document.get("kind", "")),
                namespace=str(metadata.get("namespace", "")),
                name=str(metadata.get("name", "")),
                body=document,
            )
        )
    return objects


def find(objects: list[RenderedObject], kind: str, namespace: str | None, name: str) -> RenderedObject | None:
    for item in objects:
        if item.kind == kind and item.name == name and namespace in (None, item.namespace):
            return item
    return None


def secret_config(secret: RenderedObject) -> str:
    string_data = secret.body.get("stringData") or {}
    if CONFIG_KEY in string_data:
        return str(string_data[CONFIG_KEY])
    data = secret.body.get("data") or {}
    if CONFIG_KEY not in data:
        raise ValueError(f"has no {CONFIG_KEY} key; the operator would fall back to a drop-all config")
    try:
        return base64.b64decode("".join(str(data[CONFIG_KEY]).split()), validate=True).decode()
    except (binascii.Error, UnicodeDecodeError) as error:
        raise ValueError(f"{CONFIG_KEY} is not valid base64 UTF-8: {error}") from error


def parse_config(text: str) -> dict:
    [config] = helm_yaml.parse_documents([text])
    return config


def chart_problems(objects: list[RenderedObject], source: str) -> list[str]:
    alertmanagers = [item for item in objects if item.kind == "Alertmanager"]
    if not alertmanagers:
        return [f"{source}: renders no Alertmanager"]
    problems = []
    for alertmanager in alertmanagers:
        spec = alertmanager.body.get("spec") or {}
        if spec.get("configSecret") != SECRET_NAME:
            problems.append(f"{source}: Alertmanager {alertmanager.name} configSecret is not {SECRET_NAME}")
        if SECRET_NAME not in (spec.get("secrets") or []):
            problems.append(
                f"{source}: Alertmanager {alertmanager.name} does not mount Secret {SECRET_NAME}, "
                "so it starts with the operator's drop-all config when the Secret is missing"
            )
    for name in (SECRET_NAME, CHART_CONFIG_SECRET):
        if find(objects, "Secret", None, name):
            problems.append(f"{source}: renders Secret {name}; the overlay must own the routing config")
    return problems


def overlay_problems(objects: list[RenderedObject], source: str, required: bool) -> list[str]:
    secret = find(objects, "Secret", SECRET_NAMESPACE, SECRET_NAME)
    if secret is None:
        return [f"{source}: has no Secret {SECRET_NAMESPACE}/{SECRET_NAME}"] if required else []
    try:
        config = parse_config(secret_config(secret))
    except ValueError as error:
        return [f"{source}: Secret {SECRET_NAME} {error}"]
    return config_problems(config, f"{source} Secret {SECRET_NAME}")


def flux_alert_problems(objects: list[RenderedObject], alert: RenderedObject | None) -> list[str]:
    if alert is None:
        return [f"no Flux Alert {FLUX_ALERT[0]}/{FLUX_ALERT[1]} is rendered"]
    spec = alert.body.get("spec") or {}
    problems = []
    if spec.get("eventSeverity") != "error":
        problems.append(f"Flux Alert {alert.name} eventSeverity is not error")
    covered = {
        (str(source.get("kind")), str(source.get("namespace") or alert.namespace))
        for source in spec.get("eventSources") or []
        if source.get("name") == "*"
    }
    wanted = {(item.kind, item.namespace) for item in objects if item.kind in FLUX_KINDS}
    for kind, namespace in sorted(wanted - covered):
        problems.append(f"Flux Alert {alert.name} has no eventSource for {kind} in namespace {namespace}")
    return problems


def check(
    rendered_dir: Path, example: Path, overlays: dict[str, str], required: Iterable[str]
) -> list[str]:
    problems = config_problems(parse_config(example.read_text()), str(example))
    problems += chart_problems(rendered_objects(rendered_dir / CHART_RENDER), CHART_RENDER)
    base = rendered_objects(rendered_dir / BASE_RENDER)
    if find(base, "Secret", None, SECRET_NAME):
        problems.append(f"{BASE_RENDER}: renders Secret {SECRET_NAME}; the overlay must own it")
    problems += [f"{name} was not rendered" for name in sorted(set(required) - set(overlays))]
    flux_objects = list(base)
    for name, text in sorted(overlays.items()):
        objects = parse_objects(text)
        flux_objects += objects
        problems += overlay_problems(objects, name, name in required)
    for render in sorted(rendered_dir.glob("*.yaml")):
        if render.name != BASE_RENDER and not render.name.startswith("chart-"):
            flux_objects += rendered_objects(render)
    problems += flux_alert_problems(flux_objects, find(base, "Alert", *FLUX_ALERT))
    return problems


def overlay_directories(platform: Path) -> list[Path]:
    return sorted(
        path.parent
        for pattern in ("overlays/*/kustomization.yaml", "overlays/*/components/*/kustomization.yaml")
        for path in platform.glob(pattern)
    )


def build_overlays(kustomize: Path, platform: Path) -> dict[str, str]:
    renders = {}
    for directory in overlay_directories(platform):
        result = helm_yaml.process_runner.run_process(
            [str(kustomize), "build", str(directory)], capture_output=True, text=True
        )
        if result.returncode != 0:
            raise OSError(f"kustomize build {directory} failed:\n{result.stderr.strip()}")
        renders[directory.relative_to(platform).as_posix()] = result.stdout
    return renders


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--example", type=Path, default=EXAMPLE)
    parser.add_argument("--kustomize", type=Path, default=KUSTOMIZE)
    args = parser.parse_args(argv)
    try:
        overlays = build_overlays(args.kustomize, PLATFORM_DIR)
        problems = check(args.rendered, args.example, overlays, REQUIRED_OVERLAYS)
    except (helm_yaml.YamlError, OSError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for problem in problems:
        print(f"ERROR: {problem}", file=sys.stderr)
    if not problems:
        print("alertmanager routing: Secret required, Watchdog, inhibit rules and Flux alerts in place")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
