from __future__ import annotations

import copy
import importlib.util
import json
import re
import sys
import tempfile
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
    def __init__(self, realm, clients=None, users=None, scopes=None):
        self.scopes = copy.deepcopy(scopes or {})
        self.bindings = {}
        self.realm_value = copy.deepcopy(realm)
        self.clients = copy.deepcopy(clients or {})
        self.users = copy.deepcopy(users or {})
        self.roles = {
            "offline_access": {"id": "offline", "name": "offline_access"},
            "admin": {"id": "admin-role", "name": "admin"},
        }
        self.user_roles = {identity: [] for identity in self.users}
        self.updates = []
        self.client_updates = []
        self.password_resets = []

    def realm(self, realm):
        return copy.deepcopy(self.realm_value)

    def update_realm(self, realm, value):
        self.realm_value = copy.deepcopy(value)
        self.updates.append(("realm", realm))

    def find(self, realm, collection, key, value):
        values = {"clients": self.clients, "users": self.users, "client-scopes": self.scopes}[collection]
        return [
            copy.deepcopy(item)
            for item in values.values()
            if item.get(key) == value
        ]

    def get(self, realm, collection, identity):
        values = {"clients": self.clients, "users": self.users, "client-scopes": self.scopes}[collection]
        return copy.deepcopy(values[identity])

    def create(self, realm, collection, value):
        values = {"clients": self.clients, "users": self.users, "client-scopes": self.scopes}[collection]
        identity = f"{collection}-{len(values) + 1}"
        created = copy.deepcopy(value)
        created["id"] = identity
        created.pop("credentials", None)
        created.pop("realmRoles", None)
        values[identity] = created
        self.user_roles.setdefault(identity, [])
        self.updates.append((f"create-{collection}", value.get("clientId", value.get("username"))))

    def update(self, realm, collection, identity, value):
        values = {"clients": self.clients, "users": self.users, "client-scopes": self.scopes}[collection]
        if collection == "clients":
            self.client_updates.append(copy.deepcopy(value))
            values[identity].update(copy.deepcopy(value))
        elif collection == "client-scopes":
            values[identity].update({key: copy.deepcopy(value) for key, value in value.items() if key != "protocolMappers"})
        else:
            values[identity] = copy.deepcopy(value)
        self.updates.append((f"update-{collection}", identity))

    def delete(self, realm, collection, identity):
        values = {"clients": self.clients, "users": self.users, "client-scopes": self.scopes}[collection]
        del values[identity]

    def scope_bindings(self, realm, collection, client=None):
        return copy.deepcopy(self.bindings.get((collection, client), []))

    def scope_mappers(self, realm, scope):
        return copy.deepcopy(self.scopes[scope].get("protocolMappers", []))

    def create_scope_mapper(self, realm, scope, mapper):
        mappers = self.scopes[scope].setdefault("protocolMappers", [])
        mappers.append({**copy.deepcopy(mapper), "id": f"mapper-{len(mappers) + 1}"})
        self.updates.append(("create-mapper", scope))

    def update_scope_mapper(self, realm, scope, identity, mapper):
        mappers = self.scopes[scope]["protocolMappers"]
        index = next(index for index, value in enumerate(mappers) if value["id"] == identity)
        mappers[index] = copy.deepcopy(mapper)
        self.updates.append(("update-mapper", scope, identity))

    def add_scope_binding(self, realm, collection, scope, client=None):
        self.bindings.setdefault((collection, client), []).append(copy.deepcopy(self.scopes[scope]))
        self.updates.append(("scope-binding", collection, client, scope))

    def reset_password(self, realm, user_id, credential):
        self.password_resets.append((user_id, copy.deepcopy(credential)))

    def realm_role(self, realm, role_name):
        return copy.deepcopy(self.roles[role_name])

    def user_realm_roles(self, realm, user_id):
        return copy.deepcopy(self.user_roles[user_id])

    def add_user_realm_roles(self, realm, user_id, roles):
        self.user_roles[user_id].extend(copy.deepcopy(roles))


