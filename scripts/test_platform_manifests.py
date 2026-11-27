from __future__ import annotations

import subprocess
import unittest
from functools import cache
from pathlib import Path
from typing import Any

import yaml
from process_runner import run_process

PLATFORM = Path(__file__).resolve().parent.parent / "deploy/platform"
BASES = ("overlays/everything", "posthog", "redis", "redis-ha")
LOCAL_COMPONENTS = ("foundation", "observability", "postgres", "traefik")
PART_OF = "app.kubernetes.io/part-of"
PRUNE = "kustomize.toolkit.fluxcd.io/prune"
STATEFUL_NAMESPACES = {"observability", "platform-redis", "postgres", "posthog"}


@cache
def build(path: str) -> tuple[dict[str, Any], ...]:
    result = run_process(
        ["kustomize", "build", str(PLATFORM / path)],
        check=True,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
    )
    return tuple(doc for doc in yaml.safe_load_all(result.stdout) if doc)


def objects(kind: str, *paths: str) -> list[dict[str, Any]]:
    return [doc for path in paths for doc in build(path) if doc["kind"] == kind]


def helm_values(name: str, path: str) -> dict[str, Any]:
    for release in objects("HelmRelease", path):
        if release["metadata"]["name"] == name:
            return release["spec"]["values"]
    raise AssertionError(f"HelmRelease {name} not found in {path}")


def annotations(doc: dict[str, Any]) -> dict[str, str]:
    return doc["metadata"].get("annotations") or {}


class TraefikExposureTest(unittest.TestCase):
    def test_traefik_is_a_deployment_behind_a_load_balancer(self) -> None:
        for path in ("traefik", "overlays/local/components/traefik"):
            with self.subTest(path=path):
                values = helm_values("traefik", path)
                self.assertEqual(values["deployment"]["kind"], "Deployment")
                self.assertGreaterEqual(values["deployment"]["replicas"], 1)
                self.assertEqual(values["service"]["spec"]["type"], "LoadBalancer")
                self.assertEqual(values["priorityClassName"], "platform-critical")
                self.assertEqual(
                    values["updateStrategy"]["rollingUpdate"], {"maxUnavailable": 0, "maxSurge": 1}
                )
                for port in values["ports"].values():
                    self.assertNotIn("hostPort", port)

    def test_entrypoints_bound_client_reads_writes_and_idle_connections(self) -> None:
        values = helm_values("traefik", "traefik")
        for entrypoint in ("web", "websecure"):
            with self.subTest(entrypoint=entrypoint):
                self.assertEqual(
                    values["ports"][entrypoint]["transport"]["respondingTimeouts"],
                    {"readTimeout": "60s", "writeTimeout": "60s", "idleTimeout": "180s"},
                )

    def test_default_servers_transport_stays_below_the_backend_timeouts(self) -> None:
        arguments = helm_values("traefik", "traefik")["additionalArguments"]
        for flag in (
            "--serversTransport.forwardingTimeouts.dialTimeout=5s",
            "--serversTransport.forwardingTimeouts.responseHeaderTimeout=35s",
            "--serversTransport.forwardingTimeouts.idleConnTimeout=5s",
        ):
            self.assertIn(flag, arguments)

    def test_services_without_ready_pods_answer_503(self) -> None:
        providers = helm_values("traefik", "traefik")["providers"]
        for provider in ("kubernetesCRD", "kubernetesIngress"):
            with self.subTest(provider=provider):
                self.assertIs(providers[provider]["allowEmptyServices"], True)

    def test_baremetal_runs_two_replicas(self) -> None:
        values = helm_values("traefik", "overlays/baremetal/components/traefik")
        self.assertEqual(values["deployment"]["replicas"], 2)

    def test_pdb_never_blocks_a_drain_at_one_replica(self) -> None:
        values = helm_values("traefik", "traefik")
        pdb = values["podDisruptionBudget"]
        self.assertTrue(pdb["enabled"])
        self.assertNotIn("minAvailable", pdb)
        self.assertGreaterEqual(pdb["maxUnavailable"], 1)

    def test_crds_are_upgraded(self) -> None:
        release = next(
            r for r in objects("HelmRelease", "traefik") if r["metadata"]["name"] == "traefik"
        )
        self.assertEqual(release["spec"]["install"]["crds"], "CreateReplace")
        self.assertEqual(release["spec"]["upgrade"]["crds"], "CreateReplace")


