from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import check_helm_test_netpol

RELEASE = {
    "apiVersion": "helm.toolkit.fluxcd.io/v2",
    "kind": "HelmRelease",
    "metadata": {"name": "app", "namespace": "demo"},
    "spec": {"test": {"enable": True}},
}
DEFAULT_DENY = {
    "kind": "NetworkPolicy",
    "metadata": {"name": "default-deny", "namespace": "demo"},
    "spec": {"podSelector": {}, "policyTypes": ["Ingress", "Egress"]},
}
SERVER = {"app.kubernetes.io/name": "app", "app.kubernetes.io/component": "server"}
TESTER = {"app.kubernetes.io/name": "app", "app.kubernetes.io/component": "test"}
CHART = [
    {
        "kind": "Service",
        "metadata": {"name": "app-svc"},
        "spec": {"selector": SERVER, "ports": [{"port": 80, "targetPort": "http"}]},
    },
    {
        "kind": "Deployment",
        "metadata": {"name": "app"},
        "spec": {
            "template": {
                "metadata": {"labels": SERVER},
                "spec": {"containers": [{"ports": [{"name": "http", "containerPort": 8080}]}]},
            }
        },
    },
    {
        "kind": "Pod",
        "metadata": {
            "name": "app-test",
            "labels": TESTER,
            "annotations": {"helm.sh/hook": "test"},
        },
        "spec": {"containers": [{"command": ["curl"], "args": ["app-svc:80/healthz"]}]},
    },
]
ALLOW_TEST = [
    {
        "kind": "NetworkPolicy",
        "metadata": {"name": "test-egress", "namespace": "demo"},
        "spec": {
            "podSelector": {"matchLabels": TESTER},
            "policyTypes": ["Egress"],
            "egress": [
                {"to": [{"podSelector": {"matchLabels": SERVER}}], "ports": [{"port": 8080}]}
            ],
        },
    },
    {
        "kind": "NetworkPolicy",
        "metadata": {"name": "test-ingress", "namespace": "demo"},
        "spec": {
            "podSelector": {
                "matchExpressions": [
                    {"key": "app.kubernetes.io/component", "operator": "In", "values": ["server"]}
                ]
            },
            "policyTypes": ["Ingress"],
            "ingress": [
                {"from": [{"podSelector": {"matchLabels": TESTER}}], "ports": [{"port": 8080}]}
            ],
        },
    },
]


def dump_all(docs: list[dict]) -> str:
    return "\n---\n".join(json.dumps(doc) for doc in docs)


class HelmTestNetpolTests(unittest.TestCase):
    def problems(self, base: list[dict]) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            rendered = Path(tmp)
            (rendered / "base-app.yaml").write_text(dump_all(base))
            (rendered / "chart-app-app.yaml").write_text(dump_all(CHART))
            return check_helm_test_netpol.problems(rendered)

    def test_default_deny_blocks_the_test_pod_both_ways(self):
        self.assertEqual(
            self.problems([RELEASE, DEFAULT_DENY]),
            [
                "demo/app-test: egress to app-svc:8080 is not allowed",
                "demo/app-test: app-svc does not admit it on 8080",
            ],
        )

    def test_policies_for_the_test_pod_pass(self):
        self.assertEqual(self.problems([RELEASE, DEFAULT_DENY, *ALLOW_TEST]), [])

    def test_namespace_without_policies_passes(self):
        self.assertEqual(self.problems([RELEASE]), [])

    def test_release_without_tests_is_skipped(self):
        quiet = {**RELEASE, "spec": {"test": {"enable": False}}}
        self.assertEqual(self.problems([quiet, DEFAULT_DENY]), [])

    def test_excluded_test_pod_is_skipped(self):
        filtered = {
            **RELEASE,
            "spec": {"test": {"enable": True, "filters": [{"name": "app-test", "exclude": True}]}},
        }
        self.assertEqual(self.problems([filtered, DEFAULT_DENY]), [])

    def test_test_pod_with_no_known_destination_is_reported(self):
        stray = {**CHART[2], "spec": {"containers": [{"command": ["true"]}]}}
        with tempfile.TemporaryDirectory() as tmp:
            rendered = Path(tmp)
            (rendered / "base-app.yaml").write_text(dump_all([RELEASE]))
            (rendered / "chart-app-app.yaml").write_text(dump_all([stray]))
            self.assertEqual(
                check_helm_test_netpol.problems(rendered),
                ["demo/app-test: no chart Service found in the test command"],
            )

    def test_url_in_a_mounted_config_map_names_the_service(self):
        tester = {
            **CHART[2],
            "spec": {
                "containers": [{"command": ["bats", "/tests/run.sh"]}],
                "volumes": [{"name": "tests", "configMap": {"name": "app-test"}}],
            },
        }
        script = {
            "kind": "ConfigMap",
            "metadata": {"name": "app-test"},
            "data": {"run.sh": 'url="http://app-svc/api/health"'},
        }
        with tempfile.TemporaryDirectory() as tmp:
            rendered = Path(tmp)
            (rendered / "base-app.yaml").write_text(dump_all([RELEASE, DEFAULT_DENY]))
            (rendered / "chart-app-app.yaml").write_text(dump_all([*CHART[:2], tester, script]))
            self.assertEqual(
                check_helm_test_netpol.problems(rendered),
                [
                    "demo/app-test: egress to app-svc:8080 is not allowed",
                    "demo/app-test: app-svc does not admit it on 8080",
                ],
            )


if __name__ == "__main__":
    unittest.main()