class RealmReconcilerTests(unittest.TestCase):
    def test_scope_search_filters_names_and_mapper_requests_use_the_child_endpoint(self):
        class RecordingApi(reconcile_keycloak.KeycloakApi):
            def __init__(self):
                super().__init__("https://identity.example")
                self.requests = []

            def request(self, method, path, payload=None, query=None, form=None):
                self.requests.append((method, path, payload))
                return [{"id": "read", "name": "read"}, {"id": "other", "name": "other"}]

        api = RecordingApi()
        self.assertEqual(
            api.find("fixture", "client-scopes", "name", "read"),
            [{"id": "read", "name": "read"}],
        )
        api.scope_mappers("realm/name", "scope/id")
        api.create_scope_mapper("realm/name", "scope/id", {"name": "audience"})
        api.update_scope_mapper("realm/name", "scope/id", "mapper/id", {"name": "audience"})
        prefix = "/admin/realms/realm%2Fname/client-scopes/scope%2Fid/protocol-mappers/models"
        self.assertEqual(
            api.requests[1:],
            [
                ("GET", prefix, None),
                ("POST", prefix, {"name": "audience"}),
                ("PUT", prefix + "/mapper%2Fid", {"name": "audience"}),
            ],
        )

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

    def test_selected_email_and_password_fields_load_and_reconcile(self):
        for field in ("verifyEmail", "resetPasswordAllowed"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "reconcile.json"
                path.write_text(json.dumps({**self.config, "realmFields": [field]}))
                config = reconcile_keycloak.load_reconcile_config(path)
                for desired_value in (True, False):
                    api = FakeApi({"realm": "fixture", field: not desired_value})
                    reconcile_keycloak.RealmReconciler(api).reconcile(
                        {**self.desired, field: desired_value}, config, set()
                    )
                    self.assertEqual(api.realm_value[field], desired_value)

    def test_template_realm_settings_are_reconcilable(self):
        realm_source = (SCRIPT_DIRECTORY.parent / "keycloak" / "realm.json").read_text()
        realm_fields = realm_source.split('  "users":', 1)[0]
        fields = set(re.findall(r'^  "([^"]+)":', realm_fields, re.MULTILINE))
        self.assertEqual(fields - {"realm"} - reconcile_keycloak.RECONCILABLE_REALM_FIELDS, set())

    def test_mcp_scopes_are_created_updated_bound_and_reconciled_idempotently(self):
        desired = copy.deepcopy(self.desired)
        desired["clientScopes"] = [
            {
                "name": "read",
                "protocol": "openid-connect",
                "attributes": {"include.in.token.scope": "true"},
            },
            {"name": "basic", "protocol": "openid-connect"},
        ]
        desired["defaultDefaultClientScopes"] = ["basic"]
        desired["defaultOptionalClientScopes"] = ["read"]
        desired["clients"][0]["defaultClientScopes"] = ["basic"]
        desired["clients"][0]["optionalClientScopes"] = ["read"]
        config = copy.deepcopy(self.config)
        config["realmFields"] += sorted(reconcile_keycloak.SCOPE_REALM_FIELDS)
        api = FakeApi(
            {"realm": "fixture"},
            scopes={
                "scope-read": {
                    "id": "scope-read",
                    "name": "read",
                    "protocol": "openid-connect",
                    "attributes": {"include.in.token.scope": "false"},
                    "productOwned": "preserved",
                }
            },
        )
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile(desired, config, set())
        self.assertEqual(
            api.scopes["scope-read"]["attributes"]["include.in.token.scope"], "true"
        )
        self.assertEqual(api.scopes["scope-read"]["productOwned"], "preserved")
        self.assertEqual(
            {scope["name"] for scope in api.scopes.values()}, {"read", "basic"}
        )
        client_id = next(iter(api.clients))
        for collection, client, name in [
            ("default-default-client-scopes", None, "basic"),
            ("default-optional-client-scopes", None, "read"),
            ("default-client-scopes", client_id, "basic"),
            ("optional-client-scopes", client_id, "read"),
        ]:
            self.assertEqual(
                [scope["name"] for scope in api.bindings[(collection, client)]], [name]
            )
        api.updates.clear()
        reconciler.reconcile(desired, config, set())
        self.assertEqual(api.updates, [])

    def test_missing_client_scope_rejects_reconciliation(self):
        api = FakeApi({"realm": "fixture"})
        with self.assertRaisesRegex(reconcile_keycloak.ReconcileError, "absent or ambiguous"):
            reconcile_keycloak.RealmReconciler(api).reconcile_scope_bindings(
                "fixture", "optional-client-scopes", ["missing"], "client"
            )

    def test_scope_mapper_updates_preserve_server_ids_and_unowned_mappers(self):
        existing = {
            "id": "scope-basic",
            "name": "basic",
            "protocol": "openid-connect",
            "protocolMappers": [
                {
                    "id": "mapper-sub",
                    "name": "sub",
                    "config": {"access.token.claim": "false", "product.claim": "true"},
                },
                {"id": "mapper-extra", "name": "extra", "config": {}},
            ],
        }
        desired = {
            "name": "basic",
            "protocol": "openid-connect",
            "protocolMappers": [
                {
                    "id": "exported-id",
                    "name": "sub",
                    "config": {"access.token.claim": "true"},
                },
                {
                    "name": "audience",
                    "protocol": "openid-connect",
                    "protocolMapper": "oidc-audience-mapper",
                    "config": {
                        "included.custom.audience": "https://mcp.example/mcp",
                        "access.token.claim": "true",
                    },
                },
            ],
        }
        api = FakeApi({"realm": "fixture"}, scopes={"scope-basic": existing})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile_scope("fixture", desired)
        mappers = api.scopes["scope-basic"]["protocolMappers"]
        self.assertEqual(mappers[0]["id"], "mapper-sub")
        self.assertEqual(mappers[0]["config"]["access.token.claim"], "true")
        self.assertEqual(mappers[0]["config"]["product.claim"], "true")
        self.assertEqual(mappers[1], existing["protocolMappers"][1])
        self.assertEqual(
            mappers[2]["config"]["included.custom.audience"], "https://mcp.example/mcp"
        )
        self.assertIn(("update-mapper", "scope-basic", "mapper-sub"), api.updates)
        self.assertIn(("create-mapper", "scope-basic"), api.updates)
        api.updates.clear()
        reconciler.reconcile_scope("fixture", desired)
        self.assertEqual(api.updates, [])

    def test_scope_binding_names_reject_invalid_values(self):
        for value in (None, "read", [""], [7]):
            with self.subTest(value=value), self.assertRaisesRegex(
                reconcile_keycloak.ReconcileError, "array of non-empty strings"
            ):
                reconcile_keycloak.scope_names(value, "optionalClientScopes")

    def test_client_mapper_ids_and_unowned_scope_bindings_survive_updates(self):
        desired = copy.deepcopy(self.desired["clients"][0])
        desired["protocolMappers"] = [
            {"name": "audience", "config": {"access.token.claim": "true"}}
        ]
        desired["optionalClientScopes"] = ["read"]
        existing = {
            **desired,
            "id": "client",
            "optionalClientScopes": ["read", "private-scope"],
            "protocolMappers": [
                {
                    "id": "mapper",
                    "name": "audience",
                    "config": {"access.token.claim": "false"},
                }
            ],
        }
        api = FakeApi({"realm": "fixture"}, clients={"client": existing})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile_client("fixture", desired, self.config["clients"][0])
        updated = api.clients["client"]
        self.assertEqual(updated["protocolMappers"][0]["id"], "mapper")
        self.assertEqual(
            updated["protocolMappers"][0]["config"]["access.token.claim"], "true"
        )
        self.assertEqual(updated["optionalClientScopes"], ["read", "private-scope"])
        api.updates.clear()
        reconciler.reconcile_client("fixture", desired, self.config["clients"][0])
        self.assertEqual(api.updates, [])

    def test_client_attributes_preserve_keycloak_defaults_and_repair_declared_settings(self):
        desired = copy.deepcopy(self.desired["clients"][0])
        existing = {
            **desired,
            "id": "client",
            "attributes": {
                "pkce.code.challenge.method": "plain",
                "realm_client": "false",
                "post.logout.redirect.uris": "+",
            },
        }
        api = FakeApi({"realm": "fixture"}, clients={"client": existing})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile_client("fixture", desired, self.config["clients"][0])
        self.assertEqual(
            api.clients["client"]["attributes"],
            {
                "pkce.code.challenge.method": "S256",
                "realm_client": "false",
                "post.logout.redirect.uris": "+",
            },
        )
        self.assertEqual(api.updates, [("update-clients", "client")])
        api.updates.clear()
        reconciler.reconcile_client("fixture", desired, self.config["clients"][0])
        self.assertEqual(api.updates, [])

    def test_unknown_realm_field_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "reconcile.json"
            path.write_text(json.dumps({**self.config, "realmFields": ["smtpServer"]}))
            with self.assertRaisesRegex(reconcile_keycloak.ReconcileError, "smtpServer"):
                reconcile_keycloak.load_reconcile_config(path)

    def test_fresh_realm_creates_selected_client_and_user(self):
        api = FakeApi({"realm": "fixture"})
        reconcile_keycloak.RealmReconciler(api).reconcile(
            self.desired, self.config, set()
        )
        self.assertEqual(len(api.clients), 1)
        self.assertEqual(len(api.users), 1)
        client = next(iter(api.clients.values()))
        self.assertIn("http://localhost:6173/*", client["redirectUris"])

    def test_confidential_clients_need_no_browser_urls(self):
        policy = json.loads((TEST_DIRECTORY / "fixtures/development-policy.json").read_text())
        realm = json.loads((TEST_DIRECTORY / "fixtures/development-realm.json").read_text())
        for service_account in (False, True):
            with self.subTest(service_account=service_account):
                desired = {"clientId": "backend", "publicClient": False,
                           "serviceAccountsEnabled": service_account,
                           "directAccessGrantsEnabled": False, "secret": "creation-secret"}
                candidate = {**realm, "clients": [*realm["clients"], desired], "users": []}
                config = {**self.config, "clients": [{"clientId": "backend",
                          "activeOrigins": [], "activeRedirectUris": []}], "users": []}
                reconcile_keycloak.validate_inputs(candidate, policy, config)

    def test_confidential_creation_keeps_secret_and_reconciliation_preserves_rotation(self):
        desired = {"clientId": "backend", "publicClient": False,
                   "serviceAccountsEnabled": True, "secret": "creation-secret"}
        selection = {"clientId": "backend", "activeOrigins": [], "activeRedirectUris": []}
        api = FakeApi({"realm": "fixture"})
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile_client("fixture", desired, selection)
        identity, created = next(iter(api.clients.items()))
        self.assertEqual(created["secret"], "creation-secret")
        created["secret"] = "rotated-secret"
        reconciler.reconcile_client("fixture", {**desired, "name": "Updated"}, selection)
        self.assertEqual(api.clients[identity]["name"], "Updated")
        self.assertEqual(api.clients[identity]["secret"], "rotated-secret")
        self.assertNotIn("secret", api.client_updates[0])
        api.updates.clear()
        reconciler.reconcile_client("fixture", {**desired, "name": "Updated"}, selection)
        self.assertEqual(api.updates, [])

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
