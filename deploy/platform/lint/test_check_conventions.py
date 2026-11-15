from __future__ import annotations

import contextlib
import io
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path

import check_conventions as cc

PSA = "pod-security.kubernetes.io/"


def resource(kind: str, name: str, namespace: str = "app", *, kustomize: bool = True, **fields):
    metadata = {"name": name}
    for key in ("labels", "annotations"):
        if key in fields:
            metadata[key] = fields.pop(key)
    obj = {"apiVersion": fields.pop("apiVersion", "v1"), "kind": kind, "metadata": metadata}
    obj.update(fields)
    return cc.Resource(obj, namespace, kustomize)


def namespace(name: str, labels: dict | None = None, annotations: dict | None = None):
    return resource(
        "Namespace", name, "default", labels=labels or {}, annotations=annotations or {}
    )


def context(*resources: cc.Resource, **fields) -> cc.Context:
    return cc.Context(universes={"bases": list(resources)}, **fields)


def violations(rule: str, ctx: cc.Context) -> list[str]:
    return list(cc.RULES[rule](ctx))


def deny_all(namespace_name: str, types: list[str]) -> cc.Resource:
    return resource(
        "NetworkPolicy",
        "default-deny",
        namespace_name,
        apiVersion="networking.k8s.io/v1",
        spec={"podSelector": {}, "policyTypes": types},
    )


def alert(name: str, pack: str = "platform", **overrides) -> cc.Alert:
    rule = {
        "alert": name,
        "labels": {"severity": "warning"},
        "annotations": {
            "summary": "s",
            "description": "d",
            "runbook_url": f"https://runbooks.example/platform/{name.lower()}",
        },
    }
    rule.update(overrides)
    return cc.Alert("pack.yml", rule, pack)


class PsaLabelsTest(unittest.TestCase):
    def test_restricted_namespace_with_audit_passes(self) -> None:
        labels = {
            f"{PSA}enforce": "restricted",
            f"{PSA}audit": "restricted",
            f"{PSA}enforce-version": "v1.33",
        }
        self.assertEqual(violations("psa-labels", context(namespace("app", labels))), [])

    def test_missing_enforce_fails(self) -> None:
        found = violations("psa-labels", context(namespace("app", {f"{PSA}warn": "restricted"})))
        self.assertEqual(len(found), 1)
        self.assertIn("enforce", found[0])

    def test_missing_audit_and_warn_fails(self) -> None:
        found = violations("psa-labels", context(namespace("app", {f"{PSA}enforce": "baseline"})))
        self.assertIn("neither a PSA audit nor a warn", found[-1])

    def test_missing_enforce_version_fails(self) -> None:
        labels = {f"{PSA}enforce": "restricted", f"{PSA}audit": "restricted"}
        found = violations("psa-labels", context(namespace("app", labels)))
        self.assertEqual(found, [f"Namespace app has no {PSA}enforce-version label"])

    def test_privileged_needs_the_allowlist(self) -> None:
        labels = {f"{PSA}enforce": "privileged", f"{PSA}warn": "restricted"}
        labels[f"{PSA}enforce-version"] = "v1.33"
        ctx = context(namespace("app", labels), namespace("kured", labels))
        ctx.privileged_namespaces = {"kured"}
        found = violations("psa-labels", ctx)
        self.assertEqual(len(found), 1)
        self.assertIn("Namespace app enforces privileged", found[0])


