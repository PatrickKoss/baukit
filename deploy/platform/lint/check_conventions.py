#!/usr/bin/env python3
"""Check cross-resource platform conventions on the output validate.sh renders.

Each rule is named. conventions.toml lists the active rules and, for each rule that today's
bases still violate, the plan task that will fix it. A rule in neither list fails, and so
does a pending rule that already passes.
"""

from __future__ import annotations

import argparse
import contextlib
import dataclasses
import fnmatch
import re
import sys
import tarfile
import tomllib
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path

import chart_source
import helm_yaml
import image_digests

LINT_DIR = Path(__file__).resolve().parent
REPO_ROOT = LINT_DIR.parents[2]
CRD_KIND_LINE = re.compile(r"^kind:[ \t]*['\"]?CustomResourceDefinition['\"]?[ \t]*$", re.MULTILINE)
MONITOR_BLOCK_KEYS = {"servicemonitor", "podmonitor"}
MONITOR_FLAG_KEYS = {
    "podmonitorenabled",
    "servicemonitorenabled",
    "enablepodmonitor",
    "enableservicemonitor",
}

PSA_PREFIX = "pod-security.kubernetes.io/"
PSA_LEVELS = {"privileged", "baseline", "restricted"}
SEVERITIES = {"critical", "warning", "info"}
PRUNE_ANNOTATION = "kustomize.toolkit.fluxcd.io/prune"
PRUNE_DISABLED = "disabled"
FORCE_ANNOTATION = "kustomize.toolkit.fluxcd.io/force"
FORCE_ENABLED = "enabled"
WORKLOAD_KINDS = {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job", "CronJob", "Pod"}
MONITOR_KINDS = {"ServiceMonitor", "PodMonitor"}
CNPG_GROUP = "postgresql.cnpg.io/"
MONITORING_GROUP = "monitoring.coreos.com/"
DEFAULT_NAMESPACE = "default"
NAMESPACE_NAME_LABEL = "kubernetes.io/metadata.name"
SIDECAR_ALL_NAMESPACES = "ALL"
CNPG_METRICS_PORT = "metrics"


class ConventionError(Exception):
    pass


@dataclass(frozen=True)
class Resource:
    obj: dict
    namespace: str
    managed_by_kustomize: bool
    release: tuple[str, str] | None = None

    @property
    def kind(self) -> str:
        return str(self.obj.get("kind", ""))

    @property
    def api_version(self) -> str:
        return str(self.obj.get("apiVersion", ""))

    @property
    def name(self) -> str:
        return str(self.metadata.get("name", ""))

    @property
    def metadata(self) -> dict:
        return self.obj.get("metadata") or {}

    @property
    def labels(self) -> dict:
        return self.metadata.get("labels") or {}

    @property
    def annotations(self) -> dict:
        return self.metadata.get("annotations") or {}

    @property
    def spec(self) -> dict:
        return self.obj.get("spec") or {}

    def ref(self) -> str:
        if self.kind == "Namespace":
            return f"Namespace {self.name}"
        return f"{self.kind} {self.namespace}/{self.name}"


@dataclass(frozen=True)
class Alert:
    source: str
    rule: dict
    pack: str = "platform"

    @property
    def name(self) -> str:
        return str(self.rule.get("alert", ""))


@dataclass
class Context:
    universes: dict[str, list[Resource]]
    alerts: list[Alert] = field(default_factory=list)
    fired_alerts: set[str] = field(default_factory=set)
    runbook_slugs: set[str] = field(default_factory=set)
    chart_values: dict[str, dict] = field(default_factory=dict)
    chart_dependencies: dict[str, list[tuple[str, str]]] = field(default_factory=dict)
    scraped_images: dict[str, str] = field(default_factory=dict)
    privileged_namespaces: set[str] = field(default_factory=set)
    network_policy_exempt: set[str] = field(default_factory=set)
    policy_exception_budget: int = 0
    digest_exemptions: image_digests.Exemptions = field(
        default_factory=lambda: image_digests.Exemptions({}, {})
    )
    installs: dict[str, list[Resource]] = field(default_factory=dict)
    chart_sources: dict[chart_source.SourceKey, dict] = field(default_factory=dict)


Rule = Callable[[Context], Iterable[str]]


def by_kind(resources: Iterable[Resource], kind: str) -> list[Resource]:
    return [resource for resource in resources if resource.kind == kind]


def all_resources(context: Context) -> Iterable[tuple[str, Resource]]:
    for universe, resources in context.universes.items():
        for resource in resources:
            yield universe, resource


def namespace_labels(resources: list[Resource]) -> dict[str, dict]:
    labels = {
        resource.name: {**resource.labels, NAMESPACE_NAME_LABEL: resource.name}
        for resource in by_kind(resources, "Namespace")
    }
    for resource in resources:
        labels.setdefault(resource.namespace, {NAMESPACE_NAME_LABEL: resource.namespace})
    return labels


def matches_selector(selector: dict | None, labels: dict) -> bool:
    if selector is None:
        return False
    for key, value in (selector.get("matchLabels") or {}).items():
        if labels.get(key) != value:
            return False
    for expression in selector.get("matchExpressions") or []:
        key = expression.get("key")
        values = expression.get("values") or []
        operator = expression.get("operator")
        present = key in labels
        checks = {
            "In": present and labels.get(key) in values,
            "NotIn": not present or labels.get(key) not in values,
            "Exists": present,
            "DoesNotExist": not present,
        }
        if not checks.get(operator, False):
            return False
    return True


def runs_pods(resource: Resource) -> bool:
    if resource.kind in WORKLOAD_KINDS:
        return True
    return resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP)


