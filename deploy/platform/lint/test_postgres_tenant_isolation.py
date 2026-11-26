from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

PLATFORM = Path(__file__).resolve().parent.parent
POSTGRES = PLATFORM / "postgres-cluster"
DATABASE_CONFIG = PLATFORM.parent.parent / "rust/crates/baukit-config/src/database.rs"
ROLE_TIMEOUTS = {
    "statement_timeout": "DEFAULT_STATEMENT_TIMEOUT",
    "lock_timeout": "DEFAULT_LOCK_TIMEOUT",
    "idle_in_transaction_session_timeout": "DEFAULT_IDLE_IN_TRANSACTION_SESSION_TIMEOUT",
}
KEYCLOAK_POOL_MAX = 10
TENANT_GROUP = "baukit_tenants"
TENANT_CONNECTION_LIMIT = 25
MAX_CONNECTIONS = 150
TENANTS_IN_BUDGET = 5
KEYCLOAK_CONNECTION_LIMIT = 12
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

    def test_bootstrap_owner_gets_the_tenant_limit_and_config_timeouts(self) -> None:
        (cluster,) = documents(POSTGRES / "cluster.yaml")
        initdb = cluster["spec"]["bootstrap"]["initdb"]
        owner = initdb["owner"]
        expected = [f"ALTER ROLE {owner} CONNECTION LIMIT {TENANT_CONNECTION_LIMIT}"] + [
            f"ALTER ROLE {owner} SET {name} = '{seconds}s'"
            for name, seconds in config_timeouts().items()
        ]
        self.assertEqual(initdb["postInitApplicationSQL"], expected)

    def test_example_documents_the_same_owner_timeouts(self) -> None:
        example = (POSTGRES / "product-database.example.yaml").read_text()
        for name, seconds in config_timeouts().items():
            self.assertIn(f"ALTER ROLE product_owner SET {name} = '{seconds}s'", example)

    def test_pooler_timeouts_sit_above_the_role_timeouts(self) -> None:
        (pooler,) = documents(POSTGRES / "pooler.yaml")
        parameters = pooler["spec"]["pgbouncer"]["parameters"]
        timeouts = config_timeouts()
        self.assertGreater(int(parameters["query_timeout"]), timeouts["statement_timeout"])
        self.assertEqual(
            int(parameters["idle_transaction_timeout"]),
            timeouts["idle_in_transaction_session_timeout"],
        )
        self.assertIn("options", parameters["ignore_startup_parameters"].split(","))

    def test_keycloak_owner_is_capped_above_its_pool(self) -> None:
        documents_by_kind = {
            doc["kind"]: doc for doc in documents(PLATFORM / "keycloak/database.yaml")
        }
        limit = documents_by_kind["DatabaseRole"]["spec"]["connectionLimit"]
        self.assertEqual(limit, KEYCLOAK_CONNECTION_LIMIT)
        self.assertGreater(limit, KEYCLOAK_POOL_MAX)
        self.assertLess(
            TENANTS_IN_BUDGET * TENANT_CONNECTION_LIMIT + KEYCLOAK_CONNECTION_LIMIT, MAX_CONNECTIONS
        )


def config_timeouts() -> dict[str, int]:
    source = DATABASE_CONFIG.read_text()
    found = {}
    for setting, constant in ROLE_TIMEOUTS.items():
        match = re.search(rf"const {constant}: Duration = Duration::from_secs\((\d+)\);", source)
        if match is None:
            raise AssertionError(f"{constant} not found in {DATABASE_CONFIG}")
        found[setting] = int(match.group(1))
    return found


if __name__ == "__main__":
    unittest.main()