class DefaultDenyTest(unittest.TestCase):
    def test_namespace_running_pods_without_policy_fails(self) -> None:
        found = violations("default-deny-network-policy", context(resource("Deployment", "api")))
        self.assertIn("for Egress, Ingress", found[0])

    def test_ingress_only_policy_misses_egress(self) -> None:
        ctx = context(resource("Deployment", "api"), deny_all("app", ["Ingress"]))
        found = violations("default-deny-network-policy", ctx)
        self.assertIn("for Egress", found[0])

    def test_policy_with_allow_rules_is_not_a_default_deny(self) -> None:
        allow = deny_all("app", ["Ingress", "Egress"])
        allow.obj["spec"]["ingress"] = [{"from": [{"podSelector": {}}]}]
        found = violations("default-deny-network-policy", context(resource("Pod", "p"), allow))
        self.assertIn("for Ingress", found[0])

    def test_scoped_pod_selector_is_not_a_default_deny(self) -> None:
        scoped = deny_all("app", ["Ingress", "Egress"])
        scoped.obj["spec"]["podSelector"] = {"matchLabels": {"app": "x"}}
        self.assertTrue(
            violations("default-deny-network-policy", context(resource("Pod", "p"), scoped))
        )

    def test_deny_both_directions_passes(self) -> None:
        ctx = context(
            resource("Cluster", "db", apiVersion="postgresql.cnpg.io/v1"),
            deny_all("app", ["Ingress"]),
            deny_all("app", ["Egress"]),
        )
        self.assertEqual(violations("default-deny-network-policy", ctx), [])

    def test_exempt_namespace_passes(self) -> None:
        ctx = context(resource("DaemonSet", "ccm", "kube-system"))
        ctx.network_policy_exempt = {"kube-system"}
        self.assertEqual(violations("default-deny-network-policy", ctx), [])


def helm_release(name: str, chart: str, version: str, namespace_name: str = "app") -> cc.Resource:
    spec = {"chart": {"spec": {"chart": chart, "version": version}}}
    return resource(
        "HelmRelease", name, namespace_name, apiVersion="helm.toolkit.fluxcd.io/v2", spec=spec
    )


class MetricsEnabledTest(unittest.TestCase):
    def test_chart_with_toggle_and_no_monitor_fails(self) -> None:
        ctx = context(helm_release("x", "x", "1.0.0"))
        ctx.chart_values = {"x@1.0.0": {"metrics": {"serviceMonitor": {"enabled": False}}}}
        self.assertIn("monitor toggle", violations("metrics-enabled", ctx)[0])

    def test_chart_with_toggle_and_a_monitor_passes(self) -> None:
        monitor = resource(
            "PodMonitor", "x", kustomize=False, apiVersion="monitoring.coreos.com/v1"
        )
        monitor = cc.Resource(monitor.obj, "x", False, ("x", "x"))
        ctx = context(helm_release("x", "x", "1.0.0", "x"), monitor)
        ctx.chart_values = {"x@1.0.0": {"monitoring": {"podMonitorEnabled": True}}}
        self.assertEqual(violations("metrics-enabled", ctx), [])

    def test_monitor_from_another_release_does_not_count(self) -> None:
        other = resource("PodMonitor", "y", "x", apiVersion="monitoring.coreos.com/v1")
        ctx = context(helm_release("x", "x", "1.0.0", "x"), other)
        ctx.chart_values = {"x@1.0.0": {"monitoring": {"podMonitorEnabled": False}}}
        self.assertIn("monitoring.podMonitorEnabled", violations("metrics-enabled", ctx)[0])

    def test_release_values_override_chart_defaults(self) -> None:
        release = helm_release("x", "x", "1.0.0")
        release.spec["values"] = {"sub": {"enabled": False}}
        ctx = context(release)
        ctx.chart_values = {"x@1.0.0": {"sub": {"serviceMonitor": {"enabled": False}}}}
        ctx.chart_dependencies = {"x@1.0.0": [("sub", "sub.enabled")]}
        self.assertEqual(violations("metrics-enabled", ctx), [])
        release.spec["values"] = {"sub": {"enabled": True}}
        self.assertIn("sub.serviceMonitor.enabled", violations("metrics-enabled", ctx)[0])

    def test_condition_uses_the_first_path_that_resolves(self) -> None:
        values = {"a": {"enabled": True}, "b": {"enabled": False}}
        self.assertFalse(cc.condition_disabled(values, "missing.enabled,a.enabled,b.enabled"))
        self.assertTrue(cc.condition_disabled(values, "b.enabled,a.enabled"))
        self.assertFalse(cc.condition_disabled(values, "missing.enabled"))

    def test_dependency_conditions_use_the_alias(self) -> None:
        chart = {
            "dependencies": [
                {"name": "loki", "condition": "loki.enabled"},
                {"name": "redis", "alias": "cache", "condition": "cache.enabled"},
                {"name": "always"},
            ]
        }
        self.assertEqual(
            cc.dependency_conditions(chart), [("loki", "loki.enabled"), ("cache", "cache.enabled")]
        )

    def test_chart_without_toggle_passes(self) -> None:
        ctx = context(helm_release("x", "x", "1.0.0"))
        ctx.chart_values = {"x@1.0.0": {"replicas": 1, "serviceMonitor": {"labels": {}}}}
        self.assertEqual(violations("metrics-enabled", ctx), [])

    def test_known_toggle_paths_are_found_at_any_depth(self) -> None:
        values = {
            "serviceMonitor": {"enabled": False},
            "prometheus": {"servicemonitor": {"enabled": False}},
            "a": {"metrics": {"serviceMonitor": {"enabled": True}}},
            "monitoring": {"podMonitorEnabled": False, "enabled": False},
        }
        self.assertEqual(
            cc.monitor_toggles(values),
            [
                "serviceMonitor.enabled",
                "prometheus.servicemonitor.enabled",
                "a.metrics.serviceMonitor.enabled",
                "monitoring.podMonitorEnabled",
            ],
        )

    def test_unknown_chart_values_fail(self) -> None:
        self.assertIn(
            "no chart values",
            violations("metrics-enabled", context(helm_release("x", "x", "1")))[0],
        )


