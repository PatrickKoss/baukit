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


class LabPolicyTests(unittest.TestCase):
    def fixture(self, name: str):
        return keycloak_policy.load_json(TEST_DIRECTORY / "fixtures" / name)

    def validate(self, realm=None, policy=None):
        return keycloak_policy.validate_realm(
            realm or self.fixture("lab-realm.json"),
            policy or self.fixture("lab-policy.json"),
            "lab",
        )

    def client(self, realm, client_id: str):
        return next(client for client in realm["clients"] if client["clientId"] == client_id)

    def test_lab_fixture_passes(self):
        self.assertEqual(self.validate(), [])

    def test_development_policy_is_not_a_lab_policy(self):
        failures = keycloak_policy.validate_realm(
            self.fixture("lab-realm.json"), self.fixture("development-policy.json"), "lab"
        )
        self.assertTrue(any("environmentClass must be 'lab'" in failure for failure in failures))

    def test_lab_policy_cannot_allow_loopback_http_or_skip_tls(self):
        policy = self.fixture("lab-policy.json")
        policy["redirectUris"]["allowDevelopmentLoopbackHttp"] = True
        policy["minimumTls"] = "none"
        failures = self.validate(policy=policy)
        self.assertIn(
            "lab policy redirectUris.allowDevelopmentLoopbackHttp must be false", failures
        )
        self.assertIn("lab policy minimumTls must be external or all", failures)

    def test_lab_policy_mcp_resource_must_be_a_lab_https_mcp_url(self):
        for resource in ("http://localhost:8080/mcp", "https://api.example.test/other"):
            policy = self.fixture("lab-policy.json")
            policy["mcpResourceUrl"] = resource
            self.assertEqual(
                self.validate(policy=policy),
                ["lab policy mcpResourceUrl must be a non-loopback https URL ending in /mcp"],
            )

    def test_default_and_known_passwords_are_rejected(self):
        for value in ("development-password", "Password", "fixture-user"):
            realm = self.fixture("lab-realm.json")
            realm["users"][0]["credentials"][0]["value"] = value
            self.assertEqual(
                self.validate(realm),
                ["user 'fixture-user' uses a default or known password"],
            )

    def test_literal_passwords_must_use_substitution(self):
        realm = self.fixture("lab-realm.json")
        realm["users"][0]["credentials"][0]["value"] = "Correct-Horse-Battery-9"
        failures = self.validate(realm)
        self.assertEqual(len(failures), 1)
        self.assertIn("stores a literal password", failures[0])

    def test_file_substitution_and_other_credential_types_pass(self):
        realm = self.fixture("lab-realm.json")
        realm["users"][0]["credentials"] = [
            {"type": "password", "value": "$(file:/run/secrets/user-password)"},
            {"type": "otp", "value": "development-password"},
        ]
        self.assertEqual(self.validate(realm), [])

    def test_loopback_redirect_uris_are_rejected(self):
        realm = self.fixture("lab-realm.json")
        self.client(realm, "fixture-web")["redirectUris"] = [
            "http://localhost:5173/*",
            "https://127.0.0.1:8443/callback",
        ]
        failures = self.validate(realm)
        self.assertEqual(len(failures), 2)
        self.assertIn("uses HTTP outside an allowed development loopback address", failures[0])
        self.assertIn("points at a loopback host", failures[1])

    def test_plain_http_lab_redirect_is_rejected(self):
        realm = self.fixture("lab-realm.json")
        self.client(realm, "fixture-web")["redirectUris"] = ["http://fixture.lab.example.test/*"]
        failures = self.validate(realm)
        self.assertEqual(len(failures), 1)
        self.assertIn("uses HTTP", failures[0])

    def test_public_client_without_pkce_s256_is_rejected(self):
        realm = self.fixture("lab-realm.json")
        self.client(realm, "fixture-mcp")["attributes"] = {"pkce.code.challenge.method": "plain"}
        self.assertEqual(
            self.validate(realm), ["public client 'fixture-mcp' must require PKCE S256"]
        )

    def test_mcp_audience_must_point_at_the_lab_url(self):
        realm = self.fixture("lab-realm.json")
        mapper = self.client(realm, "fixture-mcp")["protocolMappers"][0]
        mapper["config"]["included.custom.audience"] = "http://localhost:8080/mcp"
        self.assertEqual(
            self.validate(realm),
            [
                "client 'fixture-mcp' MCP audience 'http://localhost:8080/mcp' must be "
                "'https://api.fixture.lab.example.test/mcp'",
                "no client maps the MCP audience 'https://api.fixture.lab.example.test/mcp'",
            ],
        )

    def test_missing_mcp_audience_mapper_is_rejected(self):
        realm = self.fixture("lab-realm.json")
        del self.client(realm, "fixture-mcp")["protocolMappers"]
        self.assertEqual(
            self.validate(realm),
            ["no client maps the MCP audience 'https://api.fixture.lab.example.test/mcp'"],
        )

    def test_mcp_audience_without_a_policy_url_is_rejected(self):
        policy = self.fixture("lab-policy.json")
        del policy["mcpResourceUrl"]
        failures = self.validate(policy=policy)
        self.assertEqual(len(failures), 1)
        self.assertIn("the lab policy sets no mcpResourceUrl", failures[0])

    def test_realm_without_mcp_passes_without_a_policy_url(self):
        realm = self.fixture("lab-realm.json")
        realm["clients"] = [c for c in realm["clients"] if c["clientId"] != "fixture-mcp"]
        policy = self.fixture("lab-policy.json")
        del policy["mcpResourceUrl"]
        self.assertEqual(self.validate(realm, policy), [])

    def test_development_class_keeps_literal_passwords_and_loopback(self):
        realm = self.fixture("development-realm.json")
        realm["users"] = [
            {
                "username": "dev",
                "credentials": [{"type": "password", "value": "development-password"}],
            }
        ]
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("development-policy.json"), "development"
        )
        self.assertEqual(failures, [])

    def test_cli_accepts_the_lab_class(self):
        fixtures = TEST_DIRECTORY / "fixtures"
        status = keycloak_policy.main(
            [
                "--realm",
                str(fixtures / "lab-realm.json"),
                "--policy",
                str(fixtures / "lab-policy.json"),
                "--environment-class",
                "lab",
            ]
        )
        self.assertEqual(status, 0)


if __name__ == "__main__":
    unittest.main()
