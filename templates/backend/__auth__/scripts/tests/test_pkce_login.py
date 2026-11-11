from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
import urllib.error
import urllib.request
from email.message import Message
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location("pkce_session", SCRIPTS / "pkce_session.py")
assert SPEC is not None and SPEC.loader is not None
pkce_session = importlib.util.module_from_spec(SPEC)
sys.modules["pkce_session"] = pkce_session
SPEC.loader.exec_module(pkce_session)
pkce = pkce_session.load_pkce_login()

REDIRECT_CODES = (301, 302, 303, 307, 308)


def redirect_headers(location: str) -> Message:
    headers = Message()
    headers["Location"] = location
    return headers


class CallbackTests(unittest.TestCase):
    def test_native_callback_is_captured_before_urllib_rejects_the_scheme(self):
        handler = pkce.CallbackRedirectHandler("app://oauth")
        request = urllib.request.Request("https://identity.test/login")
        location = "app://oauth?code=authorization-code&state=expected-state"
        for code in REDIRECT_CODES:
            with self.subTest(code=code):
                redirect = getattr(handler, f"http_error_{code}")
                with self.assertRaises(pkce.CallbackCaptured) as captured:
                    redirect(request, None, code, "Redirect", redirect_headers(location))
                self.assertEqual(captured.exception.location, location)

    def test_http_callback_keeps_the_authorization_code_and_state(self):
        handler = pkce.CallbackRedirectHandler("http://localhost:18888/callback")
        location = "http://localhost:18888/callback?code=authorization-code&state=expected-state"
        with self.assertRaises(pkce.CallbackCaptured) as captured:
            handler.redirect_request(None, None, 302, "Redirect", {}, location)
        self.assertEqual(captured.exception.location, location)

    def test_unrelated_native_scheme_is_rejected(self):
        handler = pkce.CallbackRedirectHandler("app://oauth")
        request = urllib.request.Request("https://identity.test/login")
        for location in ("app://other", "other-app://oauth", "app://oauth/extra"):
            with self.subTest(location=location):
                with self.assertRaises(urllib.error.HTTPError) as rejected:
                    handler.http_error_302(
                        request, None, 302, "Redirect", redirect_headers(location)
                    )
                rejected.exception.close()


class SignInTests(unittest.TestCase):
    def sign_in_as(self, username, secret):
        request = pkce_session.SignIn(
            "http://issuer.test/realms/app",
            client_id="app-mobile",
            redirect_uri="app://oauth",
            credentials=pkce_session.Credentials(username, secret),
            resource="http://api.test",
        )
        with (
            mock.patch.object(pkce, "discover", return_value={"token_endpoint": "t"}),
            mock.patch.object(pkce, "login", side_effect=self.token_for) as login,
        ):
            token = pkce_session.sign_in(request)
        return token, login.call_args.args[2]

    @staticmethod
    def token_for(_opener, _metadata, request):
        return f"token-for-{request.username}"

    def test_sign_in_builds_the_login_request_from_the_call(self):
        token, request = self.sign_in_as("qa-user", "qa-secret")
        self.assertEqual(token, "token-for-qa-user")
        self.assertEqual(request.password, "qa-secret")
        self.assertEqual(
            (request.client_id, request.redirect_uri, request.scope, request.resource),
            ("app-mobile", "app://oauth", pkce_session.DEFAULT_SCOPE, "http://api.test"),
        )

    def test_two_users_in_one_process_get_their_own_tokens(self):
        trainer, _ = self.sign_in_as("trainer", "trainer-secret")
        athlete, _ = self.sign_in_as("athlete", "athlete-secret")
        self.assertNotEqual(trainer, athlete)

    def write_realm(self, users):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "realm.json"
        path.write_text(json.dumps({"users": users}))
        return path

    def test_development_credentials_come_from_the_realm(self):
        credentials = [{"type": "password", "value": "realm-secret"}]
        realm = self.write_realm(
            [
                {"username": "other", "credentials": []},
                {"username": "test", "credentials": credentials},
            ]
        )
        self.assertEqual(
            pkce_session.Credentials.development(realm),
            pkce_session.Credentials("test", "realm-secret"),
        )

    def test_development_credentials_fail_without_the_user_or_password(self):
        for users in ([], [{"username": "test", "credentials": []}]):
            with self.subTest(users=users), self.assertRaises(ValueError):
                pkce_session.Credentials.development(self.write_realm(users))

    def test_env_credentials_default_to_the_rendered_realm(self):
        realm = json.loads(pkce_session.REALM_PATH.read_text())
        user = next(entry for entry in realm["users"] if entry["username"] == "test")
        expected = pkce_session.Credentials("test", user["credentials"][0]["value"])
        with mock.patch.dict("os.environ", {}, clear=True):
            self.assertEqual(pkce_session.Credentials.from_env(), expected)

    def test_credentials_come_from_the_environment(self):
        env = {"BAUKIT_QA_USERNAME": "qa-user", "BAUKIT_QA_PASSWORD": "qa-secret"}
        with mock.patch.dict("os.environ", env, clear=True):
            credentials = pkce_session.Credentials.from_env()
        self.assertEqual(credentials, pkce_session.Credentials("qa-user", "qa-secret"))

    def test_the_loader_registers_the_module_once(self):
        self.assertIs(sys.modules["pkce_login"], pkce)
        self.assertIs(pkce_session.load_pkce_login(), pkce)


if __name__ == "__main__":
    unittest.main()