def holds_volumes(resource: Resource) -> bool:
    if resource.kind == "PersistentVolumeClaim":
        return True
    if resource.kind == "StatefulSet":
        return bool(resource.spec.get("volumeClaimTemplates"))
    if resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP):
        return True
    if resource.kind in {"Prometheus", "Alertmanager"} and resource.api_version.startswith(
        MONITORING_GROUP
    ):
        return bool((resource.spec.get("storage") or {}).get("volumeClaimTemplate"))
    return False


def rule_psa_labels(context: Context) -> Iterable[str]:
    for _, namespace in all_resources(context):
        if namespace.kind != "Namespace":
            continue
        labels = namespace.labels
        enforce = labels.get(f"{PSA_PREFIX}enforce")
        if enforce not in PSA_LEVELS:
            yield f"Namespace {namespace.name} has no valid {PSA_PREFIX}enforce label"
            continue
        if not labels.get(f"{PSA_PREFIX}enforce-version"):
            yield f"Namespace {namespace.name} has no {PSA_PREFIX}enforce-version label"
        if not any(f"{PSA_PREFIX}{mode}" in labels for mode in ("audit", "warn")):
            yield f"Namespace {namespace.name} has neither a PSA audit nor a warn label"
        if enforce == "privileged" and namespace.name not in context.privileged_namespaces:
            yield f"Namespace {namespace.name} enforces privileged but is not on the allowlist"


def denied_policy_types(policy: Resource) -> set[str]:
    selector = policy.spec.get("podSelector")
    if selector is None or selector.get("matchLabels") or selector.get("matchExpressions"):
        return set()
    denied = set()
    for policy_type in policy.spec.get("policyTypes") or []:
        if not policy.spec.get(policy_type.lower()):
            denied.add(policy_type)
    return denied


TENANT_LABEL = "baukit.dev/tenant"