def redis_deployment(ports: list[dict]) -> cc.Resource:
    container = {"name": "redis", "image": "docker.io/library/redis:8.10.2-alpine@sha256:ab"}
    sidecar = {"name": "redis-exporter", "image": "docker.io/oliver006/redis_exporter:v1"}
    template = {
        "metadata": {"labels": {"app": "redis"}},
        "spec": {"containers": [container, {**sidecar, "ports": ports}]},
    }
    return resource(
        "Deployment", "redis", "cache", apiVersion="apps/v1", spec={"template": template}
    )


def pod_monitor(namespace_name: str, match: dict, port: str = "metrics", **spec) -> cc.Resource:
    spec = {"selector": {"matchLabels": match}, "podMetricsEndpoints": [{"port": port}], **spec}
    return resource(
        "PodMonitor", "m", namespace_name, apiVersion="monitoring.coreos.com/v1", spec=spec
    )


class ExportersScrapedTest(unittest.TestCase):
    SCRAPED = {"redis": "Redis", "postgresql": "PostgreSQL"}

    def check(self, *resources: cc.Resource) -> list[str]:
        return violations("exporters-scraped", context(*resources, scraped_images=self.SCRAPED))

    def test_redis_without_a_pod_monitor_fails(self) -> None:
        found = self.check(redis_deployment([{"name": "metrics", "containerPort": 9121}]))
        self.assertIn("Deployment cache/redis runs Redis", found[0])

    def test_redis_with_exporter_and_pod_monitor_passes(self) -> None:
        workload = redis_deployment([{"name": "metrics", "containerPort": 9121}])
        self.assertEqual(self.check(workload, pod_monitor("cache", {"app": "redis"})), [])

    def test_pod_monitor_on_a_port_the_pod_lacks_fails(self) -> None:
        self.assertEqual(
            len(self.check(redis_deployment([]), pod_monitor("cache", {"app": "redis"}))), 1
        )

    def test_pod_monitor_in_another_namespace_needs_match_names(self) -> None:
        workload = redis_deployment([{"name": "metrics", "containerPort": 9121}])
        elsewhere = pod_monitor("observability", {"app": "redis"})
        self.assertEqual(len(self.check(workload, elsewhere)), 1)
        across = pod_monitor(
            "observability", {"app": "redis"}, namespaceSelector={"matchNames": ["cache"]}
        )
        self.assertEqual(self.check(workload, across), [])

    def test_cnpg_cluster_needs_a_pod_monitor_on_its_instances(self) -> None:
        cluster = resource(
            "Cluster",
            "pg",
            "db",
            apiVersion="postgresql.cnpg.io/v1",
            spec={"imageName": "ghcr.io/cloudnative-pg/postgresql:18.6-system-trixie"},
        )
        self.assertIn("Cluster db/pg runs PostgreSQL", self.check(cluster)[0])
        wrong = pod_monitor("db", {"cnpg.io/cluster": "other"})
        self.assertEqual(len(self.check(cluster, wrong)), 1)
        right = pod_monitor("db", {"cnpg.io/cluster": "pg", "cnpg.io/podRole": "instance"})
        self.assertEqual(self.check(cluster, right), [])

    def test_chart_rendered_workloads_are_left_to_metrics_enabled(self) -> None:
        workload = redis_deployment([])
        chart_owned = cc.Resource(workload.obj, "cache", False, ("cache", "x"))
        self.assertEqual(self.check(chart_owned), [])

    def test_image_repository_ignores_registry_tag_and_digest(self) -> None:
        for image in ("redis", "redis:8", "docker.io/library/redis:8@sha256:ab", "host:5000/redis"):
            self.assertEqual(cc.image_repository(image), "redis", image)


