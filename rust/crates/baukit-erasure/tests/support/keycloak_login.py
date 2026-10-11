"""Return PKCE and refresh token responses through a private temporary file."""

import base64
import hashlib
import html
import http.cookiejar
import json
from pathlib import Path
import runpy
import secrets
import sys
import time
import urllib.parse
import urllib.request

REQUEST_TIMEOUT_SECONDS = 20
CLOCK_POLL_INTERVAL_SECONDS = 0.01


def form_request(url, fields):
    return urllib.request.Request(
        url,
        data=urllib.parse.urlencode(fields).encode(),
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )


def authorization_callback(client, helpers, authorization_url, require_login):
    try:
        with client.open(authorization_url, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            parser = helpers["LoginFormParser"]()
            parser.feed(response.read().decode())
            login_page_url = response.geturl()
    except helpers["CallbackCaptured"] as callback:
        if require_login:
            raise RuntimeError("Keycloak skipped the required login form") from None
        return callback.location
    if not require_login:
        raise RuntimeError("Keycloak silent SSO unexpectedly required a login form")
    if not parser.action:
        raise RuntimeError("Keycloak login form was not found")
    fields = {
        **parser.hidden,
        "username": "test",
        "password": "development-password",
        "credentialId": "",
    }
    try:
        response = client.open(
            form_request(
                urllib.parse.urljoin(login_page_url, html.unescape(parser.action)),
                fields,
            ),
            timeout=REQUEST_TIMEOUT_SECONDS,
        )
    except helpers["CallbackCaptured"] as callback:
        return callback.location
    response.close()
    raise RuntimeError("Keycloak did not redirect to the PKCE callback")


def authorize(client, helpers, metadata, redirect_uri, require_login, prompt=None):
    verifier = secrets.token_urlsafe(64)
    challenge = (
        base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest())
        .decode()
        .rstrip("=")
    )
    state = secrets.token_urlsafe(24)
    parameters = {
        "client_id": "reconcile-fixture-web",
        "redirect_uri": redirect_uri,
        "response_type": "code",
        "scope": "openid",
        "state": state,
        "code_challenge": challenge,
        "code_challenge_method": "S256",
    }
    if prompt is not None:
        parameters["prompt"] = prompt
    authorization_url = (
        metadata["authorization_endpoint"] + "?" + urllib.parse.urlencode(parameters)
    )
    callback = authorization_callback(client, helpers, authorization_url, require_login)
    returned = urllib.parse.parse_qs(urllib.parse.urlsplit(callback).query)
    if returned.get("state") != [state] or not returned.get("code"):
        raise RuntimeError("PKCE callback omitted the code or changed the state")
    return helpers["open_json"](
        client,
        form_request(
            metadata["token_endpoint"],
            {
                "grant_type": "authorization_code",
                "client_id": "reconcile-fixture-web",
                "redirect_uri": redirect_uri,
                "code": returned["code"][0],
                "code_verifier": verifier,
            },
        ),
    )


def wait_for_later_second(second):
    deadline = time.monotonic() + REQUEST_TIMEOUT_SECONDS
    while True:
        remaining = second + 1 - time.time()
        if remaining <= 0:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError("The clock did not advance past the first login second")
        time.sleep(min(remaining, CLOCK_POLL_INTERVAL_SECONDS))


def main():
    helpers = runpy.run_path(str(Path(sys.argv[1]) / "scripts/pkce-login.py"))
    redirect_uri = "http://localhost:5173/auth/callback"
    client = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(
            http.cookiejar.CookieJar(policy=helpers["LocalDevelopmentCookiePolicy"]())
        ),
        helpers["CallbackRedirectHandler"](redirect_uri),
    )
    metadata = helpers["discover"](client, sys.argv[2])
    initial = authorize(client, helpers, metadata, redirect_uri, require_login=True)
    wait_for_later_second(int(time.time()))
    refreshed = helpers["open_json"](
        client,
        form_request(
            metadata["token_endpoint"],
            {
                "grant_type": "refresh_token",
                "client_id": "reconcile-fixture-web",
                "refresh_token": initial["refresh_token"],
            },
        ),
    )
    tokens = {
        "initial": initial,
        "refreshed": refreshed,
        "silent": authorize(
            client, helpers, metadata, redirect_uri, require_login=False
        ),
        "forced": authorize(
            client, helpers, metadata, redirect_uri, require_login=True, prompt="login"
        ),
    }
    Path(sys.argv[3]).write_text(json.dumps(tokens))


if __name__ == "__main__":
    main()