def rule_default_deny(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        pod_namespaces = {resource.namespace for resource in resources if runs_pods(resource)}
        covered: dict[str, set[str]] = {}
        for policy in by_kind(resources, "NetworkPolicy"):
            covered.setdefault(policy.namespace, set()).update(denied_policy_types(policy))
        # Kyverno's generate-tenant-defaults creates default-deny in every tenant namespace.
        for tenant in by_kind(resources, "Namespace"):
            if tenant.labels.get(TENANT_LABEL) == "true":
                covered.setdefault(tenant.name, set()).update({"Ingress", "Egress"})
        for namespace in sorted(pod_namespaces - context.network_policy_exempt):
            missing = {"Ingress", "Egress"} - covered.get(namespace, set())
            if missing:
                yield (
                    f"{universe}: namespace {namespace} runs pods without a default-deny "
                    f"NetworkPolicy for {', '.join(sorted(missing))}"
                )


def release_chart(
    release: Resource, sources: dict[chart_source.SourceKey, dict]
) -> tuple[str, str]:
    try:
        return chart_source.release_chart(release.obj, sources, release.namespace)
    except chart_source.ChartSourceError as error:
        raise ConventionError(str(error)) from error


def chart_key(release: Resource, sources: dict[chart_source.SourceKey, dict]) -> str:
    name, version = release_chart(release, sources)
    return f"{name}@{version}"


def monitor_toggles(values: object, path: tuple[str, ...] = ()) -> list[str]:
    """Return dotted paths of boolean ServiceMonitor or PodMonitor switches at any depth."""
    if not isinstance(values, dict):
        return []
    found = []
    for key, value in values.items():
        here = (*path, str(key))
        lowered = str(key).lower()
        if isinstance(value, bool) and (
            lowered in MONITOR_FLAG_KEYS
            or (lowered == "enabled" and path and path[-1].lower() in MONITOR_BLOCK_KEYS)
        ):
            found.append(".".join(here))
        found.extend(monitor_toggles(value, here))
    return found


def merged(base: object, override: object) -> object:
    if not isinstance(base, dict) or not isinstance(override, dict):
        return override
    result = dict(base)
    for key, value in override.items():
        result[key] = merged(base.get(key), value) if key in base else value
    return result


def condition_disabled(values: dict, condition: str) -> bool:
    """Helm uses the first condition path that resolves to a boolean."""
    for path in condition.split(","):
        node: object = values
        for part in path.strip().split("."):
            node = node.get(part) if isinstance(node, dict) else None
        if isinstance(node, bool):
            return not node
    return False


def effective_values(release: Resource, context: Context) -> dict | None:
    key = chart_key(release, context.chart_sources)
    defaults = context.chart_values.get(key)
    if defaults is None:
        return None
    values = merged(defaults, release.spec.get("values") or {})
    if not isinstance(values, dict):
        return None
    for name, condition in context.chart_dependencies.get(key, []):
        if condition and condition_disabled(values, condition):
            values.pop(name, None)
    return values


def values_from_config_maps(release: Resource, resources: Iterable[Resource]) -> list[dict]:
    """Return the values of the ConfigMaps in the universe that the release reads via valuesFrom."""
    wanted = {
        (str(ref.get("name")), str(ref.get("valuesKey") or "values.yaml"))
        for ref in release.spec.get("valuesFrom") or []
        if ref.get("kind") == "ConfigMap"
    }
    found = []
    for config_map in by_kind(resources, "ConfigMap"):
        if config_map.namespace != release.namespace:
            continue
        for name, key in wanted:
            text = (config_map.obj.get("data") or {}).get(key)
            if config_map.name == name and isinstance(text, str):
                found.extend(helm_yaml.parse_documents(helm_yaml.split_documents(text)))
    return found


def enabled_by_values_from(release: Resource, resources: Iterable[Resource]) -> bool:
    """A monitor turned on by a valuesFrom ConfigMap renders once that ConfigMap is applied."""
    return any(
        enabled_monitor_toggle(values) for values in values_from_config_maps(release, resources)
    )


def enabled_monitor_toggle(values: object) -> bool:
    toggles = monitor_toggles(values)
    for toggle in toggles:
        node = values
        for part in toggle.split("."):
            node = node.get(part) if isinstance(node, dict) else None
        if node is True:
            return True
    return False


def rule_metrics_enabled(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        monitored = {
            resource.release
            for resource in resources
            if resource.kind in MONITOR_KINDS and resource.release
        }
        for release in by_kind(resources, "HelmRelease"):
            values = effective_values(release, context)
            if values is None:
                key = chart_key(release, context.chart_sources)
                yield f"{universe}: no chart values found for {release.ref()} ({key})"
                continue
            toggles = monitor_toggles(values)
            if (
                toggles
                and release_identity(release) not in monitored
                and not enabled_by_values_from(release, resources)
            ):
                yield (
                    f"{universe}: {release.ref()} chart has a metrics monitor toggle "
                    f"({', '.join(toggles)}), but its render has no ServiceMonitor or PodMonitor"
                )


def pod_labels(resource: Resource) -> dict:
    if resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP):
        return {"cnpg.io/cluster": resource.name, "cnpg.io/podRole": "instance"}
    return ((resource.spec.get("template") or {}).get("metadata") or {}).get("labels") or {}


def container_images(resource: Resource) -> list[str]:
    if resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP):
        return [str(resource.spec.get("imageName", ""))]
    return [str(c.get("image", "")) for c in pod_template(resource).get("containers") or []]


def image_repository(image: str) -> str:
    name = image.split("@", 1)[0]
    if ":" in name.rsplit("/", 1)[-1]:
        name = name.rsplit(":", 1)[0]
    return name.rsplit("/", 1)[-1]


def pod_port_names(resource: Resource) -> set[str]:
    if resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP):
        return {CNPG_METRICS_PORT}
    return {
        str(port.get("name"))
        for container in pod_template(resource).get("containers") or []
        for port in container.get("ports") or []
        if port.get("name")
    }


def selects_pods(monitor: Resource, workload: Resource) -> bool:
    if monitor.kind != "PodMonitor":
        return False
    namespaces = (monitor.spec.get("namespaceSelector") or {}).get("matchNames")
    if workload.namespace not in (namespaces or [monitor.namespace]):
        return False
    ports = {str(e.get("port")) for e in monitor.spec.get("podMetricsEndpoints") or []}
    return bool(ports & pod_port_names(workload)) and matches_selector(
        monitor.spec.get("selector"), pod_labels(workload)
    )