class AlertRulesTest(unittest.TestCase):
    def test_complete_alert_passes(self) -> None:
        ctx = context(alerts=[alert("Ok")], fired_alerts={"Ok"}, runbook_slugs={"platform/ok"})
        for rule in ("alert-metadata", "alert-runbook", "alert-tested"):
            self.assertEqual(violations(rule, ctx), [], rule)

    def test_bad_severity_and_missing_annotation_fail(self) -> None:
        bad = alert("Bad", labels={"severity": "page"}, annotations={"summary": "s"})
        found = violations("alert-metadata", context(alerts=[bad]))
        self.assertEqual(len(found), 3)
        self.assertIn("severity", found[0])

    def test_runbook_slug_without_file_fails(self) -> None:
        found = violations(
            "alert-runbook", context(alerts=[alert("Gone")], runbook_slugs={"platform/other"})
        )
        self.assertIn("'gone'", found[0])

    def test_runbook_in_the_other_pack_does_not_count(self) -> None:
        ctx = context(alerts=[alert("Same", pack="app")], runbook_slugs={"platform/same"})
        found = violations("alert-runbook", ctx)
        self.assertIn("docs/runbooks/app/same.md", found[0])
        ctx = context(alerts=[alert("Same", pack="app")], runbook_slugs={"app/same"})
        self.assertEqual(violations("alert-runbook", ctx), [])

    def test_pack_follows_the_chart_directory(self) -> None:
        self.assertEqual(cc.pack_of(Path("deploy/observability/alerts/a.yml")), "app")
        self.assertEqual(cc.pack_of(Path("deploy/observability-platform/alerts/a.yml")), "platform")

    def test_alert_without_firing_test_fails(self) -> None:
        found = violations("alert-tested", context(alerts=[alert("Quiet")], fired_alerts=set()))
        self.assertIn("Quiet has no promtool test", found[0])

    def test_only_cases_with_expected_alerts_count_as_firing(self) -> None:
        tests = {
            Path("t.yml"): [
                {
                    "tests": [
                        {
                            "alert_rule_test": [
                                {"alertname": "Quiet", "exp_alerts": []},
                                {"alertname": "Loud", "exp_alerts": [{"exp_labels": {}}]},
                            ]
                        }
                    ]
                }
            ]
        }
        self.assertEqual(cc.fired_alerts(tests), {"Loud"})


def prometheus(rule_namespace_selector: dict | None, rule_selector: dict | None = None):
    spec = {"ruleSelector": {} if rule_selector is None else rule_selector}
    if rule_namespace_selector is not None:
        spec["ruleNamespaceSelector"] = rule_namespace_selector
    return resource(
        "Prometheus",
        "kps",
        "observability",
        kustomize=False,
        apiVersion="monitoring.coreos.com/v1",
        spec=spec,
    )


def prometheus_rule(namespace_name: str, labels: dict | None = None) -> cc.Resource:
    return resource(
        "PrometheusRule",
        "r",
        namespace_name,
        apiVersion="monitoring.coreos.com/v1",
        labels=labels or {},
    )


class RulesSelectedTest(unittest.TestCase):
    def test_rule_outside_the_namespace_selector_fails(self) -> None:
        selector = {"matchLabels": {"kubernetes.io/metadata.name": "observability"}}
        ctx = context(prometheus(selector), prometheus_rule("postgres"))
        self.assertIn(
            "no Prometheus selects PrometheusRule postgres/r", violations("rules-selected", ctx)[0]
        )

    def test_rule_in_a_labelled_namespace_passes(self) -> None:
        selector = {"matchLabels": {"app.kubernetes.io/part-of": "baukit-platform"}}
        ns = namespace("postgres", {"app.kubernetes.io/part-of": "baukit-platform"})
        ctx = context(prometheus(selector), ns, prometheus_rule("postgres"))
        self.assertEqual(violations("rules-selected", ctx), [])

    def test_nil_namespace_selector_admits_only_the_own_namespace(self) -> None:
        ctx = context(
            prometheus(None), prometheus_rule("observability"), prometheus_rule("postgres")
        )
        self.assertEqual(len(violations("rules-selected", ctx)), 1)

    def test_rule_selector_must_match_rule_labels(self) -> None:
        ctx = context(
            prometheus({}, {"matchExpressions": [{"key": "release", "operator": "Exists"}]}),
            prometheus_rule("observability"),
        )
        self.assertEqual(len(violations("rules-selected", ctx)), 1)


