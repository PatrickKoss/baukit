from __future__ import annotations

import unittest

from test_chart_canary import RELEASE, find, render

SHARED = (
    "backing.postgres.mode=shared",
    "worker.enabled=true",
    "migration.enabled=true",
    "product=shop",
)
DIRECT = (*SHARED, "backing.postgres.shared.directUrlSecretRef.name=shop-db-direct")
URL = "SHOP__DATABASE__URL"
POOLER = {"cnpg.io/poolerName": "postgres-cluster-pooler-rw"}


def database_env(docs: list[dict], kind: str, process: str) -> list[dict]:
    [workload] = find(docs, kind, f"{RELEASE}-baukit-app-{process}")
    container = workload["spec"]["template"]["spec"]["containers"][0]
    return [entry for entry in container.get("env", []) if entry["name"] == URL]


def pod_selectors(docs: list[dict], process: str) -> list[dict]:
    [policy] = find(docs, "NetworkPolicy", f"{RELEASE}-baukit-app-{process}-backing-egress")
    return [
        peer["podSelector"]["matchLabels"]
        for rule in policy["spec"]["egress"]
        for peer in rule["to"]
    ]


class SharedPostgresSplitTest(unittest.TestCase):
    def test_without_a_direct_url_every_process_may_use_the_pooler(self) -> None:
        docs = render(*SHARED)
        for kind, process in [("Deployment", "api"), ("Deployment", "worker"), ("Job", "migrate")]:
            with self.subTest(process=process):
                self.assertEqual(database_env(docs, kind, process), [])
                self.assertIn(POOLER, pod_selectors(docs, process))

    def test_direct_url_moves_worker_and_migrations_off_the_pooler(self) -> None:
        docs = render(*DIRECT)
        expected = [
            {"name": URL, "valueFrom": {"secretKeyRef": {"name": "shop-db-direct", "key": "uri"}}}
        ]
        for kind, process in [("Deployment", "worker"), ("Job", "migrate")]:
            with self.subTest(process=process):
                self.assertEqual(database_env(docs, kind, process), expected)
                self.assertNotIn(POOLER, pod_selectors(docs, process))
                self.assertIn({"cnpg.io/cluster": "postgres-cluster"}, pod_selectors(docs, process))
        self.assertEqual(database_env(docs, "Deployment", "api"), [])
        self.assertIn(POOLER, pod_selectors(docs, "api"))


if __name__ == "__main__":
    unittest.main()