def rule_exporters_scraped(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        monitors = [resource for resource in resources if resource.kind in MONITOR_KINDS]
        for workload in resources:
            if not workload.managed_by_kustomize or not runs_pods(workload):
                continue
            exporters = {
                context.scraped_images[repository]
                for repository in map(image_repository, container_images(workload))
                if repository in context.scraped_images
            }
            if not exporters:
                continue
            if not any(selects_pods(monitor, workload) for monitor in monitors):
                yield (
                    f"{universe}: {workload.ref()} runs {', '.join(sorted(exporters))}, "
                    "but no PodMonitor scrapes a named port of its pods"
                )


def rule_alert_metadata(context: Context) -> Iterable[str]:
    for alert in context.alerts:
        labels = alert.rule.get("labels") or {}
        annotations = alert.rule.get("annotations") or {}
        if labels.get("severity") not in SEVERITIES:
            yield f"{alert.source}: {alert.name} severity is not one of {sorted(SEVERITIES)}"
        for annotation in ("summary", "description", "runbook_url"):
            if not str(annotations.get(annotation, "")).strip():
                yield f"{alert.source}: {alert.name} has no {annotation} annotation"


def runbook_slug(url: str) -> str:
    return url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".md")


def rule_alert_runbook(context: Context) -> Iterable[str]:
    for alert in context.alerts:
        url = str((alert.rule.get("annotations") or {}).get("runbook_url", "")).strip()
        slug = runbook_slug(url)
        if not slug or f"{alert.pack}/{slug}" not in context.runbook_slugs:
            yield (
                f"{alert.source}: {alert.name} runbook slug {slug!r} has no "
                f"docs/runbooks/{alert.pack}/{slug}.md"
            )


def rule_alert_tested(context: Context) -> Iterable[str]:
    for alert in context.alerts:
        if alert.name not in context.fired_alerts:
            yield f"{alert.source}: {alert.name} has no promtool test case that fires it"


