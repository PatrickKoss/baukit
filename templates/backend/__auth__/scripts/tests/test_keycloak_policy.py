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

    def test_production_fixture_declares_and_passes_its_class(self):
        failures = keycloak_policy.validate_realm(
            self.fixture("production-realm.json"),
            self.fixture("production-policy.json"),
            "production",
        )
        self.assertEqual(failures, [])

    def test_environment_class_is_not_inferred_from_realm_name(self):
        failures = keycloak_policy.validate_realm(
            self.fixture("development-realm.json"),
            self.fixture("development-policy.json"),
            "production",
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
        realm = self.fixture("production-realm.json")
        realm["sslRequired"] = "none"
        client = realm["clients"][0]
        client["directAccessGrantsEnabled"] = True
        client["attributes"] = {}
        failures = keycloak_policy.validate_realm(
            realm, self.fixture("production-policy.json"), "production"
        )
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


if __name__ == "__main__":
    unittest.main()
