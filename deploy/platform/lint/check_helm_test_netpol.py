#!/usr/bin/env python3
"""Fail when NetworkPolicies block the Helm test pods of a HelmRelease that runs its tests.

For each HelmRelease with spec.test.enable in a rendered base, the chart's helm.sh/hook test pods
are read from validate.sh's chart render. A test pod connects to each chart Service whose name and
port appear in its command line or in a chart ConfigMap it mounts; `http://<service>` with no port
means port 80. The pod's egress and the Service backends' ingress must both be
admitted by the NetworkPolicies of the base, or by no policy at all for that direction.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import helm_yaml

WORKLOAD_KINDS = {"Deployment", "StatefulSet", "DaemonSet"}
TEST_HOOKS = {"test", "test-success"}
HTTP_PORT = 80


def documents(path: Path) -> list[dict]:
    return helm_yaml.parse_documents(helm_yaml.split_documents(path.read_text()))


def selects(selector: dict, labels: dict) -> bool:
    for key, value in (selector.get("matchLabels") or {}).items():
        if labels.get(key) != value:
            return False
    return all(expression_holds(item, labels) for item in selector.get("matchExpressions") or [])


def expression_holds(expression: dict, labels: dict) -> bool:
    key, operator = expression["key"], expression["operator"]
    values = expression.get("values") or []
    if operator == "In":
        return labels.get(key) in values
    if operator == "NotIn":
        return labels.get(key) not in values
    if operator == "Exists":
        return key in labels
    return key not in labels


def is_test_pod(doc: dict) -> bool:
    if doc.get("kind") != "Pod":
        return False
    hooks = (doc["metadata"].get("annotations") or {}).get("helm.sh/hook", "")
    return bool(TEST_HOOKS & {hook.strip() for hook in hooks.split(",")})


def command_line(pod: dict) -> str:
    parts = []
    for container in pod["spec"].get("containers", []):
        parts += container.get("command") or []
        parts += container.get("args") or []
    return " ".join(str(part) for part in parts)


def mounted_scripts(pod: dict, chart: list[dict]) -> str:
    names = {
        (volume.get("configMap") or {}).get("name") for volume in pod["spec"].get("volumes") or []
    }
    return " ".join(
        str(value)
        for doc in chart
        if doc.get("kind") == "ConfigMap" and doc["metadata"].get("name") in names
        for value in (doc.get("data") or {}).values()
    )


def names_port(line: str, service: str, port: int) -> bool:
    if mentions(line, str(port)):
        return True
    url = re.search(rf"http://{re.escape(service)}(?![\w.:-])", line)
    return port == HTTP_PORT and url is not None


def backends(service: dict, chart: list[dict]) -> list[tuple[dict, dict]]:
    selector = service["spec"].get("selector") or {}
    found = []
    for doc in chart:
        if doc.get("kind") not in WORKLOAD_KINDS:
            continue
        template = doc["spec"]["template"]
        labels = template["metadata"].get("labels") or {}
        if selector and selects({"matchLabels": selector}, labels):
            found.append((labels, template["spec"]))
    return found


def target_port(port: dict, pod_spec: dict) -> int | None:
    target = port.get("targetPort", port["port"])
    if isinstance(target, int):
        return target
    for container in pod_spec.get("containers", []):
        for container_port in container.get("ports") or []:
            if container_port.get("name") == target:
                return container_port["containerPort"]
    return None


def mentions(line: str, token: str) -> bool:
    return re.search(rf"(?<![\w.-]){re.escape(token)}(?![\w-])", line) is not None


def destinations(pod: dict, chart: list[dict]) -> list[tuple[str, dict, int]]:
    line = f"{command_line(pod)} {mounted_scripts(pod, chart)}"
    found = []
    for service in (doc for doc in chart if doc.get("kind") == "Service"):
        name = service["metadata"]["name"]
        if not mentions(line, name):
            continue
        for port in service["spec"].get("ports", []):
            if not names_port(line, name, port["port"]):
                continue
            for labels, pod_spec in backends(service, chart):
                number = target_port(port, pod_spec)
                if number is not None:
                    found.append((name, labels, number))
    return found


def admits(rules: list[dict], peer_key: str, peer_labels: dict, port: int) -> bool:
    for rule in rules:
        if not port_matches(rule.get("ports"), port):
            continue
        peers = rule.get(peer_key)
        if not peers or any(peer_matches(peer, peer_labels) for peer in peers):
            return True
    return False


def port_matches(ports: list[dict] | None, port: int) -> bool:
    return not ports or any(item.get("port") in (None, port) for item in ports)


def peer_matches(peer: dict, labels: dict) -> bool:
    if "ipBlock" in peer or "namespaceSelector" in peer:
        return False
    return selects(peer.get("podSelector") or {}, labels)


def allowed(policies: list[dict], direction: str, pod: dict, peer: dict, port: int) -> bool:
    key, peer_key = ("egress", "to") if direction == "Egress" else ("ingress", "from")
    applying = [
        policy
        for policy in policies
        if direction in policy["spec"].get("policyTypes", ["Ingress"])
        and selects(policy["spec"].get("podSelector") or {}, pod)
    ]
    if not applying:
        return True
    return any(admits(policy["spec"].get(key) or [], peer_key, peer, port) for policy in applying)


def release_problems(release: dict, base: list[dict], chart: list[dict]) -> list[str]:
    namespace = release["spec"].get("targetNamespace") or release["metadata"].get("namespace")
    policies = [
        doc
        for doc in base
        if doc.get("kind") == "NetworkPolicy" and doc["metadata"].get("namespace") == namespace
    ]
    excluded = {
        item["name"]
        for item in (release["spec"].get("test") or {}).get("filters") or []
        if item.get("exclude")
    }
    problems = []
    for pod in filter(is_test_pod, chart):
        name = pod["metadata"]["name"]
        if name in excluded:
            continue
        labels = pod["metadata"].get("labels") or {}
        targets = destinations(pod, chart)
        if not targets:
            problems.append(f"{namespace}/{name}: no chart Service found in the test command")
        for service, backend, port in targets:
            if not allowed(policies, "Egress", labels, backend, port):
                problems.append(f"{namespace}/{name}: egress to {service}:{port} is not allowed")
            if not allowed(policies, "Ingress", backend, labels, port):
                problems.append(f"{namespace}/{name}: {service} does not admit it on {port}")
    return problems


def problems(rendered: Path) -> list[str]:
    found = []
    for base_file in sorted(rendered.glob("base-*.yaml")):
        base = documents(base_file)
        base_name = base_file.stem.removeprefix("base-")
        for release in base:
            if release.get("kind") != "HelmRelease":
                continue
            if not (release["spec"].get("test") or {}).get("enable"):
                continue
            chart_file = rendered / f"chart-{base_name}-{release['metadata']['name']}.yaml"
            if not chart_file.is_file():
                found.append(f"{base_name}: no chart render {chart_file.name}")
                continue
            found += release_problems(release, base, documents(chart_file))
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    args = parser.parse_args(argv)
    found = problems(args.rendered)
    for problem in found:
        print(f"ERROR: {problem}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
