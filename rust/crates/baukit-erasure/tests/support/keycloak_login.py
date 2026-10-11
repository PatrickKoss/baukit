"""Return a real PKCE token response to the Rust test through a private temporary file."""

import base64
import hashlib
import html
import http.cookiejar
import json
from pathlib import Path
import runpy
import secrets
import sys
import urllib.parse
import urllib.request


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
    authorization_url = (
        metadata["authorization_endpoint"] + "?" + urllib.parse.urlencode(parameters)
    )
    with client.open(authorization_url, timeout=20) as response:
        parser = helpers["LoginFormParser"]()
        parser.feed(response.read().decode())
        login_page_url = response.geturl()
    if not parser.action:
        raise RuntimeError("Keycloak login form was not found")
    fields = {
        **parser.hidden,
        "username": "test",
        "password": "development-password",
        "credentialId": "",
    }
    try:
        client.open(
            urllib.request.Request(
                urllib.parse.urljoin(login_page_url, html.unescape(parser.action)),
                data=urllib.parse.urlencode(fields).encode(),
                headers={"Content-Type": "application/x-www-form-urlencoded"},
            ),
            timeout=20,
        )
    except helpers["CallbackCaptured"] as callback:
        returned = urllib.parse.parse_qs(urllib.parse.urlsplit(callback.location).query)
    else:
        raise RuntimeError("Keycloak did not redirect to the PKCE callback")
    if returned.get("state") != [state] or not returned.get("code"):
        raise RuntimeError("PKCE callback omitted the code or changed the state")
    tokens = helpers["open_json"](
        client,
        urllib.request.Request(
            metadata["token_endpoint"],
            data=urllib.parse.urlencode({
                "grant_type": "authorization_code",
                "client_id": "reconcile-fixture-web",
                "redirect_uri": redirect_uri,
                "code": returned["code"][0],
                "code_verifier": verifier,
            }).encode(),
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        ),
    )
    Path(sys.argv[3]).write_text(json.dumps(tokens))


if __name__ == "__main__":
    main()
