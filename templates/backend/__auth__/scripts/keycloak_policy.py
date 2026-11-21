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

ENVIRONMENT_CLASSES = ("development", "lab")
TLS_LEVELS = {"none": 0, "external": 1, "all": 2}
ADMIN_API_SCOPE = "kc-admin-api"
POLICY_EXPRESSION = re.compile(r"^([A-Za-z][A-Za-z0-9]*)(?:\(([^)]*)\))?$")
LOOPBACK_HOSTS = ("localhost", "127.0.0.1", "::1")
KNOWN_PASSWORDS = frozenset(
    {"admin", "changeme", "development-password", "keycloak", "password", "secret", "test"}
)
SUBSTITUTION = re.compile(r"^\$\((env|file):[^)]+\)$")
AUDIENCE_MAPPER = "oidc-audience-mapper"
MCP_PATH = "/mcp"


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
    if environment_class == "lab" and not failures:
        failures.extend(lab_policy_failures(policy))
    return failures


def is_lab_https_url(value: object) -> bool:
    if not isinstance(value, str):
        return False
    parsed = urlsplit(value)
    return parsed.scheme == "https" and bool(parsed.hostname) and not is_loopback(parsed)


def lab_policy_failures(policy: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    if policy["redirectUris"]["allowDevelopmentLoopbackHttp"] is not False:
        failures.append("lab policy redirectUris.allowDevelopmentLoopbackHttp must be false")
    if TLS_LEVELS[policy["minimumTls"]] < TLS_LEVELS["external"]:
        failures.append("lab policy minimumTls must be external or all")
    resource = policy.get("mcpResourceUrl")
    if resource is not None and not (
        is_lab_https_url(resource) and urlsplit(resource).path == MCP_PATH
    ):
        failures.append("lab policy mcpResourceUrl must be a non-loopback https URL ending in /mcp")
    return failures


def is_loopback(parsed: SplitResult) -> bool:
    return parsed.hostname in LOOPBACK_HOSTS


def http_redirect_problem(
    parsed: SplitResult, candidate: str, allow_loopback_http: bool
) -> str | None:
    if not parsed.hostname:
        return "must include a host"
    if "*" in parsed.netloc:
        return "cannot wildcard the host or port"
    if "*" in candidate:
        return "has an unbounded wildcard"
    if parsed.scheme == "http" and not (allow_loopback_http and is_loopback(parsed)):
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
        problem = http_redirect_problem(
            parsed, candidate, redirect_policy["allowDevelopmentLoopbackHttp"]
        )
        if problem is None and policy["environmentClass"] == "lab" and is_loopback(parsed):
            return "points at a loopback host, which a lab realm cannot use"
        return problem
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


def admin_api_client_ids(users: list[Any]) -> list[str]:
    client_ids: list[str] = []
    for user in users:
        if not isinstance(user, dict):
            continue
        client_id = user.get("serviceAccountClientId")
        mappings = user.get("clientRoles")
        roles = mappings.get("realm-management") if isinstance(mappings, dict) else None
        if isinstance(client_id, str) and isinstance(roles, list) and roles:
            client_ids.append(client_id)
    return client_ids


def admin_api_scope_failures(
    realm: dict[str, Any], clients: list[Any], users: list[Any]
) -> list[str]:
    client_ids = admin_api_client_ids(users)
    if not client_ids:
        return []
    failures: list[str] = []
    scopes = realm.get("clientScopes")
    if isinstance(scopes, list) and not any(
        isinstance(scope, dict) and scope.get("name") == ADMIN_API_SCOPE for scope in scopes
    ):
        failures.append(
            f"realm declares clientScopes without {ADMIN_API_SCOPE!r}, so Keycloak drops it from "
            "imported clients"
        )
    for client_id in client_ids:
        client = next(
            (
                value
                for value in clients
                if isinstance(value, dict) and value.get("clientId") == client_id
            ),
            {},
        )
        default_scopes = client.get("defaultClientScopes")
        if not isinstance(default_scopes, list) or ADMIN_API_SCOPE not in default_scopes:
            failures.append(
                f"client {client_id!r} calls the admin API but lacks the {ADMIN_API_SCOPE} scope "
                "Keycloak 26.8.2 requires"
            )
    return failures


def credential_problem(username: object, value: object) -> str | None:
    if not isinstance(value, str) or not value:
        return "has no password value"
    if SUBSTITUTION.fullmatch(value):
        return None
    if value.lower() in KNOWN_PASSWORDS or value == username:
        return "uses a default or known password"
    return "stores a literal password; use $(env:NAME) or $(file:PATH) substitution"


def lab_credential_failures(users: list[Any]) -> list[str]:
    failures: list[str] = []
    for index, user in enumerate(users):
        if not isinstance(user, dict):
            continue
        username = user.get("username")
        label = username if isinstance(username, str) else f"users[{index}]"
        credentials = user.get("credentials", [])
        for credential in credentials if isinstance(credentials, list) else []:
            if not isinstance(credential, dict) or credential.get("type") != "password":
                continue
            problem = credential_problem(username, credential.get("value"))
            if problem:
                failures.append(f"user {label!r} {problem}")
    return failures


def mcp_audiences(clients: list[Any]) -> list[tuple[str, object]]:
    found: list[tuple[str, object]] = []
    for client in clients:
        if not isinstance(client, dict):
            continue
        mappers = client.get("protocolMappers", [])
        for mapper in mappers if isinstance(mappers, list) else []:
            if not isinstance(mapper, dict) or mapper.get("protocolMapper") != AUDIENCE_MAPPER:
                continue
            config = mapper.get("config")
            audience = config.get("included.custom.audience") if isinstance(config, dict) else None
            if isinstance(audience, str) and urlsplit(audience).path == MCP_PATH:
                found.append((str(client.get("clientId")), audience))
    return found


def lab_mcp_failures(clients: list[Any], policy: dict[str, Any]) -> list[str]:
    expected = policy.get("mcpResourceUrl")
    audiences = mcp_audiences(clients)
    if expected is None:
        return [
            f"client {client_id!r} maps MCP audience {audience!r}, but the lab policy sets no "
            "mcpResourceUrl"
            for client_id, audience in audiences
        ]
    failures = [
        f"client {client_id!r} MCP audience {audience!r} must be {expected!r}"
        for client_id, audience in audiences
        if audience != expected
    ]
    if not any(audience == expected for _, audience in audiences):
        failures.append(f"no client maps the MCP audience {expected!r}")
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
    users = users if isinstance(users, list) else []
    failures.extend(service_account_failures(clients, users, policy))
    failures.extend(admin_api_scope_failures(realm, clients, users))
    if environment_class == "lab":
        failures.extend(lab_credential_failures(users))
        failures.extend(lab_mcp_failures(clients, policy))
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