def grafana(namespace_env: str | None) -> cc.Resource:
    env = [{"name": "LABEL", "value": "grafana_dashboard"}, {"name": "LABEL_VALUE", "value": "1"}]
    env.append({"name": "FOLDER", "value": "/var/lib/grafana/dashboards"})
    if namespace_env is not None:
        env.append({"name": "NAMESPACE", "value": namespace_env})
    template = {"spec": {"containers": [{"name": "sc", "env": env}]}}
    return resource(
        "Deployment", "grafana", "observability", kustomize=False, spec={"template": template}
    )


def dashboard(namespace_name: str, value: str = "1") -> cc.Resource:
    return resource("ConfigMap", "dash", namespace_name, labels={"grafana_dashboard": value})


class DashboardsSelectedTest(unittest.TestCase):
    def test_dashboard_outside_sidecar_namespace_fails(self) -> None:
        found = violations(
            "dashboards-selected", context(grafana("observability"), dashboard("postgres"))
        )
        self.assertIn("outside every Grafana sidecar namespace", found[0])

    def test_sidecar_default_is_its_own_namespace(self) -> None:
        self.assertTrue(
            violations("dashboards-selected", context(grafana(None), dashboard("postgres")))
        )

    def test_wrong_label_value_fails(self) -> None:
        found = violations("dashboards-selected", context(grafana("ALL"), dashboard("app", "0")))
        self.assertIn("matches no Grafana sidecar", found[0])

    def test_all_namespaces_and_listed_namespace_pass(self) -> None:
        self.assertEqual(
            violations("dashboards-selected", context(grafana("ALL"), dashboard("app"))), []
        )
        ctx = context(grafana("observability,postgres"), dashboard("postgres"))
        self.assertEqual(violations("dashboards-selected", ctx), [])


class PolicyExceptionBudgetTest(unittest.TestCase):
    def exceptions(self, count: int) -> list[cc.Resource]:
        return [resource("PolicyException", f"e{index}", "kyverno") for index in range(count)]

    def test_count_above_budget_fails(self) -> None:
        ctx = context(*self.exceptions(2), policy_exception_budget=1)
        self.assertIn("exceed the budget of 1", violations("policy-exception-budget", ctx)[0])

    def test_count_below_budget_asks_to_lower_it(self) -> None:
        ctx = context(*self.exceptions(1), policy_exception_budget=2)
        self.assertIn("lower budget.toml", violations("policy-exception-budget", ctx)[0])

    def test_duplicates_across_overlays_count_once(self) -> None:
        ctx = cc.Context(
            universes={"bases": self.exceptions(1), "overlays/local": self.exceptions(1)},
            policy_exception_budget=1,
        )
        self.assertEqual(violations("policy-exception-budget", ctx), [])


class PruneDisabledTest(unittest.TestCase):
    disabled = {cc.PRUNE_ANNOTATION: cc.PRUNE_DISABLED}

    def test_unannotated_stateful_objects_fail(self) -> None:
        ctx = context(
            namespace("app"),
            resource("PersistentVolumeClaim", "data"),
            resource("Cluster", "db", apiVersion="postgresql.cnpg.io/v1"),
        )
        self.assertEqual(len(violations("prune-disabled", ctx)), 3)

    def test_chart_statefulset_volume_marks_the_namespace(self) -> None:
        sts = resource("StatefulSet", "loki", kustomize=False, spec={"volumeClaimTemplates": [{}]})
        found = violations("prune-disabled", context(namespace("app"), sts))
        self.assertEqual(
            found, ["bases: Namespace app lacks kustomize.toolkit.fluxcd.io/prune: disabled"]
        )

    def test_annotated_objects_and_stateless_namespaces_pass(self) -> None:
        ctx = context(
            namespace("app", annotations=self.disabled),
            namespace("stateless"),
            resource("PersistentVolumeClaim", "data", annotations=self.disabled),
        )
        self.assertEqual(violations("prune-disabled", ctx), [])