class PostgresDisruptionTest(unittest.TestCase):
    def test_single_instance_clusters_disable_the_primary_pdb(self) -> None:
        clusters = objects("Cluster", "postgres-cluster", "overlays/local/components/postgres")
        self.assertTrue(clusters)
        for cluster in clusters:
            spec = cluster["spec"]
            if spec["instances"] == 1:
                self.assertIs(spec.get("enablePDB"), False, cluster["metadata"]["name"])

    def test_restore_role_cannot_touch_the_production_cluster(self) -> None:
        (role,) = objects("Role", "postgres-cluster")
        for rule in role["rules"]:
            destructive = {"delete", "patch", "update", "deletecollection", "*"} & set(
                rule["verbs"]
            )
            if destructive:
                self.assertEqual(rule.get("resourceNames"), ["postgres-cluster-restore-test"])
        verbs = {verb for rule in role["rules"] for verb in rule["verbs"]}
        self.assertTrue({"create", "get", "list", "watch", "delete", "patch"} <= verbs)


class PruneSafetyTest(unittest.TestCase):
    def test_no_base_manages_kube_system(self) -> None:
        names = {ns["metadata"]["name"] for path in BASES for ns in objects("Namespace", path)}
        self.assertNotIn("kube-system", names)

    def test_every_base_namespace_is_labelled_for_discovery(self) -> None:
        for path in BASES:
            for namespace in objects("Namespace", path):
                with self.subTest(namespace=namespace["metadata"]["name"]):
                    self.assertEqual(namespace["metadata"]["labels"][PART_OF], "baukit-platform")

    def test_stateful_objects_are_never_pruned(self) -> None:
        components = tuple(f"overlays/local/components/{name}" for name in LOCAL_COMPONENTS)
        stateful = [
            ns
            for path in BASES
            for ns in objects("Namespace", path)
            if ns["metadata"]["name"] in STATEFUL_NAMESPACES
        ]
        stateful += objects("Cluster", *BASES, *components)
        stateful += objects("PersistentVolumeClaim", *BASES, *components)
        self.assertGreaterEqual(len(stateful), len(STATEFUL_NAMESPACES) + 2)
        for doc in stateful:
            with self.subTest(kind=doc["kind"], name=doc["metadata"]["name"]):
                self.assertEqual(annotations(doc).get(PRUNE), "disabled")


class ObservabilityDiscoveryTest(unittest.TestCase):
    def test_prometheus_selects_platform_namespaces(self) -> None:
        spec = helm_values("kube-prometheus-stack", "observability-stack")["prometheus"][
            "prometheusSpec"
        ]
        platform = {"matchLabels": {PART_OF: "baukit-platform"}}
        self.assertEqual(spec["ruleNamespaceSelector"], platform)
        for key in ("serviceMonitorNamespaceSelector", "podMonitorNamespaceSelector"):
            with self.subTest(selector=key):
                self.assertEqual(spec[key], {})
        self.assertEqual(spec["enforcedNamespaceLabel"], "namespace")

    def test_prometheus_rules_live_in_selected_namespaces(self) -> None:
        labelled = {
            ns["metadata"]["name"]
            for path in BASES
            for ns in objects("Namespace", path)
            if ns["metadata"]["labels"].get(PART_OF) == "baukit-platform"
        }
        packs = [
            release
            for release in objects("HelmRelease", *BASES)
            if release["metadata"]["name"] == "baukit-platform-observability"
        ]
        self.assertEqual(len(packs), 1)
        for release in packs:
            self.assertIn(release["spec"]["targetNamespace"], labelled)
        for rule in objects("PrometheusRule", *BASES):
            with self.subTest(rule=rule["metadata"]["name"]):
                self.assertIn(rule["metadata"]["namespace"], labelled)

    def test_grafana_finds_dashboards_in_every_namespace_by_folder(self) -> None:
        sidecar = helm_values("kube-prometheus-stack", "observability-stack")["grafana"]["sidecar"]
        dashboards = sidecar["dashboards"]
        self.assertEqual(dashboards["searchNamespace"], "ALL")
        self.assertEqual(dashboards["label"], "grafana_dashboard")
        self.assertEqual(dashboards["folderAnnotation"], "grafana_folder")
        self.assertTrue(dashboards["provider"]["foldersFromFilesStructure"])


if __name__ == "__main__":
    unittest.main()
