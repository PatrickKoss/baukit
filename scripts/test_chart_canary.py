from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
IMAGES = CHART / "tests/images-values.yaml"
RELEASE = "canary-test"
NAMESPACE = "shop"
API = f"{RELEASE}-baukit-app-api"
SERVICE_MONITOR_API = "monitoring.coreos.com/v1/ServiceMonitor"
CANARY = (
    "canary.enabled=true",
    "ingress.enabled=true",
    "serviceMonitor.enabled=true",
    "product=shop",
    "deploymentEnvironment=production",
)


def render_raw(*settings: str) -> subprocess.CompletedProcess:
    command = [
        "helm",
        "template",
        RELEASE,
        str(CHART),
        "-f",
        str(IMAGES),
        "--namespace",
        NAMESPACE,
        "--api-versions",
        SERVICE_MONITOR_API,
    ]
    for setting in settings:
        command.extend(["--set", setting])
    return run_process(command, capture_output=True, text=True, check=False)


def render(*settings: str) -> list[dict]:
    result = render_raw(*settings)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return [doc for doc in yaml.safe_load_all(result.stdout) if doc]


def find(docs: list[dict], kind: str, name: str | None = None) -> list[dict]:
    return [
        doc
        for doc in docs
        if doc["kind"] == kind and (name is None or doc["metadata"]["name"] == name)
    ]


def api_monitor_relabelings(docs: list[dict]) -> list[dict]:
    [monitor] = find(docs, "ServiceMonitor", API)
    return monitor["spec"]["endpoints"][0]["relabelings"]


class WorkloadSelectorTest(unittest.TestCase):
    def test_deployments_select_on_the_flagger_workload_label(self) -> None:
        docs = render("worker.enabled=true")
        for process in ["api", "worker"]:
            with self.subTest(process=process):
                name = f"{RELEASE}-baukit-app-{process}"
                [deployment] = find(docs, "Deployment", name)
                self.assertEqual(
                    deployment["spec"]["selector"]["matchLabels"]["baukit.dev/workload"], name
                )
                pod_labels = deployment["spec"]["template"]["metadata"]["labels"]
                self.assertEqual(pod_labels["baukit.dev/workload"], name)

    def test_services_leave_out_the_workload_label(self) -> None:
        for service in find(render(), "Service"):
            with self.subTest(service=service["metadata"]["name"]):
                self.assertNotIn("baukit.dev/workload", service["spec"]["selector"])