DIGEST = "@sha256:" + "a" * 64


def pod_spec(*, containers: list[str], init: list[str] | None = None) -> dict:
    return {
        "containers": [{"name": f"c{i}", "image": image} for i, image in enumerate(containers)],
        "initContainers": [{"name": f"i{i}", "image": image} for i, image in enumerate(init or [])],
    }


class ImageDigestTest(unittest.TestCase):
    def exemptions(self, rendered=None, embedded=None) -> cc.image_digests.Exemptions:
        return cc.image_digests.Exemptions(rendered or {}, embedded or {})

    def test_undigested_containers_init_containers_and_test_hooks_fail(self) -> None:
        deployment = resource(
            "Deployment",
            "app",
            spec={"template": {"spec": pod_spec(containers=[f"app:1{DIGEST}"], init=["init:1"])}},
        )
        hook = resource(
            "Pod",
            "chart-test",
            kustomize=False,
            annotations={"helm.sh/hook": "test-success"},
            spec=pod_spec(containers=["curlimages/curl"]),
        )
        found = violations("image-digest", context(deployment, hook))
        self.assertEqual(
            found,
            [
                "bases: Deployment app/app runs init:1 without an @sha256 digest",
                "bases: Pod app/chart-test runs curlimages/curl without an @sha256 digest",
            ],
        )

    def test_secret_injected_image_and_flux_install_are_checked(self) -> None:
        secret = resource("Secret", "plugin", data={"SIDECAR_IMAGE": "c2lkZWNhcjox"})
        controller = resource(
            "Deployment",
            "source-controller",
            "flux-system",
            kustomize=False,
            spec={
                "template": {"spec": pod_spec(containers=["ghcr.io/fluxcd/source-controller:v1"])}
            },
        )
        ctx = context(secret, installs={"installs/flux": [controller]})
        self.assertEqual(
            violations("image-digest", ctx),
            [
                "bases: Secret app/plugin runs sidecar:1 without an @sha256 digest",
                "installs/flux: Deployment flux-system/source-controller runs "
                "ghcr.io/fluxcd/source-controller:v1 without an @sha256 digest",
            ],
        )

    def test_digests_and_matching_exemptions_pass(self) -> None:
        catalog = resource(
            "ImageCatalog",
            "postgresql",
            apiVersion="postgresql.cnpg.io/v1",
            spec={"images": [{"major": 18, "image": f"postgresql:18{DIGEST}"}]},
        )
        hook = resource("Pod", "test", spec=pod_spec(containers=["curlimages/curl"]))
        k3s = resource(
            "Job",
            "restore",
            spec={"template": {"spec": pod_spec(containers=[f"rancher/k3s:v1{DIGEST}"])}},
        )
        release = resource("HelmRelease", "chart", spec={"values": {"image": "values:1"}})
        ctx = context(
            catalog,
            hook,
            k3s,
            release,
            digest_exemptions=self.exemptions(
                {"curlimages/curl": "hardcoded"}, {"rancher/klipper-lb:v1": "rancher/k3s"}
            ),
        )
        self.assertEqual(violations("image-digest", ctx), [])

    def test_stale_exemptions_fail(self) -> None:
        ctx = context(
            digest_exemptions=self.exemptions(
                {"curlimages/curl": "hardcoded"}, {"rancher/klipper-lb:v1": "rancher/k3s"}
            )
        )
        self.assertEqual(
            violations("image-digest", ctx),
            [
                "exemption curlimages/curl matches no rendered image; remove the entry",
                "embedded rancher/klipper-lb:v1 names carrier rancher/k3s, "
                "which no render pins by digest",
            ],
        )

    def test_exemption_file_needs_reasons_and_carriers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "exempt.toml"
            for text, message in [
                ('[images."a:1"]\nreason = ""\n', "needs a reason"),
                ('[embedded."a:1"]\nreason = "r"\n', "needs a carrier"),
                ('[other."a:1"]\nreason = "r"\n', "unknown sections"),
            ]:
                with self.subTest(message=message):
                    path.write_text(text)
                    with self.assertRaisesRegex(cc.image_digests.ExemptionError, message):
                        cc.image_digests.load_exemptions(path)

    def test_checked_in_exemptions_load(self) -> None:
        exemptions = cc.image_digests.load_exemptions(cc.LINT_DIR / "image-digest-exempt.toml")
        self.assertIn("curlimages/curl", exemptions.rendered)
        self.assertEqual(set(exemptions.embedded.values()), {"rancher/k3s"})


