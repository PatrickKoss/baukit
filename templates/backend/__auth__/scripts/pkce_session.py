"""Sign in with `pkce-login.py` from product QA, seed and smoke scripts."""

from __future__ import annotations

import http.cookiejar
import importlib.util
import json
import os
import sys
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType

MODULE_NAME = "pkce_login"
MODULE_PATH = Path(__file__).with_name("pkce-login.py")
DEFAULT_SCOPE = "openid profile email"
REALM_PATH = Path(__file__).resolve().parent.parent / "keycloak" / "realm.json"
DEVELOPMENT_USERNAME = "test"


def load_pkce_login() -> ModuleType:
    """Load `pkce-login.py` once and register it so its dataclasses resolve their module."""
    loaded = sys.modules.get(MODULE_NAME)
    if loaded is not None:
        return loaded
    spec = importlib.util.spec_from_file_location(MODULE_NAME, MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[MODULE_NAME] = module
    spec.loader.exec_module(module)
    return module


@dataclass(frozen=True)
class Credentials:
    username: str
    password: str

    @classmethod
    def development(cls, realm_path: Path = REALM_PATH) -> Credentials:
        """Read the development user and its password credential from the Keycloak realm."""
        realm = json.loads(realm_path.read_text())
        for user in realm.get("users", []):
            if user.get("username") != DEVELOPMENT_USERNAME:
                continue
            for credential in user.get("credentials", []):
                if credential.get("type") == "password" and credential.get("value"):
                    return cls(DEVELOPMENT_USERNAME, credential["value"])
        raise ValueError(f"{realm_path} has no `{DEVELOPMENT_USERNAME}` user with a password")

    @classmethod
    def from_env(cls) -> Credentials:
        """Read `BAUKIT_QA_USERNAME` and `BAUKIT_QA_PASSWORD`, else the development user."""
        username = os.environ.get("BAUKIT_QA_USERNAME")
        password = os.environ.get("BAUKIT_QA_PASSWORD")
        if username is not None and password is not None:
            return cls(username, password)
        default = cls.development()
        return cls(username or default.username, password or default.password)


@dataclass(frozen=True)
class SignIn:
    issuer: str
    client_id: str
    redirect_uri: str
    credentials: Credentials
    scope: str = DEFAULT_SCOPE
    resource: str | None = None


def sign_in(request: SignIn) -> str:
    """Run the headless authorization-code + PKCE login and return the access token."""
    module = load_pkce_login()
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(
            http.cookiejar.CookieJar(policy=module.LocalDevelopmentCookiePolicy())
        ),
        module.CallbackRedirectHandler(request.redirect_uri),
    )
    extra = {} if request.resource is None else {"resource": request.resource}
    login_request = module.LoginRequest(
        username=request.credentials.username,
        password=request.credentials.password,
        client_id=request.client_id,
        redirect_uri=request.redirect_uri,
        scope=request.scope,
        **extra,
    )
    return module.login(opener, module.discover(opener, request.issuer), login_request)