def rule_rules_selected(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        prometheuses = by_kind(resources, "Prometheus")
        labels = namespace_labels(resources)
        for rule in by_kind(resources, "PrometheusRule"):
            if not any(selects_rule(prometheus, rule, labels) for prometheus in prometheuses):
                yield f"{universe}: no Prometheus selects {rule.ref()}"


def selects_rule(prometheus: Resource, rule: Resource, labels: dict[str, dict]) -> bool:
    namespace_selector = prometheus.spec.get("ruleNamespaceSelector")
    if namespace_selector is None:
        namespace_ok = rule.namespace == prometheus.namespace
    else:
        namespace_ok = matches_selector(namespace_selector, labels.get(rule.namespace, {}))
    return namespace_ok and matches_selector(prometheus.spec.get("ruleSelector"), rule.labels)


@dataclass(frozen=True)
class Sidecar:
    label: str
    value: str | None
    namespaces: frozenset[str] | None

    def watches(self, resource: Resource) -> bool:
        if self.label not in resource.labels:
            return False
        return self.value is None or str(resource.labels[self.label]) == self.value

    def admits(self, namespace: str) -> bool:
        return self.namespaces is None or namespace in self.namespaces


def pod_template(resource: Resource) -> dict:
    return (resource.spec.get("template") or {}).get("spec") or {}


def grafana_sidecars(resources: list[Resource]) -> list[Sidecar]:
    sidecars = []
    for workload in resources:
        if workload.kind not in WORKLOAD_KINDS:
            continue
        for container in pod_template(workload).get("containers") or []:
            env = {item.get("name"): item.get("value") for item in container.get("env") or []}
            if "LABEL" not in env or "FOLDER" not in env:
                continue
            namespaces = env.get("NAMESPACE") or workload.namespace
            scope = (
                None
                if namespaces == SIDECAR_ALL_NAMESPACES
                else frozenset(part.strip() for part in namespaces.split(","))
            )
            sidecars.append(Sidecar(env["LABEL"], env.get("LABEL_VALUE"), scope))
    return sidecars


def rule_dashboards_selected(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        sidecars = grafana_sidecars(resources)
        labels = {sidecar.label for sidecar in sidecars}
        for config in by_kind(resources, "ConfigMap"):
            watchers = [sidecar for sidecar in sidecars if sidecar.watches(config)]
            if not watchers and labels & set(config.labels):
                yield f"{universe}: {config.ref()} label value matches no Grafana sidecar"
            elif watchers and not any(sidecar.admits(config.namespace) for sidecar in watchers):
                yield f"{universe}: {config.ref()} sits outside every Grafana sidecar namespace"


def rule_policy_exception_budget(context: Context) -> Iterable[str]:
    exceptions = {
        (resource.namespace, resource.name)
        for _, resource in all_resources(context)
        if resource.kind == "PolicyException"
    }
    count = len(exceptions)
    budget = context.policy_exception_budget
    if count > budget:
        yield f"{count} PolicyExceptions exceed the budget of {budget} in budget.toml"
    elif count < budget:
        yield f"{count} PolicyExceptions are below the budget of {budget}; lower budget.toml"


def rule_prune_disabled(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        stateful_namespaces = {
            resource.namespace for resource in resources if holds_volumes(resource)
        }
        for resource in resources:
            if not resource.managed_by_kustomize:
                continue
            needs = resource.kind == "PersistentVolumeClaim" or (
                resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP)
            )
            needs = needs or (resource.kind == "Namespace" and resource.name in stateful_namespaces)
            if needs and resource.annotations.get(PRUNE_ANNOTATION) != PRUNE_DISABLED:
                yield f"{universe}: {resource.ref()} lacks {PRUNE_ANNOTATION}: {PRUNE_DISABLED}"


def rule_flux_job_force(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        for job in by_kind(resources, "Job"):
            if not job.managed_by_kustomize or job.api_version != "batch/v1":
                continue
            if job.annotations.get(FORCE_ANNOTATION) != FORCE_ENABLED:
                yield f"{universe}: {job.ref()} lacks {FORCE_ANNOTATION}: {FORCE_ENABLED}"


def rule_image_digest(context: Context) -> Iterable[str]:
    sources = {
        universe: [(resource.ref(), resource.obj) for resource in resources]
        for universe, resources in {**context.universes, **context.installs}.items()
    }
    return image_digests.problems(sources, context.digest_exemptions)


PRIORITY_CLASSES = {
    "platform-critical",
    "platform-standard",
    "system-cluster-critical",
    "system-node-critical",
}
OPERATOR_POD_NAMES = {
    "Prometheus": "prometheus",
    "Alertmanager": "alertmanager",
    "ThanosRuler": "thanos-ruler",
}
REPLICATED_KINDS = {"Deployment", "StatefulSet", "ReplicaSet"}


def is_cnpg_cluster(resource: Resource) -> bool:
    return resource.kind == "Cluster" and resource.api_version.startswith(CNPG_GROUP)


def is_operator_pod_owner(resource: Resource) -> bool:
    return resource.kind in OPERATOR_POD_NAMES and resource.api_version.startswith(MONITORING_GROUP)


def pod_spec(resource: Resource) -> dict | None:
    if is_cnpg_cluster(resource) or is_operator_pod_owner(resource) or resource.kind == "Pod":
        return resource.spec
    if resource.kind == "CronJob":
        job = (resource.spec.get("jobTemplate") or {}).get("spec") or {}
        return (job.get("template") or {}).get("spec") or {}
    if resource.kind in WORKLOAD_KINDS:
        return pod_template(resource)
    return None


def rule_priority_class(context: Context) -> Iterable[str]:
    for universe, resource in all_resources(context):
        spec = pod_spec(resource)
        if spec is None:
            continue
        priority = spec.get("priorityClassName")
        if priority not in PRIORITY_CLASSES:
            yield (
                f"{universe}: {resource.ref()} has priorityClassName {priority!r}; "
                "use platform-critical or platform-standard"
            )


def replica_count(resource: Resource) -> int:
    if is_cnpg_cluster(resource):
        value = resource.spec.get("instances", 1)
    else:
        value = resource.spec.get("replicas", 1)
    return value if isinstance(value, int) else 1


def disruption_target(resource: Resource) -> bool:
    return resource.kind in REPLICATED_KINDS or is_operator_pod_owner(resource)


def operator_pod_labels(resource: Resource) -> dict:
    if is_operator_pod_owner(resource):
        name = OPERATOR_POD_NAMES[resource.kind]
        return {"app.kubernetes.io/name": name, name: resource.name}
    return pod_labels(resource)


def allowed_disruptions(budget: Resource, replicas: int) -> int:
    spec = budget.spec
    if "maxUnavailable" in spec:
        return absolute(spec["maxUnavailable"], replicas)
    if "minAvailable" in spec:
        return replicas - absolute(spec["minAvailable"], replicas, round_up=True)
    return replicas


def absolute(value: object, replicas: int, *, round_up: bool = False) -> int:
    if isinstance(value, int):
        return value
    text = str(value)
    if not text.endswith("%") or not text[:-1].isdigit():
        return 0
    scaled = replicas * int(text[:-1])
    return -(-scaled // 100) if round_up else scaled // 100


def cnpg_pdb_problems(cluster: Resource) -> Iterable[str]:
    instances = replica_count(cluster)
    enabled = cluster.spec.get("enablePDB", True) is not False
    if enabled and instances == 1:
        yield f"{cluster.ref()} has 1 instance, so its default PDB blocks every drain"
    elif not enabled and instances > 1:
        yield f"{cluster.ref()} runs {instances} instances with enablePDB false"


def budget_problems(workload: Resource, budgets: list[Resource]) -> Iterable[str]:
    replicas = replica_count(workload)
    labels = operator_pod_labels(workload)
    matching = [
        budget
        for budget in budgets
        if budget.namespace == workload.namespace
        and matches_selector(budget.spec.get("selector"), labels)
    ]
    if replicas > 1 and not matching:
        yield f"{workload.ref()} runs {replicas} replicas without a PodDisruptionBudget"
    for budget in matching:
        if replicas > 0 and allowed_disruptions(budget, replicas) < 1:
            yield f"{budget.ref()} blocks every drain of {workload.ref()}"


def rule_pdb_multi_replica(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        budgets = by_kind(resources, "PodDisruptionBudget")
        for resource in resources:
            if is_cnpg_cluster(resource):
                problems = cnpg_pdb_problems(resource)
            elif disruption_target(resource):
                problems = budget_problems(resource, budgets)
            else:
                continue
            yield from (f"{universe}: {problem}" for problem in problems)


QUIET_SEVERITIES = {"info", "none"}


def chart_name(release: Resource) -> str:
    return str(((release.spec.get("chart") or {}).get("spec") or {}).get("chart", ""))


def rule_severities(resources: list[Resource]) -> set[tuple[str, str]]:
    return {
        (str(rule["alert"]), str((rule.get("labels") or {}).get("severity", "")))
        for resource in by_kind(resources, "PrometheusRule")
        for group in resource.spec.get("groups") or []
        for rule in group.get("rules") or []
        if "alert" in rule
    }


def kured_filter_problems(configuration: dict, alerts: set[tuple[str, str]]) -> Iterable[str]:
    if not configuration.get("prometheusUrl"):
        yield "sets no prometheusUrl, so firing alerts do not block reboots"
        return
    if configuration.get("alertFiringOnly") is not True:
        yield "does not set alertFiringOnly, so pending alerts block reboots"
    text = str(configuration.get("alertFilterRegexp") or "")
    try:
        pattern = re.compile(text) if text else None
    except re.error as error:
        yield f"alertFilterRegexp does not compile: {error}"
        return
    for name, severity in sorted(alerts):
        filtered = pattern is not None and pattern.search(name) is not None
        if severity in QUIET_SEVERITIES and not filtered:
            yield f"alertFilterRegexp misses {severity} alert {name}, which blocks every reboot"
        elif severity not in QUIET_SEVERITIES and filtered:
            yield f"alertFilterRegexp ignores {severity} alert {name}"


def rule_kured_alert_filter(context: Context) -> Iterable[str]:
    for universe, resources in context.universes.items():
        alerts = rule_severities(resources)
        for release in by_kind(resources, "HelmRelease"):
            if chart_name(release) != "kured":
                continue
            configuration = (release.spec.get("values") or {}).get("configuration") or {}
            for problem in kured_filter_problems(configuration, alerts):
                yield f"{universe}: {release.ref()} {problem}"


RULES: dict[str, Rule] = {
    "psa-labels": rule_psa_labels,
    "default-deny-network-policy": rule_default_deny,
    "metrics-enabled": rule_metrics_enabled,
    "exporters-scraped": rule_exporters_scraped,
    "alert-metadata": rule_alert_metadata,
    "alert-runbook": rule_alert_runbook,
    "alert-tested": rule_alert_tested,
    "rules-selected": rule_rules_selected,
    "dashboards-selected": rule_dashboards_selected,
    "policy-exception-budget": rule_policy_exception_budget,
    "prune-disabled": rule_prune_disabled,
    "flux-job-force": rule_flux_job_force,
    "image-digest": rule_image_digest,
    "priority-class": rule_priority_class,
    "pdb-multi-replica": rule_pdb_multi_replica,
    "kured-alert-filter": rule_kured_alert_filter,
}


@dataclass(frozen=True)
class Activation:
    active: list[str]
    pending: dict[str, str]


def load_activation(config: dict) -> Activation:
    rules = config.get("rules") or {}
    activation = Activation(list(rules.get("active") or []), dict(rules.get("pending") or {}))
    problems = []
    for name in sorted(set(RULES) - set(activation.active) - set(activation.pending)):
        problems.append(f"rule {name} is neither active nor pending in conventions.toml")
    for name in sorted((set(activation.active) | set(activation.pending)) - set(RULES)):
        problems.append(f"conventions.toml names unknown rule {name}")
    for name in sorted(set(activation.active) & set(activation.pending)):
        problems.append(f"rule {name} is both active and pending")
    if problems:
        raise ConventionError("\n".join(problems))
    return activation


@dataclass
class Outcome:
    failures: list[str]
    summary: str


def evaluate(context: Context, activation: Activation) -> Outcome:
    failures = []
    pending_report = []
    for name in activation.active:
        failures.extend(f"[{name}] {problem}" for problem in sorted(set(RULES[name](context))))
    for name, task in sorted(activation.pending.items()):
        count = len(set(RULES[name](context)))
        if count == 0:
            failures.append(f"[{name}] passes today; move it from pending ({task}) to active")
        pending_report.append(f"{name} ({task}): {count}")
    summary = f"Conventions: {len(activation.active)} active, {len(activation.pending)} pending" + (
        f" [{'; '.join(pending_report)}]" if pending_report else ""
    )
    return Outcome(failures, summary)


def parse_files(paths: list[Path]) -> dict[Path, list[dict]]:
    """Parse each file's non-CRD documents in one helm call."""
    owners: list[Path] = []
    documents: list[str] = []
    for path in paths:
        for document in helm_yaml.split_documents(path.read_text()):
            if document.strip() and not CRD_KIND_LINE.search(document):
                owners.append(path)
                documents.append(document)
    parsed: dict[Path, list[dict]] = {path: [] for path in paths}
    for path, obj in zip(owners, helm_yaml.parse_documents(documents), strict=True):
        if obj:
            parsed[path].append(obj)
    return parsed


def resources_of(
    objects: list[dict],
    *,
    default_namespace: str,
    kustomize: bool,
    release: tuple[str, str] | None = None,
) -> list[Resource]:
    resources = []
    for obj in objects:
        namespace = (obj.get("metadata") or {}).get("namespace") or default_namespace
        resources.append(Resource(obj, namespace, kustomize, release))
    return resources


def release_identity(release: Resource) -> tuple[str, str]:
    return (release.namespace, release.name)


def build_universes(rendered: dict[Path, list[dict]]) -> dict[str, list[Resource]]:
    bases = {path: objs for path, objs in rendered.items() if path.name.startswith("base-")}
    charts = {path.name: objs for path, objs in rendered.items() if path.name.startswith("chart-")}
    overlays = {path: objs for path, objs in rendered.items() if path.name.startswith("overlay-")}

    chart_resources: dict[tuple[str, str], list[Resource]] = {}
    base_resources: list[Resource] = []
    for path, objects in sorted(bases.items()):
        resources = resources_of(objects, default_namespace=DEFAULT_NAMESPACE, kustomize=True)
        base_resources.extend(resources)
        base_name = path.stem.removeprefix("base-")
        for release in by_kind(resources, "HelmRelease"):
            chart_file = f"chart-{base_name}-{release.name}.yaml"
            if chart_file not in charts:
                continue
            target = release.spec.get("targetNamespace") or release.namespace
            chart_resources[release_identity(release)] = resources_of(
                charts.pop(chart_file),
                default_namespace=target,
                kustomize=False,
                release=release_identity(release),
            )
    if charts:
        raise ConventionError(f"chart renders without a HelmRelease: {', '.join(sorted(charts))}")

    def with_charts(resources: list[Resource]) -> list[Resource]:
        extra = []
        for release in by_kind(resources, "HelmRelease"):
            extra.extend(chart_resources.get(release_identity(release), []))
        return resources + extra

    universes = {"bases": with_charts(base_resources)}
    for path, objects in sorted(overlays.items()):
        resources = resources_of(objects, default_namespace=DEFAULT_NAMESPACE, kustomize=True)
        universes[path.stem.replace("-", "s/", 1)] = with_charts(resources)
    return universes


def chart_file_text(
    chart: tuple[str, str], chart_cache: Path, repo_root: Path, file: str = "values.yaml"
) -> str | None:
    name, version = chart
    if name.startswith("./"):
        path = repo_root / name / file
        return path.read_text() if path.is_file() else None
    for archive in sorted(chart_cache.glob("*.tgz")):
        if not fnmatch.fnmatchcase(archive.name, f"{name}-{version}-*.tgz"):
            continue
        with tarfile.open(archive) as bundle, contextlib.suppress(KeyError):
            member = bundle.extractfile(f"{name}/{file}")
            if member is not None:
                return member.read().decode()
    return None


def chart_values_text(chart: tuple[str, str], chart_cache: Path, repo_root: Path) -> str | None:
    return chart_file_text(chart, chart_cache, repo_root)


def dependency_conditions(chart: dict) -> list[tuple[str, str]]:
    return [
        (str(dependency.get("alias") or dependency.get("name", "")), str(dependency["condition"]))
        for dependency in chart.get("dependencies") or []
        if dependency.get("condition")
    ]


def load_alerts(packs: dict[Path, list[dict]], universes: dict[str, list[Resource]]) -> list[Alert]:
    alerts = []
    for path, documents in packs.items():
        for document in documents:
            alerts.extend(alerts_in_groups(str(path), document.get("groups") or [], pack_of(path)))
    for resource in universes.get("bases", []):
        if resource.kind == "PrometheusRule" and resource.managed_by_kustomize:
            alerts.extend(alerts_in_groups(resource.ref(), resource.spec.get("groups") or []))
    return alerts


def pack_of(path: Path) -> str:
    return "app" if "observability" in path.parts else "platform"


def runbook_slugs(runbooks: Path) -> set[str]:
    return {
        path.relative_to(runbooks).with_suffix("").as_posix() for path in runbooks.rglob("*.md")
    }


def alerts_in_groups(source: str, groups: list[dict], pack: str = "platform") -> list[Alert]:
    return [
        Alert(source, rule, pack)
        for group in groups
        for rule in group.get("rules") or []
        if "alert" in rule
    ]


def fired_alerts(test_files: dict[Path, list[dict]]) -> set[str]:
    fired = set()
    for documents in test_files.values():
        for document in documents:
            for test in document.get("tests") or []:
                for case in test.get("alert_rule_test") or []:
                    if case.get("exp_alerts"):
                        fired.add(str(case.get("alertname", "")))
    return fired


def rule_pack_files(repo_root: Path) -> list[Path]:
    return sorted(
        path
        for pattern in (
            "deploy/observability*/alerts/*.y*ml",
            "deploy/observability*/recording-rules/*.y*ml",
        )
        for path in repo_root.glob(pattern)
    )


def promtool_test_files(repo_root: Path) -> list[Path]:
    return sorted(
        path
        for path in (repo_root / "deploy").rglob("tests/*.y*ml")
        if ".helm-cache" not in path.parts
        and "node_modules" not in path.parts
        and "rule_files" in path.read_text()
    )


def build_installs(rendered: dict[Path, list[dict]]) -> dict[str, list[Resource]]:
    return {
        f"installs/{path.stem.removeprefix('install-')}": resources_of(
            objects, default_namespace=DEFAULT_NAMESPACE, kustomize=False
        )
        for path, objects in sorted(rendered.items())
        if path.name.startswith("install-")
    }


def build_context(
    rendered_dir: Path, chart_cache: Path, repo_root: Path, config: dict, budget: dict
) -> Context:
    rendered_paths = sorted(rendered_dir.glob("*.yaml"))
    if not rendered_paths:
        raise ConventionError(f"no rendered manifests in {rendered_dir}")
    packs = rule_pack_files(repo_root)
    tests = promtool_test_files(repo_root)
    parsed = parse_files([*rendered_paths, *packs, *tests])
    universes = build_universes({path: parsed[path] for path in rendered_paths})
    sources = {
        key: obj
        for resources in universes.values()
        for key, obj in chart_source.oci_sources(
            [resource.obj for resource in resources if resource.managed_by_kustomize]
        ).items()
    }
    values_text, chart_text = {}, {}
    for _, resource in all_resources(Context(universes)):
        if resource.kind == "HelmRelease":
            chart = release_chart(resource, sources)
            text = chart_values_text(chart, chart_cache, repo_root)
            if text is not None:
                values_text[chart_key(resource, sources)] = text
                chart_text[chart_key(resource, sources)] = (
                    chart_file_text(chart, chart_cache, repo_root, "Chart.yaml") or "{}"
                )
    keys = sorted(values_text)
    chart_documents = helm_yaml.parse_documents(
        [values_text[key] for key in keys] + [chart_text[key] for key in keys]
    )
    chart_values = dict(zip(keys, chart_documents[: len(keys)], strict=True))
    charts = dict(zip(keys, chart_documents[len(keys) :], strict=True))
    return Context(
        universes=universes,
        alerts=load_alerts({path: parsed[path] for path in packs}, universes),
        fired_alerts=fired_alerts({path: parsed[path] for path in tests}),
        runbook_slugs=runbook_slugs(repo_root / "docs/runbooks"),
        chart_values=chart_values,
        chart_dependencies={
            key: dependency_conditions(chart or {}) for key, chart in charts.items()
        },
        scraped_images=dict(config.get("scraped_images") or {}),
        privileged_namespaces=set(config.get("privileged_namespaces") or []),
        network_policy_exempt=set(config.get("network_policy_exempt_namespaces") or []),
        policy_exception_budget=int(budget.get("policy_exceptions", 0)),
        installs=build_installs({path: parsed[path] for path in rendered_paths}),
        chart_sources=sources,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--chart-cache", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    parser.add_argument("--config", type=Path, default=LINT_DIR / "conventions.toml")
    parser.add_argument("--budget", type=Path, default=LINT_DIR / "budget.toml")
    parser.add_argument("--digest-exempt", type=Path, default=LINT_DIR / "image-digest-exempt.toml")
    args = parser.parse_args(argv)
    try:
        config = tomllib.loads(args.config.read_text())
        activation = load_activation(config)
        budget = tomllib.loads(args.budget.read_text())
        exemptions = image_digests.load_exemptions(args.digest_exempt)
        context = dataclasses.replace(
            build_context(args.rendered, args.chart_cache, args.repo_root, config, budget),
            digest_exemptions=exemptions,
        )
    except (
        ConventionError,
        helm_yaml.YamlError,
        image_digests.ExemptionError,
        OSError,
        tomllib.TOMLDecodeError,
    ) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    outcome = evaluate(context, activation)
    for failure in outcome.failures:
        print(f"ERROR: {failure}", file=sys.stderr)
    print(outcome.summary)
    return 1 if outcome.failures else 0


if __name__ == "__main__":
    sys.exit(main())