class ActivationTest(unittest.TestCase):
    def config(self, active: list[str], pending: dict[str, str]) -> dict:
        return {"rules": {"active": active, "pending": pending}}

    def test_every_rule_must_be_listed(self) -> None:
        with self.assertRaisesRegex(cc.ConventionError, "neither active nor pending"):
            cc.load_activation(self.config(["psa-labels"], {}))

    def test_unknown_and_double_listed_rules_fail(self) -> None:
        names = list(cc.RULES)
        with self.assertRaisesRegex(cc.ConventionError, "unknown rule nope"):
            cc.load_activation(self.config([*names, "nope"], {}))
        with self.assertRaisesRegex(cc.ConventionError, "both active and pending"):
            cc.load_activation(self.config(names, {"psa-labels": "N1"}))

    def test_pending_rule_that_passes_must_move_to_active(self) -> None:
        names = [name for name in cc.RULES if name != "psa-labels"]
        activation = cc.load_activation(self.config(names, {"psa-labels": "N1"}))
        outcome = cc.evaluate(context(), activation)
        self.assertEqual(
            outcome.failures, ["[psa-labels] passes today; move it from pending (N1) to active"]
        )

    def test_pending_rule_that_fails_is_reported_not_failed(self) -> None:
        names = [name for name in cc.RULES if name != "psa-labels"]
        activation = cc.load_activation(self.config(names, {"psa-labels": "N1"}))
        outcome = cc.evaluate(context(namespace("app")), activation)
        self.assertEqual(outcome.failures, [])
        self.assertIn(f"{len(names)} active, 1 pending [psa-labels (N1): 1]", outcome.summary)

    def test_checked_in_config_lists_every_rule(self) -> None:
        config = cc.tomllib.loads((cc.LINT_DIR / "conventions.toml").read_text())
        activation = cc.load_activation(config)
        self.assertEqual(
            set(config["privileged_namespaces"]),
            {"kube-system", "observability-agents", "kured", "falco"},
        )
        self.assertIn("policy-exception-budget", activation.active)


class ChartValuesTest(unittest.TestCase):
    def test_reads_values_from_the_cached_archive_and_local_charts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = b"serviceMonitor:\n  enabled: false\n"
            with tarfile.open(root / "x-1.0.0-123.tgz", "w:gz") as archive:
                info = tarfile.TarInfo("x/values.yaml")
                info.size = len(payload)
                archive.addfile(info, io.BytesIO(payload))
            (root / "charts/local").mkdir(parents=True)
            (root / "charts/local/values.yaml").write_text("a: 1\n")
            remote = cc.chart_values_text(("x", "1.0.0"), root, root)
            local = cc.chart_values_text(("./charts/local", ""), root, root)
            missing = cc.chart_values_text(("x", "2.0.0"), root, root)
        self.assertEqual(remote, payload.decode())
        self.assertEqual(local, "a: 1\n")
        self.assertIsNone(missing)

    def test_chart_ref_resolves_through_its_oci_repository(self) -> None:
        release = resource(
            "HelmRelease",
            "x",
            "app",
            apiVersion="helm.toolkit.fluxcd.io/v2",
            spec={"chartRef": {"kind": "OCIRepository", "name": "x"}},
        )
        source = {"spec": {"url": "oci://ghcr.io/acme/charts/x", "ref": {"tag": "1.0.0"}}}
        self.assertEqual(cc.chart_key(release, {("app", "x"): source}), "x@1.0.0")
        with self.assertRaisesRegex(cc.ConventionError, "missing OCIRepository app/x"):
            cc.chart_key(release, {})


RENDERED_BASE = """\
apiVersion: v1
kind: Namespace
metadata:
  name: app
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: web
  namespace: app
spec:
  chart:
    spec:
      chart: ./charts/web
---
apiVersion: kyverno.io/v2
kind: PolicyException
metadata:
  name: legacy
  namespace: app
"""

