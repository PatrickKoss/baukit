from __future__ import annotations

import importlib.util
import sys
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
    def test_sign_in_builds_the_login_request_from_keywords(self):
        with (
            mock.patch.dict(
                "os.environ",
                {"BAUKIT_QA_USERNAME": "qa-user", "BAUKIT_QA_PASSWORD": "qa-secret"},
                clear=True,
            ),
            mock.patch.object(pkce, "discover", return_value={"token_endpoint": "t"}),
            mock.patch.object(pkce, "login", return_value="token") as login,
        ):
            token = pkce_session.sign_in(
                "http://issuer.test/realms/app",
                client_id="app-mobile",
                redirect_uri="app://oauth",
                resource="http://api.test",
            )
        self.assertEqual(token, "token")
        request = login.call_args.args[2]
        self.assertEqual(request.password, "qa-secret")
        self.assertEqual(
            (request.username, request.client_id, request.redirect_uri, request.scope),
            ("qa-user", "app-mobile", "app://oauth", "openid profile email"),
        )
        self.assertEqual(request.resource, "http://api.test")

    def test_the_loader_registers_the_module_once(self):
        self.assertIs(sys.modules["pkce_login"], pkce)
        self.assertIs(pkce_session.load_pkce_login(), pkce)


if __name__ == "__main__":
    unittest.main()
