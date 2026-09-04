from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path

TEST_DIRECTORY = Path(__file__).resolve().parent
SCRIPT_DIRECTORY = TEST_DIRECTORY.parent
sys.path.insert(0, str(SCRIPT_DIRECTORY))
SCRIPT_PATH = SCRIPT_DIRECTORY / "reconcile_keycloak.py"
SPEC = importlib.util.spec_from_file_location("reconcile_keycloak", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
reconcile_keycloak = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reconcile_keycloak)


class FakeApi:
    def __init__(self, realm, clients=None, users=None):
        self.realm_value = copy.deepcopy(realm)
        self.clients = copy.deepcopy(clients or {})
        self.users = copy.deepcopy(users or {})
        self.roles = {
            "offline_access": {"id": "offline", "name": "offline_access"},
            "admin": {"id": "admin-role", "name": "admin"},
        }
        self.user_roles = {identity: [] for identity in self.users}
        self.updates = []
        self.password_resets = []

    def realm(self, realm):
        return copy.deepcopy(self.realm_value)

    def update_realm(self, realm, value):
        self.realm_value = copy.deepcopy(value)
        self.updates.append(("realm", realm))

    def find(self, realm, collection, key, value):
        values = self.clients if collection == "clients" else self.users
        return [
            copy.deepcopy(item)
            for item in values.values()
            if item.get(key) == value
        ]

    def get(self, realm, collection, identity):
        values = self.clients if collection == "clients" else self.users
        return copy.deepcopy(values[identity])

    def create(self, realm, collection, value):
        values = self.clients if collection == "clients" else self.users
        identity = f"{collection}-{len(values) + 1}"
        created = copy.deepcopy(value)
        created["id"] = identity
        created.pop("credentials", None)
        created.pop("realmRoles", None)
        values[identity] = created
        self.user_roles.setdefault(identity, [])
        self.updates.append((f"create-{collection}", value.get("clientId", value.get("username"))))

    def update(self, realm, collection, identity, value):
        values = self.clients if collection == "clients" else self.users
        values[identity] = copy.deepcopy(value)
        self.updates.append((f"update-{collection}", identity))

    def delete(self, realm, collection, identity):
        values = self.clients if collection == "clients" else self.users
        del values[identity]

    def reset_password(self, realm, user_id, credential):
        self.password_resets.append((user_id, copy.deepcopy(credential)))

    def realm_role(self, realm, role_name):
        return copy.deepcopy(self.roles[role_name])

    def user_realm_roles(self, realm, user_id):
        return copy.deepcopy(self.user_roles[user_id])

    def add_user_realm_roles(self, realm, user_id, roles):
        self.user_roles[user_id].extend(copy.deepcopy(roles))


