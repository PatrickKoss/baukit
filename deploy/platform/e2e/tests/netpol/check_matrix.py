#!/usr/bin/env python3
"""Fail when a probe matrix row names a namespace, label set, Service or port the e2e cluster lacks.

The e2e inventory is every path in e2e/overlay/reconciliation.yaml built with kustomize, the chart
renders of its HelmReleases from validate.sh, the Flux install and the namespaces the netpol suite
creates. Objects that operators create at runtime are derived from their custom resources (CNPG
Cluster) or, for pod labels, accepted when a NetworkPolicy selects them. The suite's own tenant
namespaces have no workloads, so their probe labels must be ones a cross-namespace peer selects.

The live probe cannot tell a denied 169.254.169.254 from a missing listener, so the check also fails
when a rendered NetworkPolicy or the tenant defaults allow egress into 169.254.0.0/16.
"""

from __future__ import annotations

import argparse
import ipaddress
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

import netpol_matrix

helm_yaml = netpol_matrix.helm_yaml
SUITE = Path(__file__).resolve().parent
E2E = SUITE.parents[1]
RECONCILIATION = E2E / "overlay/reconciliation.yaml"
CHAINSAW_TEST = SUITE / "chainsaw-test.yaml"
TEMPLATE_KINDS = {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
NAME_LABEL = "kubernetes.io/metadata.name"
TENANT_LABEL = "baukit.dev/tenant"
CNPG_PORTS = frozenset({5432, 8000, 9187})
CNPG_SERVICE_SUFFIXES = ("rw", "ro", "r")
LINK_LOCAL = ipaddress.ip_network("169.254.0.0/16")
TENANT_DEFAULTS = E2E.parent / "kyverno-policies/tests/generate-tenant-defaults/generated.yaml"
K3S_NAMESPACES = {"kube-system", "default"}
K3S_PODS = [("kube-system", {"k8s-app": "kube-dns"}, frozenset({53, 9153}))]


@dataclass
class Inventory:
    namespaces: dict[str, dict] = field(default_factory=dict)
    templates: list[tuple[str, dict, frozenset[int]]] = field(default_factory=list)
    selectors: list[tuple[str, dict, frozenset[int]]] = field(default_factory=list)
    scoped: list[tuple[dict, dict]] = field(default_factory=list)
    synthetic: set[str] = field(default_factory=set)
    services: dict[tuple[str, str], set[int]] = field(default_factory=dict)
    policies: list[tuple[str, str, dict]] = field(default_factory=list)

    def add(self, obj: dict, namespace: str) -> None:
        kind = obj.get("kind")
        metadata = obj.get("metadata") or {}
        namespace = metadata.get("namespace") or namespace
        if kind == "Namespace":
            labels = {NAME_LABEL: metadata["name"], **(metadata.get("labels") or {})}
            self.namespaces.setdefault(metadata["name"], {}).update(labels)
        elif kind == "Service":
            ports = {item["port"] for item in (obj.get("spec") or {}).get("ports") or []}
            self.services.setdefault((namespace, metadata["name"]), set()).update(ports)
        elif kind == "NetworkPolicy":
            self.add_policy(obj, namespace)
        elif kind == "Cluster" and obj.get("apiVersion", "").startswith("postgresql.cnpg.io/"):
            self.add_cnpg_cluster(metadata["name"], namespace)
        else:
            template = pod_template(obj)
            if template is not None:
                self.templates.append((namespace, *template))

    def add_policy(self, policy: dict, namespace: str) -> None:
        spec = policy.get("spec") or {}
        self.policies.append((namespace, (policy.get("metadata") or {}).get("name", ""), spec))
        own = (spec.get("podSelector") or {}).get("matchLabels") or {}
        ports = frozenset(
            item["port"]
            for rule in spec.get("ingress") or []
            for item in rule.get("ports") or []
            if isinstance(item.get("port"), int)
        )
        if own:
            self.selectors.append((namespace, own, ports))
        for rule in [*(spec.get("ingress") or []), *(spec.get("egress") or [])]:
            for peer in [*(rule.get("from") or []), *(rule.get("to") or [])]:
                self.add_peer(peer, namespace)

    def add_peer(self, peer: dict, namespace: str) -> None:
        labels = (peer.get("podSelector") or {}).get("matchLabels") or {}
        if not labels:
            return
        if "namespaceSelector" not in peer:
            self.selectors.append((namespace, labels, frozenset()))
            return
        namespace_labels = (peer.get("namespaceSelector") or {}).get("matchLabels") or {}
        self.scoped.append((namespace_labels, labels))

    def add_cnpg_cluster(self, name: str, namespace: str) -> None:
        for suffix in CNPG_SERVICE_SUFFIXES:
            self.services.setdefault((namespace, f"{name}-{suffix}"), set()).add(5432)
        labels = {"cnpg.io/cluster": name, "cnpg.io/podRole": "instance"}
        self.templates.append((namespace, labels, CNPG_PORTS))

    def labels_exist(self, namespace: str, labels: dict) -> bool:
        if not labels:
            return True
        return any(
            ns == namespace and subset(labels, template) for ns, template, _ in self.templates
        ) or any(subset(chosen, labels) for chosen in self.selectors_in(namespace))

    def selectors_in(self, namespace: str) -> list[dict]:
        if namespace in self.synthetic:
            return [chosen for _, chosen in self.scoped]
        found = [chosen for ns, chosen, _ in self.selectors if ns == namespace]
        namespace_labels = self.namespaces.get(namespace, {})
        found += [chosen for scope, chosen in self.scoped if subset(scope, namespace_labels)]
        return found

    def pod_ports(self, namespace: str, labels: dict) -> set[int]:
        ports: set[int] = set()
        for ns, template, template_ports in self.templates:
            if ns == namespace and subset(labels, template):
                ports |= template_ports
        for ns, chosen, policy_ports in self.selectors:
            if ns == namespace and subset(chosen, labels):
                ports |= policy_ports
        return ports


def subset(small: dict, large: dict) -> bool:
    return all(large.get(key) == value for key, value in small.items())


def pod_template(obj: dict) -> tuple[dict, frozenset[int]] | None:
    kind = obj.get("kind")
    spec = obj.get("spec") or {}
    if kind in TEMPLATE_KINDS:
        template = spec.get("template") or {}
    elif kind == "CronJob":
        template = ((spec.get("jobTemplate") or {}).get("spec") or {}).get("template") or {}
    elif kind == "Pod":
        template = obj
    else:
        return None
    labels = (template.get("metadata") or {}).get("labels") or {}
    pod_spec = template.get("spec") or {}
    containers = [*(pod_spec.get("containers") or []), *(pod_spec.get("initContainers") or [])]
    ports = frozenset(
        port["containerPort"] for container in containers for port in container.get("ports") or []
    )
    return labels, ports


def parse(text: str) -> list[dict]:
    return [obj for obj in helm_yaml.parse_documents(helm_yaml.split_documents(text)) if obj]


def overlay_paths(reconciliation: Path) -> list[Path]:
    paths = re.findall(r"^  path: \./(\S+)$", reconciliation.read_text(), re.MULTILINE)
    return sorted({netpol_matrix.REPO / path for path in paths})


def chart_renders(rendered: Path) -> dict[tuple[str, str], Path]:
    renders = {}
    for base in sorted(rendered.glob("base-*.yaml")):
        base_name = base.stem.removeprefix("base-")
        for obj in parse(base.read_text()):
            if obj.get("kind") != "HelmRelease":
                continue
            metadata = obj["metadata"]
            key = (metadata.get("namespace", ""), metadata["name"])
            renders[key] = rendered / f"chart-{base_name}-{metadata['name']}.yaml"
    return renders


def suite_namespaces(chainsaw_test: Path) -> list[dict]:
    test = parse(chainsaw_test.read_text())[0]
    found = []
    for step in test.get("spec", {}).get("steps") or []:
        for operation in step.get("try") or []:
            resource = (operation.get("apply") or {}).get("resource") or {}
            if resource.get("kind") == "Namespace":
                found.append(resource)
    return found


def build(kustomize: str, path: Path) -> str:
    result = helm_yaml.process_runner.run_process(
        [kustomize, "build", str(path)], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise netpol_matrix.MatrixError(f"kustomize build {path} failed:\n{result.stderr.strip()}")
    return result.stdout


def inventory(rendered: Path, kustomize: str, paths: list[Path], chainsaw_test: Path) -> Inventory:
    found = Inventory(namespaces={name: {NAME_LABEL: name} for name in K3S_NAMESPACES})
    for namespace, labels, ports in K3S_PODS:
        found.templates.append((namespace, labels, ports))
    renders = chart_renders(rendered)
    for path in paths:
        for obj in parse(build(kustomize, path)):
            found.add(obj, "")
            if obj.get("kind") == "HelmRelease":
                add_release(found, obj, renders)
    for obj in parse((rendered / "install-flux-lockdown.yaml").read_text()):
        found.add(obj, "flux-system")
    for namespace in suite_namespaces(chainsaw_test):
        found.add(namespace, "")
        found.synthetic.add(namespace["metadata"]["name"])
    return found


def add_release(found: Inventory, release: dict, renders: dict[tuple[str, str], Path]) -> None:
    metadata = release["metadata"]
    render = renders.get((metadata.get("namespace", ""), metadata["name"]))
    if render is None or not render.is_file():
        return
    target = (release.get("spec") or {}).get("targetNamespace") or metadata.get("namespace", "")
    for obj in parse(render.read_text()):
        found.add(obj, target)


def row_problems(row: netpol_matrix.Row, found: Inventory) -> list[str]:
    where = row.describe()
    problems = []
    if row.namespace not in found.namespaces:
        problems.append(f"{where}: namespace {row.namespace} is not in the e2e cluster")
    elif not found.labels_exist(row.namespace, row.labels):
        problems.append(f"{where}: no pod in {row.namespace} carries the source labels")
    if row.kind == "service":
        name, namespace = row.target.split(".")
        ports = found.services.get((namespace, name))
        if ports is None:
            problems.append(f"{where}: Service {namespace}/{name} is not in the e2e cluster")
        elif row.port not in ports:
            problems.append(f"{where}: Service {namespace}/{name} has no port {row.port}")
    elif row.kind == "pod":
        if row.pod_namespace not in found.namespaces:
            problems.append(f"{where}: namespace {row.pod_namespace} is not in the e2e cluster")
        elif not found.labels_exist(row.pod_namespace, row.pod_labels):
            problems.append(
                f"{where}: no pod in {row.pod_namespace} carries the destination labels"
            )
        elif row.port not in found.pod_ports(row.pod_namespace, row.pod_labels):
            problems.append(f"{where}: the destination pods expose no port {row.port}")
    return problems


def suite_problems(chainsaw_test: Path) -> list[str]:
    problems = []
    for namespace in suite_namespaces(chainsaw_test):
        metadata = namespace.get("metadata") or {}
        if (metadata.get("labels") or {}).get(TENANT_LABEL) != "true":
            problems.append(f"suite namespace {metadata.get('name')} lacks {TENANT_LABEL}=true")
    return problems


def opens_link_local(peer: dict) -> bool:
    block = peer.get("ipBlock")
    if block is None:
        return False
    cidr = ipaddress.ip_network(block["cidr"], strict=False)
    if cidr.version != LINK_LOCAL.version or not cidr.overlaps(LINK_LOCAL):
        return False
    excluded = (ipaddress.ip_network(item, strict=False) for item in block.get("except") or [])
    return not any(
        item.version == LINK_LOCAL.version and LINK_LOCAL.subnet_of(item) for item in excluded
    )


def metadata_problems(policies: list[tuple[str, str, dict]]) -> list[str]:
    problems = []
    for namespace, name, spec in policies:
        for index, rule in enumerate(spec.get("egress") or []):
            peers = rule.get("to") or []
            if not peers or any(opens_link_local(peer) for peer in peers):
                problems.append(
                    f"NetworkPolicy {namespace}/{name} egress rule {index} allows "
                    f"{LINK_LOCAL}, which holds the metadata endpoint"
                )
    return problems


def tenant_policies(tenant_defaults: Path) -> list[tuple[str, str, dict]]:
    found = Inventory()
    for obj in parse(tenant_defaults.read_text()):
        if obj.get("kind") == "NetworkPolicy":
            found.add_policy(obj, "<tenant>")
    return found.policies


@dataclass(frozen=True)
class Sources:
    rendered: Path
    kustomize: str
    paths: list[Path]
    chainsaw_test: Path = CHAINSAW_TEST
    tenant_defaults: Path = TENANT_DEFAULTS


def problems(sources: Sources, matrix: Path) -> list[str]:
    rows = netpol_matrix.load(matrix)
    found = inventory(sources.rendered, sources.kustomize, sources.paths, sources.chainsaw_test)
    result = suite_problems(sources.chainsaw_test)
    result += metadata_problems(found.policies + tenant_policies(sources.tenant_defaults))
    for row in rows:
        result += row_problems(row, found)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--kustomize", required=True)
    parser.add_argument("--matrix", type=Path, default=SUITE / "matrix.yaml")
    parser.add_argument("--reconciliation", type=Path, default=RECONCILIATION)
    parser.add_argument("--chainsaw-test", type=Path, default=CHAINSAW_TEST)
    parser.add_argument("--tenant-defaults", type=Path, default=TENANT_DEFAULTS)
    parser.add_argument(
        "--extra-path",
        type=Path,
        action="append",
        default=[],
        help="another kustomization to inventory, such as a base the e2e overlay leaves out",
    )
    args = parser.parse_args(argv)
    paths = overlay_paths(args.reconciliation) + [path.resolve() for path in args.extra_path]
    try:
        sources = Sources(
            args.rendered, args.kustomize, paths, args.chainsaw_test, args.tenant_defaults
        )
        found = problems(sources, args.matrix)
    except (netpol_matrix.MatrixError, helm_yaml.YamlError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for problem in found:
        print(f"ERROR: {problem}", file=sys.stderr)
    if not found:
        print(f"netpol matrix: every row of {args.matrix.name} resolves in the inventory")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
