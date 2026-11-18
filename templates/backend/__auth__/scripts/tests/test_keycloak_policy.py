from __future__ import annotations

import copy
import importlib.util
import unittest
from pathlib import Path

TEST_DIRECTORY = Path(__file__).resolve().parent
SCRIPT_PATH = TEST_DIRECTORY.parent / "keycloak_policy.py"
SPEC = importlib.util.spec_from_file_location("keycloak_policy", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
keycloak_policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(keycloak_policy)


class KeycloakPolicyTests(unittest.TestCase):
    def fixture(self, name: str):
        return keycloak_policy.load_json(TEST_DIRECTORY / "fixtures" / name)

    def test_development_fixture_declares_and_passes_its_class(self):
        failures = keycloak_policy.validate_realm(
            self.fixture("development-realm.json"),
            self.fixture("development-policy.json"),
            "development",
        )
        self.assertEqual(failures, [])

    def test_policy_must_declare_the_requested_class(self):
        policy = self.fixture("development-policy.json")
        policy["environmentClass"] = "staging"
        failures = keycloak_policy.validate_realm(
            self.fixture("development-realm.json"), policy, "development"
        )
        self.assertTrue(any("environmentClass" in failure for failure in failures))

    def test_weakened_password_and_brute_force_policy_is_rejected(self):
        realm = self.fixture("development-realm.json")
        realm["passwordPolicy"] = "length(8) and maxLength(512)"
        realm["bruteForceProtected"] = False
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertTrue(any("at least 12" in failure for failure in failures))
        self.assertTrue(any("at most 128" in failure for failure in failures))
        self.assertTrue(any("notUsername" in failure for failure in failures))
        self.assertTrue(any("notEmail" in failure for failure in failures))
        self.assertTrue(any("bruteForceProtected" in failure for failure in failures))

    def test_weakened_tls_pkce_and_direct_grants_are_rejected(self):
        realm = self.fixture("development-realm.json")
        realm["sslRequired"] = "none"
        client = next(client for client in realm["clients"] if client.get("publicClient"))
        client["directAccessGrantsEnabled"] = True
        client["attributes"] = {}
        policy = self.fixture("development-policy.json")
        policy["minimumTls"] = "external"
        failures = keycloak_policy.validate_realm(realm, policy, "development")
        self.assertTrue(any("sslRequired" in failure for failure in failures))
        self.assertTrue(any("direct-access" in failure for failure in failures))
        self.assertTrue(any("PKCE S256" in failure for failure in failures))

    def test_unbounded_and_excess_redirect_uris_are_rejected(self):
        realm = copy.deepcopy(self.fixture("development-realm.json"))
        realm["clients"][1]["redirectUris"] = [
            "*",
            "http://*.example.test/*",
            "http://example.test/oauth",
            "https://one.example.test/oauth",
            "https://two.example.test/oauth",
        ]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertTrue(any("more than 4" in failure for failure in failures))
        self.assertTrue(any("unbounded wildcard" in failure for failure in failures))
        self.assertTrue(any("wildcard the host" in failure for failure in failures))
        self.assertTrue(any("outside an allowed" in failure for failure in failures))

    def admin_api_realm(self):
        realm = self.fixture("development-realm.json")
        realm["users"] = [
            {
                "username": "service-account-fixture-backend",
                "serviceAccountClientId": "fixture-backend",
                "clientRoles": {"realm-management": ["manage-users"]},
            }
        ]
        realm["clients"][0]["defaultClientScopes"] = ["basic", "kc-admin-api"]
        return realm

    def test_admin_api_client_with_the_scope_passes(self):
        realm = self.admin_api_realm()
        realm["clientScopes"] = [{"name": "basic"}, {"name": "kc-admin-api"}]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertEqual(failures, [])

    def test_admin_api_client_without_the_scope_is_rejected(self):
        realm = self.admin_api_realm()
        realm["clients"][0]["defaultClientScopes"] = ["basic"]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertEqual(
            failures,
            [
                "client 'fixture-backend' calls the admin API but lacks the kc-admin-api "
                "scope Keycloak 26.8.2 requires"
            ],
        )

    def test_admin_api_client_without_default_scopes_is_rejected(self):
        realm = self.admin_api_realm()
        del realm["clients"][0]["defaultClientScopes"]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertTrue(any("lacks the kc-admin-api scope" in failure for failure in failures))

    def test_declared_client_scopes_must_include_the_admin_api_scope(self):
        realm = self.admin_api_realm()
        realm["clientScopes"] = [{"name": "basic"}]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("clientScopes without 'kc-admin-api'", failures[0])

    def test_service_account_without_realm_management_roles_needs_no_scope(self):
        realm = self.admin_api_realm()
        realm["users"][0]["clientRoles"] = {}
        realm["clients"][0]["defaultClientScopes"] = ["basic"]
        realm["clientScopes"] = [{"name": "basic"}]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertEqual(failures, [])


if __name__ == "__main__":
    unittest.main()
