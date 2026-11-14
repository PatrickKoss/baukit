from __future__ import annotations

import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
ALL_OPTIONS = CHART / "tests/all-options-values.yaml"
TRAEFIK_RELEASE = Path(__file__).resolve().parents[1] / "deploy/platform/traefik/helm-release.yaml"
RELEASE = "netpol-test"
TRAEFIK_PEER = {
    "namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "traefik"}},
    "podSelector": {
        "matchLabels": {
            "app.kubernetes.io/name": "traefik",
            "app.kubernetes.io/instance": "traefik-traefik",
        }
    },
}
PROMETHEUS_PEER = {
    "namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "observability"}},
    "podSelector": {"matchLabels": {"app.kubernetes.io/name": "prometheus"}},
}


def render(*args: str) -> dict[str, dict]:
    result = run_process(
        ["helm", "template", RELEASE, str(CHART), "-f", str(ALL_OPTIONS), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return {
        doc["metadata"]["name"].removeprefix("chart-options-"): doc["spec"]
        for doc in yaml.safe_load_all(result.stdout)
        if doc and doc["kind"] == "NetworkPolicy"
    }


def process_of(spec: dict) -> str | None:
    return spec["podSelector"]["matchLabels"].get("baukit.dev/process")


class NetworkPolicyTest(unittest.TestCase):
    def test_default_deny_selects_every_process(self) -> None:
        deny = render()["default-deny"]
        self.assertEqual(deny["policyTypes"], ["Ingress", "Egress"])
        self.assertNotIn("ingress", deny)
        self.assertNotIn("egress", deny)
        self.assertIsNone(process_of(deny))

    def test_traefik_base_reaches_api_and_web_by_default(self) -> None:
        policies = render()
        for name, process, port in [("api-ingress", "api", 8080), ("web-ingress", "web", 8080)]:
            with self.subTest(policy=name):
                spec = policies[name]
                self.assertEqual(process_of(spec), process)
                [rule] = spec["ingress"]
                self.assertEqual(rule["from"], [TRAEFIK_PEER])
                self.assertEqual(rule["ports"], [{"port": port, "protocol": "TCP"}])

    def test_traefik_peer_matches_the_platform_base(self) -> None:
        release = yaml.safe_load(TRAEFIK_RELEASE.read_text())
        namespace = release["metadata"]["namespace"]
        name = release["metadata"]["name"]
        selectors = TRAEFIK_PEER["namespaceSelector"]["matchLabels"]
        self.assertEqual(selectors["kubernetes.io/metadata.name"], namespace)
        pods = TRAEFIK_PEER["podSelector"]["matchLabels"]
        self.assertEqual(pods["app.kubernetes.io/instance"], f"{name}-{namespace}")

    def test_prometheus_reaches_only_ops_ports(self) -> None:
        policies = render()
        for name, process in [("api-ops-ingress", "api"), ("worker-ops-ingress", "worker")]:
            with self.subTest(policy=name):
                spec = policies[name]
                self.assertEqual(process_of(spec), process)
                [rule] = spec["ingress"]
                self.assertEqual(rule["from"], [PROMETHEUS_PEER])
                self.assertEqual(rule["ports"], [{"port": 9090, "protocol": "TCP"}])

    def test_process_rules_select_only_their_process(self) -> None:
        policies = render()
        expected = {
            "api-additional-ingress": "api",
            "api-egress": "api",
            "worker-additional-ingress": "worker",
            "worker-egress": "worker",
            "migrate-egress": "migrate",
            "seed-egress": "seed",
        }
        for name, process in expected.items():
            with self.subTest(policy=name):
                self.assertEqual(process_of(policies[name]), process)
        self.assertEqual(
            policies["worker-egress"]["egress"][0]["ports"],
            [{"protocol": "TCP", "port": 22}, {"protocol": "TCP", "port": 9418}],
        )

    def test_shared_egress_allows_dns_first(self) -> None:
        egress = render("--set", "networkPolicy.dns.k3sCompatible=false")["egress"]["egress"]
        self.assertEqual(
            egress[0]["to"],
            [
                {
                    "namespaceSelector": {
                        "matchLabels": {"kubernetes.io/metadata.name": "kube-system"}
                    },
                    "podSelector": {"matchLabels": {"k8s-app": "kube-dns"}},
                }
            ],
        )
        alloy = egress[1]["to"][0]["podSelector"]["matchLabels"]
        self.assertEqual(alloy["app.kubernetes.io/name"], "alloy")

    def test_redis_admits_only_the_api(self) -> None:
        policies = render("--set", "redis.enabled=true")
        redis = policies[f"{RELEASE}-redis-ingress"]
        self.assertEqual(process_of(redis), "redis")
        [rule] = redis["ingress"]
        self.assertEqual(rule["from"][0]["podSelector"]["matchLabels"]["baukit.dev/process"], "api")

    def test_disabled_network_policy_renders_nothing(self) -> None:
        self.assertEqual(render("--set", "networkPolicy.enabled=false"), {})


if __name__ == "__main__":
    unittest.main()
