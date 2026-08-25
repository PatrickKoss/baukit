#!/usr/bin/env python3
"""Perform a headless OIDC authorization-code + PKCE login.

The login-form submission is intentionally limited to Keycloak-compatible HTML
forms. Authorization and token endpoints are resolved through OIDC discovery.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import html
import http.cookiejar
import json
import os
import secrets
import socket
import urllib.error
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path


class LoginFormParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.action: str | None = None
        self.hidden: dict[str, str] = {}
        self.in_login_form = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if tag == "form" and (values.get("id") == "kc-form-login" or self.action is None):
            self.action = values.get("action")
            self.in_login_form = True
        elif tag == "input" and self.in_login_form and values.get("type") == "hidden":
            name = values.get("name")
            if name:
                self.hidden[name] = values.get("value") or ""

    def handle_endtag(self, tag: str) -> None:
        if tag == "form" and self.in_login_form:
            self.in_login_form = False


class CallbackCaptured(Exception):
    def __init__(self, location: str) -> None:
        super().__init__(location)
        self.location = location


class CallbackRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, redirect_uri: str) -> None:
        super().__init__()
        self.redirect = urllib.parse.urlsplit(redirect_uri)

    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        target = urllib.parse.urlsplit(new_url)
        if (
            target.scheme,
            target.netloc,
            target.path,
        ) == (self.redirect.scheme, self.redirect.netloc, self.redirect.path):
            raise CallbackCaptured(new_url)
        return super().redirect_request(request, file_pointer, code, message, headers, new_url)


class LocalSmokeCookiePolicy(http.cookiejar.DefaultCookiePolicy):
    """Match browsers' localhost secure-context behavior for disposable HTTP OIDC."""

    def return_ok_secure(self, cookie, request):
        return True


def install_host_resolution(entries: list[str]) -> None:
    aliases: dict[str, str] = {}
    for entry in entries:
        hostname, separator, address = entry.partition("=")
        if not separator or not hostname or not address:
            raise ValueError(f"invalid --resolve-host {entry!r}; expected HOST=ADDRESS")
        aliases[hostname] = address
    if not aliases:
        return

    original = socket.getaddrinfo

    def getaddrinfo(host, port, *args, **kwargs):
        return original(aliases.get(host, host), port, *args, **kwargs)

    socket.getaddrinfo = getaddrinfo  # type: ignore[assignment]


def open_json(client: urllib.request.OpenerDirector, request: urllib.request.Request) -> dict:
    with client.open(request, timeout=20) as response:
        return json.load(response)


def discover(client: urllib.request.OpenerDirector, issuer: str) -> dict:
    metadata = open_json(
        client,
        urllib.request.Request(
            issuer.rstrip("/") + "/.well-known/openid-configuration",
            headers={"Accept": "application/json"},
        ),
    )
    for key in ("authorization_endpoint", "token_endpoint"):
        if not isinstance(metadata.get(key), str):
            raise RuntimeError(f"OIDC discovery document has no {key}")
    return metadata


def pkce_login(
    client: urllib.request.OpenerDirector,
    metadata: dict,
    *,
    username: str,
    password: str,
    client_id: str,
    redirect_uri: str,
    scope: str,
) -> str:
    verifier = base64.urlsafe_b64encode(secrets.token_bytes(64)).decode().rstrip("=")
    challenge = (
        base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest())
        .decode()
        .rstrip("=")
    )
    state = secrets.token_urlsafe(24)
    authorization_url = metadata["authorization_endpoint"] + "?" + urllib.parse.urlencode(
        {
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "scope": scope,
            "state": state,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }
    )
    with client.open(authorization_url, timeout=20) as response:
        login_page = response.read().decode()
        login_page_url = response.geturl()
    form = LoginFormParser()
    form.feed(login_page)
    if not form.action:
        raise RuntimeError("Keycloak-compatible login form was not found")
    fields = dict(form.hidden)
    fields.update({"username": username, "password": password, "credentialId": ""})
    try:
        client.open(
            urllib.request.Request(
                urllib.parse.urljoin(login_page_url, html.unescape(form.action)),
                data=urllib.parse.urlencode(fields).encode(),
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                method="POST",
            ),
            timeout=20,
        )
    except CallbackCaptured as callback:
        callback_url = callback.location
    else:
        raise RuntimeError("identity provider did not redirect to the PKCE callback")

    parameters = urllib.parse.parse_qs(urllib.parse.urlsplit(callback_url).query)
    if parameters.get("state") != [state]:
        raise RuntimeError("OIDC state did not match")
    code = parameters.get("code", [None])[0]
    if not code:
        raise RuntimeError("OIDC callback did not contain an authorization code")
    tokens = open_json(
        client,
        urllib.request.Request(
            metadata["token_endpoint"],
            data=urllib.parse.urlencode(
                {
                    "grant_type": "authorization_code",
                    "client_id": client_id,
                    "redirect_uri": redirect_uri,
                    "code": code,
                    "code_verifier": verifier,
                }
            ).encode(),
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            method="POST",
        ),
    )
    access_token = tokens.get("access_token")
    if not isinstance(access_token, str) or not access_token:
        raise RuntimeError("token response did not contain an access token")
    return access_token


def write_token(path: Path, token: str) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(token + "\n")


def check_authenticated_url(url: str, token: str, expected_status: int) -> None:
    request = urllib.request.Request(
        url,
        headers={"Accept": "application/json", "Authorization": f"Bearer {token}"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            status = response.status
            response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        error.read()
    if status != expected_status:
        raise RuntimeError(f"authenticated check returned HTTP {status}; expected {expected_status}")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--issuer", required=True)
    parser.add_argument("--client-id", required=True)
    parser.add_argument("--username", required=True)
    password = parser.add_mutually_exclusive_group(required=True)
    password.add_argument("--password")
    password.add_argument("--password-env", metavar="VARIABLE")
    parser.add_argument("--redirect-uri", default="http://localhost:5173/callback")
    parser.add_argument("--scope", default="openid profile email")
    parser.add_argument("--resolve-host", action="append", default=[], metavar="HOST=ADDRESS")
    parser.add_argument("--token-file", type=Path)
    parser.add_argument("--check-url")
    parser.add_argument("--check-status", type=int, default=200)
    return parser.parse_args()


def main() -> None:
    args = arguments()
    install_host_resolution(args.resolve_host)
    password = args.password
    if args.password_env:
        password = os.environ.get(args.password_env)
        if password is None:
            raise RuntimeError(f"password environment variable is not set: {args.password_env}")
    client = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(
            http.cookiejar.CookieJar(policy=LocalSmokeCookiePolicy())
        ),
        CallbackRedirectHandler(args.redirect_uri),
    )
    metadata = discover(client, args.issuer)
    token = pkce_login(
        client,
        metadata,
        username=args.username,
        password=password,
        client_id=args.client_id,
        redirect_uri=args.redirect_uri,
        scope=args.scope,
    )
    if args.token_file:
        write_token(args.token_file, token)
    if args.check_url:
        check_authenticated_url(args.check_url, token, args.check_status)
    print("headless authorization-code + PKCE login passed")
    if args.check_url:
        print(f"authenticated check passed with HTTP {args.check_status}")


if __name__ == "__main__":
    main()
