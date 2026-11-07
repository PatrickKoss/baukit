#!/usr/bin/env python3
"""Validate a Keycloak realm against an explicit product policy."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any
from urllib.parse import SplitResult, urlsplit

ENVIRONMENT_CLASSES = ("development",)
TLS_LEVELS = {"none": 0, "external": 1, "all": 2}
POLICY_EXPRESSION = re.compile(r"^([A-Za-z][A-Za-z0-9]*)(?:\(([^)]*)\))?$")


class PolicyError(ValueError):
    pass


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PolicyError(f"could not read {path}: {error}") from error
    if not isinstance(value, dict):
        raise PolicyError(f"{path} must contain a JSON object")
    return value


def parse_password_policy(value: object) -> dict[str, str | None]:
    if not isinstance(value, str):
        return {}
    parsed: dict[str, str | None] = {}
    for expression in re.split(r"\s+and\s+", value):
        match = POLICY_EXPRESSION.fullmatch(expression.strip())
        if match:
            parsed[match.group(1)] = match.group(2)
    return parsed


def policy_integer(policies: dict[str, str | None], name: str) -> int | None:
    value = policies.get(name)
    if value is None or not value.isdecimal():
        return None
    return int(value)


def is_positive_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 1


def password_policy_failures(password: object) -> list[str]:
    if not isinstance(password, dict):
        return ["policy password must be an object"]
    failures: list[str] = []
    minimum = password.get("minimumLength")
    maximum = password.get("maximumLength")
    if not is_positive_integer(minimum):
        failures.append("policy password.minimumLength must be a positive integer")
    if not is_positive_integer(maximum):
        failures.append("policy password.maximumLength must be a positive integer")
    if isinstance(minimum, int) and isinstance(maximum, int) and minimum > maximum:
        failures.append("policy password minimumLength cannot exceed maximumLength")
    failures.extend(
        f"policy password.{name} must be true or false"
        for name in ("excludeUsername", "excludeEmail")
        if not isinstance(password.get(name), bool)
    )
    return failures


def redirect_policy_failures(redirects: object) -> list[str]:
    if not isinstance(redirects, dict):
        return ["policy redirectUris must be an object"]
    failures: list[str] = []
    if not is_positive_integer(redirects.get("maximumPerClient")):
        failures.append("policy redirectUris.maximumPerClient must be a positive integer")
    if not isinstance(redirects.get("allowDevelopmentLoopbackHttp"), bool):
        failures.append("policy redirectUris.allowDevelopmentLoopbackHttp must be true or false")
    schemes = redirects.get("allowedCustomSchemes")
    if not isinstance(schemes, list) or not all(
        isinstance(scheme, str) and scheme for scheme in schemes
    ):
        failures.append("policy redirectUris.allowedCustomSchemes must be a string array")
    return failures


def service_account_roles_valid(roles: object) -> bool:
    return isinstance(roles, dict) and all(
        isinstance(client, str)
        and client
        and isinstance(names, list)
        and names
        and all(isinstance(name, str) and name for name in names)
        for client, names in roles.items()
    )


def validate_policy_document(policy: dict[str, Any], environment_class: str) -> list[str]:
    failures: list[str] = []
    declared_class = policy.get("environmentClass")
    if declared_class != environment_class:
        failures.append(
            f"policy environmentClass must be {environment_class!r}, got {declared_class!r}"
        )
    failures.extend(password_policy_failures(policy.get("password")))
    if not isinstance(policy.get("requireBruteForceProtection"), bool):
        failures.append("policy requireBruteForceProtection must be true or false")
    if policy.get("minimumTls") not in TLS_LEVELS:
        failures.append("policy minimumTls must be one of none, external, or all")
    failures.extend(redirect_policy_failures(policy.get("redirectUris")))
    if not service_account_roles_valid(policy.get("serviceAccountRoles", {})):
        failures.append("policy serviceAccountRoles must map client ids to non-empty role arrays")
    return failures


def http_redirect_problem(
    parsed: SplitResult, candidate: str, allow_loopback_http: bool
) -> str | None:
    if not parsed.hostname:
        return "must include a host"
    if "*" in parsed.netloc:
        return "cannot wildcard the host or port"
    if "*" in candidate:
        return "has an unbounded wildcard"
    loopback = parsed.hostname in ("localhost", "127.0.0.1", "::1")
    if parsed.scheme == "http" and not (allow_loopback_http and loopback):
        return "uses HTTP outside an allowed development loopback address"
    return None


def validate_redirect_uri(uri: object, policy: dict[str, Any]) -> str | None:
    if not isinstance(uri, str) or not uri or uri == "*":
        return "must be a non-empty string" if uri != "*" else "has an unbounded wildcard"
    candidate = uri[:-1] if uri.endswith("/*") else uri
    parsed = urlsplit(candidate)
    if not parsed.scheme or parsed.username or parsed.password or parsed.fragment:
        return "must be an absolute URI without user information or a fragment"
    redirect_policy = policy["redirectUris"]
    if parsed.scheme in ("http", "https"):
        return http_redirect_problem(
            parsed, candidate, redirect_policy["allowDevelopmentLoopbackHttp"]
        )
    if parsed.scheme not in redirect_policy["allowedCustomSchemes"]:
        return f"uses custom scheme {parsed.scheme!r}, which the policy does not allow"
    return "has an unbounded wildcard" if "*" in candidate else None


def realm_password_failures(realm: dict[str, Any], policy: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    password = policy["password"]
    parsed_password = parse_password_policy(realm.get("passwordPolicy"))
    minimum = policy_integer(parsed_password, "length")
    maximum = policy_integer(parsed_password, "maxLength")
    if minimum is None or minimum < password["minimumLength"]:
        failures.append(f"passwordPolicy length must be at least {password['minimumLength']}")
    if maximum is None or maximum > password["maximumLength"]:
        failures.append(f"passwordPolicy maxLength must be at most {password['maximumLength']}")
    if password["excludeUsername"] and "notUsername" not in parsed_password:
        failures.append("passwordPolicy must include notUsername")
    if password["excludeEmail"] and "notEmail" not in parsed_password:
        failures.append("passwordPolicy must include notEmail")
    if policy["requireBruteForceProtection"] and realm.get("bruteForceProtected") is not True:
        failures.append("bruteForceProtected must be true")
    realm_tls = realm.get("sslRequired")
    if realm_tls not in TLS_LEVELS:
        failures.append("sslRequired must be one of none, external, or all")
    elif TLS_LEVELS[realm_tls] < TLS_LEVELS[policy["minimumTls"]]:
        failures.append(f"sslRequired must be at least {policy['minimumTls']}")
    return failures


def public_client_failures(client: dict[str, Any], label: str, policy: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    attributes = client.get("attributes")
    pkce = attributes.get("pkce.code.challenge.method") if isinstance(attributes, dict) else None
    if pkce != "S256":
        failures.append(f"public client {label!r} must require PKCE S256")
    redirect_uris = client.get("redirectUris", [])
    if not isinstance(redirect_uris, list):
        failures.append(f"public client {label!r} redirectUris must be an array")
        return failures
    maximum_count = policy["redirectUris"]["maximumPerClient"]
    if len(redirect_uris) > maximum_count:
        failures.append(f"public client {label!r} has more than {maximum_count} redirect URIs")
    for uri in redirect_uris:
        problem = validate_redirect_uri(uri, policy)
        if problem:
            failures.append(f"public client {label!r} redirect URI {uri!r} {problem}")
    return failures


def client_failures(clients: list[Any], policy: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    for index, client in enumerate(clients):
        if not isinstance(client, dict):
            failures.append(f"clients[{index}] must be an object")
            continue
        client_id = client.get("clientId")
        label = client_id if isinstance(client_id, str) else f"clients[{index}]"
        if client.get("directAccessGrantsEnabled") is not False:
            failures.append(f"client {label!r} must disable direct-access grants")
        if client.get("publicClient") is True:
            failures.extend(public_client_failures(client, label, policy))
    return failures


def service_account_failures(
    clients: list[Any], users: list[Any], policy: dict[str, Any]
) -> list[str]:
    failures: list[str] = []
    for client_id, required_roles in policy.get("serviceAccountRoles", {}).items():
        client = next(
            (
                value
                for value in clients
                if isinstance(value, dict) and value.get("clientId") == client_id
            ),
            {},
        )
        if (
            client.get("serviceAccountsEnabled") is not True
            or client.get("publicClient") is not False
        ):
            failures.append(f"client {client_id!r} must enable a confidential service account")
        account = next(
            (
                user
                for user in users
                if isinstance(user, dict) and user.get("serviceAccountClientId") == client_id
            ),
            {},
        )
        mappings = account.get("clientRoles")
        roles = mappings.get("realm-management", []) if isinstance(mappings, dict) else []
        if not isinstance(roles, list) or not all(role in roles for role in required_roles):
            failures.append(
                f"client {client_id!r} service account is missing required realm-management roles"
            )
    return failures


def validate_realm(
    realm: dict[str, Any], policy: dict[str, Any], environment_class: str
) -> list[str]:
    failures = validate_policy_document(policy, environment_class)
    if failures:
        return failures
    failures = realm_password_failures(realm, policy)
    clients = realm.get("clients")
    if not isinstance(clients, list):
        failures.append("clients must be an array")
        return failures
    failures.extend(client_failures(clients, policy))
    users = realm.get("users", [])
    failures.extend(
        service_account_failures(clients, users if isinstance(users, list) else [], policy)
    )
    return failures


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--realm", type=Path, default=Path("keycloak/realm.json"))
    parser.add_argument("--policy", type=Path, default=Path("keycloak/realm-policy.json"))
    parser.add_argument("--environment-class", choices=ENVIRONMENT_CLASSES, required=True)
    arguments = parser.parse_args(argv)
    try:
        realm = load_json(arguments.realm)
        policy = load_json(arguments.policy)
        failures = validate_realm(realm, policy, arguments.environment_class)
    except PolicyError as error:
        print(f"Keycloak policy check failed: {error}", file=sys.stderr)
        return 1
    if failures:
        print("Keycloak policy check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(
        f"Keycloak policy check passed for {arguments.environment_class} realm "
        f"{realm.get('realm', '<unnamed>')}."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