class RealmReconcilerTests(unittest.TestCase):
    def setUp(self):
        self.desired = {
            "realm": "fixture",
            "displayName": "Fixture development",
            "enabled": True,
            "sslRequired": "none",
            "registrationAllowed": False,
            "loginWithEmailAllowed": True,
            "loginTheme": "baukit-accessible",
            "passwordPolicy": "length(12) and notUsername and notEmail and maxLength(128)",
            "bruteForceProtected": True,
            "clients": [
                {
                    "clientId": "fixture-web",
                    "name": "Desired name",
                    "publicClient": True,
                    "standardFlowEnabled": True,
                    "directAccessGrantsEnabled": False,
                    "redirectUris": ["http://localhost:5173/*"],
                    "webOrigins": ["http://localhost:5173"],
                    "attributes": {"pkce.code.challenge.method": "S256"},
                }
            ],
            "users": [
                {
                    "username": "test",
                    "email": "test@example.test",
                    "enabled": True,
                    "realmRoles": ["offline_access"],
                    "credentials": [
                        {"type": "password", "value": "private", "temporary": False}
                    ],
                }
            ],
        }
        self.config = {
            "environmentClass": "development",
            "realmFields": [
                "displayName",
                "sslRequired",
                "loginTheme",
                "passwordPolicy",
                "bruteForceProtected",
            ],
            "clients": [
                {
                    "clientId": "fixture-web",
                    "activeOrigins": ["http://localhost:6173"],
                    "activeRedirectUris": ["http://localhost:6173/*"],
                }
            ],
            "users": ["test"],
        }

    def test_fresh_realm_creates_selected_client_and_user(self):
        api = FakeApi({"realm": "fixture"})
        reconcile_keycloak.RealmReconciler(api).reconcile(
            self.desired, self.config, set()
        )
        self.assertEqual(len(api.clients), 1)
        self.assertEqual(len(api.users), 1)
        client = next(iter(api.clients.values()))
        self.assertIn("http://localhost:6173/*", client["redirectUris"])

    def test_stale_volume_updates_policy_changed_port_and_missing_user(self):
        stale_client = copy.deepcopy(self.desired["clients"][0])
        stale_client.update(
            {
                "id": "client-1",
                "name": "Stale name",
                "redirectUris": ["http://localhost:4173/*"],
                "webOrigins": ["http://localhost:4173"],
                "productOwned": "preserved",
            }
        )
        api = FakeApi(
            {
                "realm": "fixture",
                "displayName": "Old",
                "loginTheme": "keycloak",
                "productOwned": "preserved",
            },
            {"client-1": stale_client},
        )
        reconcile_keycloak.RealmReconciler(api).reconcile(
            self.desired, self.config, set()
        )
        self.assertEqual(api.realm_value["displayName"], "Fixture development")
        self.assertEqual(api.realm_value["loginTheme"], "baukit-accessible")
        self.assertEqual(api.realm_value["productOwned"], "preserved")
        self.assertIn("http://localhost:4173/*", api.clients["client-1"]["redirectUris"])
        self.assertIn("http://localhost:6173/*", api.clients["client-1"]["redirectUris"])
        self.assertEqual(api.clients["client-1"]["productOwned"], "preserved")
        self.assertEqual(len(api.users), 1)

    def test_changed_client_updates_selected_fields_and_preserves_unknown_fields(self):
        existing = copy.deepcopy(self.desired["clients"][0])
        existing.update({"id": "client-1", "name": "Old", "unknown": {"keep": True}})
        api = FakeApi({"realm": "fixture"}, {"client-1": existing})
        reconcile_keycloak.RealmReconciler(api).reconcile(
            self.desired, {**self.config, "users": []}, set()
        )
        self.assertEqual(api.clients["client-1"]["name"], "Desired name")
        self.assertEqual(api.clients["client-1"]["unknown"], {"keep": True})

    def test_existing_user_password_changes_only_when_requested(self):
        user = {"id": "user-1", "username": "test", "enabled": True}
        api = FakeApi({"realm": "fixture"}, users={"user-1": user})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        config = {**self.config, "clients": []}
        reconciler.reconcile(self.desired, config, set())
        self.assertEqual(api.password_resets, [])
        reconciler.reconcile(self.desired, config, {"test"})
        self.assertEqual(len(api.password_resets), 1)

    def test_repeated_run_is_idempotent(self):
        api = FakeApi({"realm": "fixture"})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile(self.desired, self.config, set())
        api.updates.clear()
        reconciler.reconcile(self.desired, self.config, set())
        self.assertEqual(api.updates, [])


class RecoveryTests(unittest.TestCase):
    def test_lost_administrator_uses_recovery_then_removes_it(self):
        events = []

        def authenticate(username, password):
            if username == "admin":
                raise reconcile_keycloak.AuthenticationError("lost")
            events.append("temporary-authenticated")
            return "token"

        recovered = reconcile_keycloak.run_with_recovery(
            authenticate,
            ("admin", "private"),
            lambda token: events.append("reconciled"),
            lambda username, password: events.append("recovery-started"),
            lambda token, username, password: events.append("admin-repaired"),
            lambda token, username: events.append("temporary-removed"),
        )
        self.assertTrue(recovered)
        self.assertEqual(
            events,
            [
                "recovery-started",
                "temporary-authenticated",
                "reconciled",
                "admin-repaired",
                "temporary-removed",
            ],
        )

    def test_interrupted_recovery_still_repairs_and_cleans_up(self):
        events = []

        def authenticate(username, password):
            if username == "admin":
                raise reconcile_keycloak.AuthenticationError("lost")
            return "token"

        with self.assertRaises(KeyboardInterrupt):
            reconcile_keycloak.run_with_recovery(
                authenticate,
                ("admin", "private"),
                lambda token: (_ for _ in ()).throw(KeyboardInterrupt()),
                lambda username, password: events.append("recovery-started"),
                lambda token, username, password: events.append("admin-repaired"),
                lambda token, username: events.append("temporary-removed"),
            )
        self.assertEqual(events, ["recovery-started", "admin-repaired", "temporary-removed"])

    def test_cleanup_failure_is_reported_without_a_secret(self):
        def authenticate(username, password):
            if username == "admin":
                raise reconcile_keycloak.AuthenticationError("lost")
            return "token"

        with self.assertRaisesRegex(
            reconcile_keycloak.ReconcileError,
            "temporary recovery administrator cleanup failed",
        ) as raised:
            reconcile_keycloak.run_with_recovery(
                authenticate,
                ("admin", "private-secret"),
                lambda token: None,
                lambda username, password: None,
                lambda token, username, password: None,
                lambda token, username: (_ for _ in ()).throw(RuntimeError("private-secret")),
            )
        self.assertNotIn("private-secret", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
