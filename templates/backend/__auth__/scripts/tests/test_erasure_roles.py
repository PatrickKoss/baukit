from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import keycloak_policy
import reconcile_keycloak


class RoleApi:
    def __init__(self):
        self.roles = []
        self.additions = []

    def find(self, realm, collection, key, value):
        return [{"id": value, "clientId": value}]

    def service_account(self, realm, client_id):
        return {"id": "backend-account"}

    def client_role(self, realm, client_id, role):
        return {"id": f"role-{role}", "name": role}

    def user_client_roles(self, realm, user_id, client_id):
        return copy.deepcopy(self.roles)

    def add_user_client_roles(self, realm, user_id, client_id, roles):
        self.roles.extend(roles)
        self.additions.append((user_id, client_id, copy.deepcopy(roles)))


class ErasureRoleTests(unittest.TestCase):
    def test_reconciles_manage_users_once(self):
        api = RoleApi()
        reconciler = reconcile_keycloak.RealmReconciler(api)
        reconciler.reconcile_service_account_roles("fixture", "backend", ["manage-users"])
        reconciler.reconcile_service_account_roles("fixture", "backend", ["manage-users"])
        self.assertEqual(api.additions, [("backend-account", "realm-management", [{"id": "role-manage-users", "name": "manage-users"}])])

    def test_policy_requires_the_declared_mapping(self):
        fixtures = Path(__file__).resolve().parent / "fixtures"
        policy = keycloak_policy.load_json(fixtures / "development-policy.json")
        realm = keycloak_policy.load_json(fixtures / "development-realm.json")
        policy["serviceAccountRoles"] = {"backend": ["manage-users"]}
        realm["clients"].append({"clientId": "backend", "publicClient": False, "directAccessGrantsEnabled": False, "serviceAccountsEnabled": True})
        self.assertTrue(any("missing required" in issue for issue in keycloak_policy.validate_realm(realm, policy, "development")))
        realm.setdefault("users", []).append({"serviceAccountClientId": "backend", "clientRoles": {"realm-management": ["manage-users"]}})
        self.assertEqual(keycloak_policy.validate_realm(realm, policy, "development"), [])
        realm["clients"][-1]["publicClient"] = True
        self.assertTrue(any("confidential service account" in issue for issue in keycloak_policy.validate_realm(realm, policy, "development")))

    def test_config_and_policy_must_agree(self):
        fixtures = Path(__file__).resolve().parent / "fixtures"
        realm = keycloak_policy.load_json(fixtures / "development-realm.json")
        policy = keycloak_policy.load_json(fixtures / "development-policy.json")
        realm.setdefault("users", [])
        config = {"clients": [], "users": [], "serviceAccountRoles": {"backend": ["manage-users"]}}
        with self.assertRaisesRegex(reconcile_keycloak.ReconcileError, "must match policy"):
            reconcile_keycloak.validate_inputs(realm, policy, config)

    def test_malformed_realm_role_mapping_returns_a_policy_failure(self):
        fixtures = Path(__file__).resolve().parent / "fixtures"
        policy = keycloak_policy.load_json(fixtures / "development-policy.json")
        realm = keycloak_policy.load_json(fixtures / "development-realm.json")
        policy["serviceAccountRoles"] = {"backend": ["manage-users"]}
        realm["clients"].append(None)
        realm["users"] = [None, {
            "serviceAccountClientId": "backend",
            "clientRoles": "manage-users",
        }]
        failures = keycloak_policy.validate_realm(realm, policy, "development")
        self.assertTrue(any("confidential service account" in issue for issue in failures))
        self.assertTrue(any("missing required" in issue for issue in failures))