class CanaryTest(unittest.TestCase):
    def test_canary_is_opt_in(self) -> None:
        docs = render("ingress.enabled=true", "serviceMonitor.enabled=true")
        self.assertEqual(find(docs, "Canary"), [])
        self.assertEqual(find(docs, "MetricTemplate"), [])
        self.assertEqual(find(docs, "IngressRoute"), [])
        self.assertEqual(len(find(docs, "Ingress")), 1)
        targets = [r["targetLabel"] for r in api_monitor_relabelings(docs)]
        self.assertNotIn("rollout", targets)

    def test_canary_gates_the_api_on_errors_and_p95(self) -> None:
        docs = render(*CANARY)
        [canary] = find(docs, "Canary", API)
        spec = canary["spec"]
        self.assertEqual(spec["provider"], "traefik")
        self.assertEqual(
            spec["targetRef"], {"apiVersion": "apps/v1", "kind": "Deployment", "name": API}
        )
        self.assertNotIn("autoscalerRef", spec)
        self.assertEqual(spec["service"], {"name": API, "port": 80, "targetPort": "http"})
        gates = {m["name"]: m for m in spec["analysis"]["metrics"]}
        self.assertEqual(set(gates), {"error-percent", "p95-seconds"})
        self.assertEqual(gates["error-percent"]["thresholdRange"], {"max": 1})
        self.assertEqual(gates["p95-seconds"]["thresholdRange"], {"max": 0.5})
        for gate in gates.values():
            self.assertNotIn("namespace", gate["templateRef"])
            self.assertEqual(
                find(docs, "MetricTemplate", gate["templateRef"]["name"])[0]["kind"],
                "MetricTemplate",
            )

    def test_metric_templates_query_the_canary_through_the_tenant_proxy(self) -> None:
        templates = find(render(*CANARY), "MetricTemplate")
        self.assertEqual(len(templates), 2)
        for template in templates:
            with self.subTest(template=template["metadata"]["name"]):
                provider = template["spec"]["provider"]
                self.assertEqual(provider["type"], "prometheus")
                self.assertEqual(
                    provider["address"],
                    f"http://tenant-metrics-proxy.{NAMESPACE}.svc.cluster.local:8080",
                )
                query = template["spec"]["query"]
                self.assertIn(
                    'product="shop", service="shop-api", '
                    'environment="production", rollout="canary"',
                    query,
                )
                self.assertIn("[{{ interval }}]", query)
        [errors] = [t for t in templates if t["metadata"]["name"].endswith("-error-percent")]
        self.assertIn('status=~"5.."', errors["spec"]["query"])

    def test_prometheus_address_can_point_outside_a_tenant(self) -> None:
        address = "http://kube-prometheus-stack-prometheus.observability.svc.cluster.local:9090"
        for template in find(
            render(*CANARY, f"canary.prometheusAddress={address}"), "MetricTemplate"
        ):
            self.assertEqual(template["spec"]["provider"]["address"], address)

    def test_autoscaled_canary_hands_the_hpa_to_flagger(self) -> None:
        [canary] = find(render(*CANARY, "autoscaling.enabled=true"), "Canary")
        self.assertEqual(canary["spec"]["autoscalerRef"]["name"], API)

    def test_service_monitor_labels_rollout_from_the_pod(self) -> None:
        rollout = [
            r for r in api_monitor_relabelings(render(*CANARY)) if r["targetLabel"] == "rollout"
        ]
        source = ["__meta_kubernetes_pod_label_baukit_dev_workload"]
        self.assertEqual(
            [(r["sourceLabels"], r["regex"], r["replacement"]) for r in rollout],
            [(source, ".+", "canary"), (source, ".+-primary", "primary")],
        )

    def test_ingress_becomes_an_ingress_route_to_the_traefik_service(self) -> None:
        docs = render(
            *CANARY,
            "ingress.rateLimitMiddleware=shop-ratelimit@kubernetescrd",
            "ingress.tls[0].secretName=shop-tls",
            "ingress.hosts[0].host=shop.example.com",
            "ingress.hosts[0].paths[0].path=/",
            "ingress.hosts[0].paths[0].pathType=Prefix",
            "ingress.hosts[0].paths[1].path=/exact",
            "ingress.hosts[0].paths[1].pathType=Exact",
        )
        self.assertEqual(find(docs, "Ingress"), [])
        [route] = find(docs, "IngressRoute")
        self.assertEqual(route["spec"]["entryPoints"], ["websecure"])
        self.assertEqual(route["spec"]["tls"], {"secretName": "shop-tls"})
        routes = route["spec"]["routes"]
        self.assertEqual(
            [r["match"] for r in routes],
            [
                "Host(`shop.example.com`) && PathPrefix(`/`)",
                "Host(`shop.example.com`) && Path(`/exact`)",
            ],
        )
        for entry in routes:
            self.assertEqual(entry["services"], [{"kind": "TraefikService", "name": API}])
            self.assertEqual(
                entry["middlewares"],
                [{"name": "shop-ratelimit@kubernetescrd"}, {"name": f"{RELEASE}-baukit-app-retry"}],
            )

    def test_cert_manager_annotation_is_refused_with_a_canary(self) -> None:
        result = render_raw(*CANARY, "ingress.certManager.clusterIssuer=letsencrypt")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("issue a Certificate", result.stderr)

    def test_schema_rejects_a_zero_step_weight(self) -> None:
        result = render_raw(*CANARY, "canary.analysis.stepWeight=0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stepWeight", result.stderr)


if __name__ == "__main__":
    unittest.main()
