from __future__ import annotations

import unittest
from pathlib import Path

import yaml

POSTGRES = Path(__file__).resolve().parent.parent / "postgres-cluster"
TENANT_GROUP = "baukit_tenants"
TENANT_CONNECTION_LIMIT = 25
MAX_CONNECTIONS = 150
TENANTS_IN_BUDGET = 5
EXPECTED_PG_HBA = [
    f"host samerole +{TENANT_GROUP} all scram-sha-256",
    f"host all +{TENANT_GROUP} all reject",
]


def documents(path: Path) -> list[dict]:
    return [doc for doc in yaml.safe_load_all(path.read_text()) if isinstance(doc, dict)]


def uncommented(path: Path) -> str:
    example = path.read_text().split("\n#\n", 1)[1]
    return "\n".join(line.removeprefix("#").removeprefix(" ") for line in example.splitlines())


class PostgresTenantIsolationTest(unittest.TestCase):
    def test_cluster_rejects_tenants_outside_their_database(self) -> None:
        (cluster,) = documents(POSTGRES / "cluster.yaml")
        self.assertEqual(cluster["spec"]["postgresql"]["pg_hba"], EXPECTED_PG_HBA)

    def test_tenant_group_is_a_nologin_role_in_the_base(self) -> None:
        (role,) = documents(POSTGRES / "tenant-group.yaml")
        self.assertEqual(role["kind"], "DatabaseRole")
        self.assertEqual(role["spec"]["name"], TENANT_GROUP)
        self.assertFalse(role["spec"]["login"])
        kustomization = yaml.safe_load((POSTGRES / "kustomization.yaml").read_text())
        self.assertIn("tenant-group.yaml", kustomization["resources"])

    def test_example_owner_joins_the_group_and_its_database_role(self) -> None:
        example = uncommented(POSTGRES / "product-database.example.yaml")
        docs = [doc for doc in yaml.safe_load_all(example) if doc]
        (database,) = [doc for doc in docs if doc["kind"] == "Database"]
        roles = {doc["spec"]["name"]: doc["spec"] for doc in docs if doc["kind"] == "DatabaseRole"}
        name = database["spec"]["name"]
        self.assertFalse(roles[name]["login"])
        owner = roles[database["spec"]["owner"]]
        self.assertTrue(owner["login"])
        self.assertEqual(sorted(owner["inRoles"]), sorted([TENANT_GROUP, name]))
        self.assertEqual(owner["connectionLimit"], TENANT_CONNECTION_LIMIT)

    def test_connection_budget_fits_max_connections(self) -> None:
        (cluster,) = documents(POSTGRES / "cluster.yaml")
        max_connections = int(cluster["spec"]["postgresql"]["parameters"]["max_connections"])
        self.assertEqual(max_connections, MAX_CONNECTIONS)
        self.assertLess(TENANTS_IN_BUDGET * TENANT_CONNECTION_LIMIT, max_connections)
        (pooler,) = documents(POSTGRES / "pooler.yaml")
        parameters = pooler["spec"]["pgbouncer"]["parameters"]
        self.assertEqual(pooler["spec"]["pgbouncer"]["poolMode"], "transaction")
        self.assertLess(int(parameters["max_db_connections"]), TENANT_CONNECTION_LIMIT)
        self.assertLessEqual(
            int(parameters["default_pool_size"]) + int(parameters["reserve_pool_size"]),
            int(parameters["max_db_connections"]),
        )


if __name__ == "__main__":
    unittest.main()
