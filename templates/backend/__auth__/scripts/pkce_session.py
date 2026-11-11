"""Sign in with `pkce-login.py` from product QA, seed and smoke scripts."""

from __future__ import annotations

import http.cookiejar
import importlib.util
import os
import sys
import urllib.request
from pathlib import Path
from types import ModuleType

MODULE_NAME = "pkce_login"
MODULE_PATH = Path(__file__).with_name("pkce-login.py")
DEFAULT_SCOPE = "openid profile email"


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


def sign_in(
    issuer: str,
    *,
    client_id: str,
    redirect_uri: str,
    scope: str = DEFAULT_SCOPE,
    username: str | None = None,
    password: str | None = None,
    resource: str | None = None,
) -> str:
    """Run the headless authorization-code + PKCE login and return the access token.

    The user comes from `BAUKIT_QA_USERNAME` and `BAUKIT_QA_PASSWORD`, which default to the
    checked-in development user.
    """
    module = load_pkce_login()
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(
            http.cookiejar.CookieJar(policy=module.LocalDevelopmentCookiePolicy())
        ),
        module.CallbackRedirectHandler(redirect_uri),
    )
    extra = {} if resource is None else {"resource": resource}
    request = module.LoginRequest(
        username=username or os.environ.get("BAUKIT_QA_USERNAME", "test"),
        password=password or os.environ.get("BAUKIT_QA_PASSWORD", "development-password"),
        client_id=client_id,
        redirect_uri=redirect_uri,
        scope=scope,
        **extra,
    )
    return module.login(opener, module.discover(opener, issuer), request)