RENDERED_CHART = """\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
---
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: ignored.example.com
"""

CONFIG = """\
[rules]
active = ["policy-exception-budget", "dashboards-selected", "metrics-enabled", "alert-metadata",
  "alert-runbook", "alert-tested", "rules-selected", "prune-disabled", "exporters-scraped",
  "image-digest"]
[rules.pending]
psa-labels = "N1"
default-deny-network-policy = "N2"
"""

ALERT_PACK = """\
groups:
  - name: g
    rules:
      - alert: AppDown
        expr: up == 0
        labels:
          severity: critical
        annotations:
          summary: s
          description: '{{ $labels.job }} is down'
          runbook_url: https://runbooks.example/app/app-down
"""

ALERT_TEST = """\
rule_files: [../alerts/app.yml]
tests:
  - alert_rule_test:
      - alertname: AppDown
        exp_alerts:
          - exp_labels: {severity: critical}
"""


class MainTest(unittest.TestCase):
    def run_main(
        self, budget: int, values: str = "replicas: 1\n", runbook: str = "app/app-down.md"
    ) -> tuple[int, str, str]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rendered = root / "rendered"
            rendered.mkdir()
            (rendered / "base-app.yaml").write_text(RENDERED_BASE)
            (rendered / "chart-app-web.yaml").write_text(RENDERED_CHART)
            (root / "charts/web").mkdir(parents=True)
            (root / "charts/web/values.yaml").write_text(values)
            (root / "deploy/observability/alerts").mkdir(parents=True)
            (root / "deploy/observability/alerts/app.yml").write_text(ALERT_PACK)
            (root / "deploy/observability/tests").mkdir(parents=True)
            (root / "deploy/observability/tests/app.test.yml").write_text(ALERT_TEST)
            (root / "docs/runbooks" / runbook).parent.mkdir(parents=True)
            (root / "docs/runbooks" / runbook).write_text("# App down\n")
            (root / "conventions.toml").write_text(textwrap.dedent(CONFIG))
            (root / "budget.toml").write_text(f"policy_exceptions = {budget}\n")
            (root / "exempt.toml").write_text("")
            stdout, stderr = io.StringIO(), io.StringIO()
            argv = [
                "--rendered",
                str(rendered),
                "--chart-cache",
                str(root),
                "--repo-root",
                str(root),
            ]
            argv += [
                "--config",
                str(root / "conventions.toml"),
                "--budget",
                str(root / "budget.toml"),
                "--digest-exempt",
                str(root / "exempt.toml"),
            ]
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = cc.main(argv)
        return code, stdout.getvalue(), stderr.getvalue()

    def test_rendered_tree_within_budget_passes(self) -> None:
        code, stdout, stderr = self.run_main(budget=1)
        self.assertEqual(code, 0, stderr)
        self.assertIn(
            "Conventions: 10 active, 2 pending "
            "[default-deny-network-policy (N2): 1; psa-labels (N1): 1]",
            stdout,
        )

    def test_commented_out_monitor_toggle_does_not_count(self) -> None:
        values = "# serviceMonitor:\n#   enabled: false\nreplicas: 1\n"
        code, _, stderr = self.run_main(budget=1, values=values)
        self.assertEqual(code, 0, stderr)

    def test_parsed_monitor_toggle_without_a_monitor_fails(self) -> None:
        values = "metrics:\n  serviceMonitor:\n    enabled: false\n"
        code, _, stderr = self.run_main(budget=1, values=values)
        self.assertEqual(code, 1)
        self.assertIn("[metrics-enabled]", stderr)

    def test_app_alert_with_only_a_platform_runbook_fails(self) -> None:
        code, _, stderr = self.run_main(budget=1, runbook="platform/app-down.md")
        self.assertEqual(code, 1)
        self.assertIn("[alert-runbook]", stderr)
        self.assertIn("docs/runbooks/app/app-down.md", stderr)

    def test_one_exception_over_budget_fails(self) -> None:
        code, _, stderr = self.run_main(budget=0)
        self.assertEqual(code, 1)
        self.assertIn("1 PolicyExceptions exceed the budget of 0", stderr)


if __name__ == "__main__":
    unittest.main()
