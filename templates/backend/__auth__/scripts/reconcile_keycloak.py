#!/usr/bin/env python3
"""Reconcile selected development realm fields through the Keycloak Admin API."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable, NoReturn
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

import keycloak_policy

RECONCILABLE_REALM_FIELDS = {
    "displayName",
    "enabled",
    "sslRequired",
    "registrationAllowed",
    "loginWithEmailAllowed",
    "passwordPolicy",
    "bruteForceProtected",
    "permanentLockout",
    "failureFactor",
    "waitIncrementSeconds",
    "minimumQuickLoginWaitSeconds",
    "quickLoginCheckMilliSeconds",
    "maxFailureWaitSeconds",
    "maxDeltaTimeSeconds",
}


class ReconcileError(RuntimeError):
    pass


class AuthenticationError(ReconcileError):
    pass


def fail(message: str) -> NoReturn:
    raise ReconcileError(message)


def load_reconcile_config(path: Path) -> dict[str, Any]:
    config = keycloak_policy.load_json(path)
    if config.get("environmentClass") != "development":
        fail("reconciliation config environmentClass must be 'development'")
    realm_fields = config.get("realmFields")
    if not isinstance(realm_fields, list) or not all(
        isinstance(field, str) for field in realm_fields
    ):
        fail("reconciliation config realmFields must be a string array")
    unsupported = sorted(set(realm_fields) - RECONCILABLE_REALM_FIELDS)
    if unsupported:
        fail(f"reconciliation config selects unsupported realm fields: {', '.join(unsupported)}")
    for collection in ("clients", "users"):
        if not isinstance(config.get(collection), list):
            fail(f"reconciliation config {collection} must be an array")
    for client in config["clients"]:
        if not isinstance(client, dict) or not isinstance(client.get("clientId"), str):
            fail("each reconciliation client must have a string clientId")
        for field in ("activeOrigins", "activeRedirectUris"):
            if not isinstance(client.get(field), list) or not all(
                isinstance(value, str) for value in client[field]
            ):
                fail(f"reconciliation client {field} must be a string array")
    if not all(isinstance(username, str) and username for username in config["users"]):
        fail("reconciliation config users must contain non-empty usernames")
    return config


def values_by_key(values: object, key: str, label: str) -> dict[str, dict[str, Any]]:
    if not isinstance(values, list):
        fail(f"realm {label} must be an array")
    indexed: dict[str, dict[str, Any]] = {}
    for value in values:
        if not isinstance(value, dict) or not isinstance(value.get(key), str):
            fail(f"each realm {label} entry must have a string {key}")
        identity = value[key]
        if identity in indexed:
            fail(f"realm has duplicate {label} {identity!r}")
        indexed[identity] = value
    return indexed


def merge_unique(*collections: object) -> list[Any]:
    merged: list[Any] = []
    for collection in collections:
        if not isinstance(collection, list):
            continue
        for value in collection:
            if value not in merged:
                merged.append(value)
    return merged


def parse_client_value(value: str) -> tuple[str, str]:
    client_id, separator, setting = value.partition("=")
    if not separator or not client_id or not setting:
        fail("client URL overrides must use CLIENT_ID=URI")
    return client_id, setting


def apply_client_overrides(
    config: dict[str, Any], origins: list[str], redirects: list[str]
) -> None:
    selected = {client["clientId"]: client for client in config["clients"]}
    for raw_value, field in (
        *((value, "activeOrigins") for value in origins),
        *((value, "activeRedirectUris") for value in redirects),
    ):
        client_id, value = parse_client_value(raw_value)
        if client_id not in selected:
            fail(f"client URL override names unselected client {client_id!r}")
        selected[client_id][field] = merge_unique(selected[client_id][field], [value])


def validate_inputs(
    realm: dict[str, Any],
    policy: dict[str, Any],
    config: dict[str, Any],
) -> None:
    desired_clients = values_by_key(realm.get("clients"), "clientId", "clients")
    candidate = json.loads(json.dumps(realm))
    candidate_clients = values_by_key(candidate.get("clients"), "clientId", "clients")
    for selection in config["clients"]:
        client_id = selection["clientId"]
        desired = desired_clients.get(client_id)
        if desired is None:
            fail(f"selected client {client_id!r} is absent from the realm file")
        if desired.get("publicClient") is not True:
            fail(f"selected client {client_id!r} is not public")
        candidate_client = candidate_clients[client_id]
        candidate_client["webOrigins"] = merge_unique(
            candidate_client.get("webOrigins"), selection["activeOrigins"]
        )
        candidate_client["redirectUris"] = merge_unique(
            candidate_client.get("redirectUris"), selection["activeRedirectUris"]
        )
    desired_users = values_by_key(realm.get("users"), "username", "users")
    for username in config["users"]:
        if username not in desired_users:
            fail(f"selected user {username!r} is absent from the realm file")
    failures = keycloak_policy.validate_realm(candidate, policy, "development")
    if failures:
        fail("realm or active client URL violates policy: " + "; ".join(failures))


class KeycloakApi:
    def __init__(self, base_url: str, token: str | None = None):
        self.base_url = base_url.rstrip("/")
        self.token = token

    def request(
        self,
        method: str,
        path: str,
        payload: object | None = None,
        query: dict[str, str] | None = None,
        form: dict[str, str] | None = None,
    ) -> Any:
        url = f"{self.base_url}{path}"
        if query:
            url = f"{url}?{urlencode(query)}"
        headers: dict[str, str] = {"Accept": "application/json"}
        data = None
        if payload is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(payload, separators=(",", ":")).encode()
        elif form is not None:
            headers["Content-Type"] = "application/x-www-form-urlencoded"
            data = urlencode(form).encode()
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        request = Request(url, data=data, headers=headers, method=method)
        try:
            with urlopen(request, timeout=15) as response:
                body = response.read()
        except HTTPError as error:
            raise ReconcileError(
                f"Keycloak {method} {path} returned HTTP {error.code}"
            ) from None
        except (URLError, TimeoutError) as error:
            reason = getattr(error, "reason", type(error).__name__)
            raise ReconcileError(f"Keycloak {method} {path} failed: {reason}") from None
        if not body:
            return None
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            raise ReconcileError(f"Keycloak {method} {path} returned invalid JSON") from None

    def authenticate(self, username: str, password: str) -> str:
        try:
            response = self.request(
                "POST",
                "/realms/master/protocol/openid-connect/token",
                form={
                    "client_id": "admin-cli",
                    "grant_type": "password",
                    "username": username,
                    "password": password,
                },
            )
        except ReconcileError as error:
            raise AuthenticationError("configured Keycloak administrator is unavailable") from error
        token = response.get("access_token") if isinstance(response, dict) else None
        if not isinstance(token, str) or not token:
            raise AuthenticationError("Keycloak administrator response omitted an access token")
        return token

    def realm(self, realm: str) -> dict[str, Any]:
        return self.request("GET", f"/admin/realms/{quote(realm, safe='')}")

    def update_realm(self, realm: str, value: dict[str, Any]) -> None:
        self.request("PUT", f"/admin/realms/{quote(realm, safe='')}", payload=value)

    def find(self, realm: str, collection: str, key: str, value: str) -> list[dict[str, Any]]:
        response = self.request(
            "GET",
            f"/admin/realms/{quote(realm, safe='')}/{collection}",
            query={key: value, "exact": "true"},
        )
        if not isinstance(response, list):
            fail(f"Keycloak returned invalid {collection} search data")
        return response

    def get(self, realm: str, collection: str, identity: str) -> dict[str, Any]:
        response = self.request(
            "GET",
            f"/admin/realms/{quote(realm, safe='')}/{collection}/{quote(identity, safe='')}",
        )
        if not isinstance(response, dict):
            fail(f"Keycloak returned invalid {collection} data")
        return response

    def create(self, realm: str, collection: str, value: dict[str, Any]) -> None:
        self.request(
            "POST", f"/admin/realms/{quote(realm, safe='')}/{collection}", payload=value
        )

    def update(
        self, realm: str, collection: str, identity: str, value: dict[str, Any]
    ) -> None:
        self.request(
            "PUT",
            f"/admin/realms/{quote(realm, safe='')}/{collection}/{quote(identity, safe='')}",
            payload=value,
        )

    def delete(self, realm: str, collection: str, identity: str) -> None:
        self.request(
            "DELETE",
            f"/admin/realms/{quote(realm, safe='')}/{collection}/{quote(identity, safe='')}",
        )

    def reset_password(self, realm: str, user_id: str, credential: dict[str, Any]) -> None:
        self.request(
            "PUT",
            f"/admin/realms/{quote(realm, safe='')}/users/{quote(user_id, safe='')}/reset-password",
            payload=credential,
        )

    def realm_role(self, realm: str, role_name: str) -> dict[str, Any]:
        response = self.request(
            "GET",
            f"/admin/realms/{quote(realm, safe='')}/roles/{quote(role_name, safe='')}",
        )
        if not isinstance(response, dict):
            fail("Keycloak returned invalid role data")
        return response

    def user_realm_roles(self, realm: str, user_id: str) -> list[dict[str, Any]]:
        response = self.request(
            "GET",
            f"/admin/realms/{quote(realm, safe='')}/users/{quote(user_id, safe='')}/role-mappings/realm",
        )
        if not isinstance(response, list):
            fail("Keycloak returned invalid user role data")
        return response

    def add_user_realm_roles(
        self, realm: str, user_id: str, roles: list[dict[str, Any]]
    ) -> None:
        self.request(
            "POST",
            f"/admin/realms/{quote(realm, safe='')}/users/{quote(user_id, safe='')}/role-mappings/realm",
            payload=roles,
        )


class RealmReconciler:
    def __init__(self, api: KeycloakApi):
        self.api = api

    def reconcile(
        self,
        desired: dict[str, Any],
        config: dict[str, Any],
        reset_passwords: set[str],
    ) -> None:
        realm_name = desired.get("realm")
        if not isinstance(realm_name, str) or not realm_name:
            fail("realm file must contain a non-empty realm name")
        desired_clients = values_by_key(desired.get("clients"), "clientId", "clients")
        desired_users = values_by_key(desired.get("users"), "username", "users")
        self.reconcile_realm(realm_name, desired, config["realmFields"])
        for selection in config["clients"]:
            self.reconcile_client(realm_name, desired_clients[selection["clientId"]], selection)
        for username in config["users"]:
            self.reconcile_user(
                realm_name,
                desired_users[username],
                username in reset_passwords,
            )

    def reconcile_realm(
        self, realm_name: str, desired: dict[str, Any], fields: list[str]
    ) -> None:
        existing = self.api.realm(realm_name)
        merged = dict(existing)
        for field in fields:
            if field not in desired:
                fail(f"selected realm field {field!r} is absent from the realm file")
            merged[field] = desired[field]
        if merged != existing:
            self.api.update_realm(realm_name, merged)

    def reconcile_client(
        self,
        realm_name: str,
        desired: dict[str, Any],
        selection: dict[str, Any],
    ) -> None:
        client_id = desired["clientId"]
        matches = self.api.find(realm_name, "clients", "clientId", client_id)
        if not matches:
            created = dict(desired)
            created.pop("secret", None)
            created["webOrigins"] = merge_unique(
                desired.get("webOrigins"), selection["activeOrigins"]
            )
            created["redirectUris"] = merge_unique(
                desired.get("redirectUris"), selection["activeRedirectUris"]
            )
            self.api.create(realm_name, "clients", created)
            return
        identity = matches[0].get("id")
        if not isinstance(identity, str):
            fail(f"Keycloak client {client_id!r} has no id")
        existing = self.api.get(realm_name, "clients", identity)
        merged = dict(existing)
        for key, value in desired.items():
            if key not in ("id", "secret", "webOrigins", "redirectUris"):
                merged[key] = value
        merged["webOrigins"] = merge_unique(
            existing.get("webOrigins"), desired.get("webOrigins"), selection["activeOrigins"]
        )
        merged["redirectUris"] = merge_unique(
            existing.get("redirectUris"),
            desired.get("redirectUris"),
            selection["activeRedirectUris"],
        )
        if merged != existing:
            self.api.update(realm_name, "clients", identity, merged)

    def reconcile_user(
        self, realm_name: str, desired: dict[str, Any], reset_password: bool
    ) -> None:
        username = desired["username"]
        matches = self.api.find(realm_name, "users", "username", username)
        created = not matches
        if created:
            self.api.create(realm_name, "users", dict(desired))
            matches = self.api.find(realm_name, "users", "username", username)
            if not matches:
                fail(f"created Keycloak user {username!r} could not be found")
        identity = matches[0].get("id")
        if not isinstance(identity, str):
            fail(f"Keycloak user {username!r} has no id")
        if not created:
            existing = self.api.get(realm_name, "users", identity)
            merged = dict(existing)
            for key, value in desired.items():
                if key not in ("id", "credentials", "realmRoles"):
                    merged[key] = value
            if merged != existing:
                self.api.update(realm_name, "users", identity, merged)
        if reset_password:
            credentials = desired.get("credentials")
            if not isinstance(credentials, list) or not credentials:
                fail(f"user {username!r} has no credential to reset")
            credential = credentials[0]
            if not isinstance(credential, dict):
                fail(f"user {username!r} has an invalid credential")
            self.api.reset_password(realm_name, identity, credential)
        desired_roles = desired.get("realmRoles", [])
        if not isinstance(desired_roles, list):
            fail(f"user {username!r} realmRoles must be an array")
        existing_roles = {
            role.get("name") for role in self.api.user_realm_roles(realm_name, identity)
        }
        missing_roles = [
            self.api.realm_role(realm_name, role_name)
            for role_name in desired_roles
            if role_name not in existing_roles
        ]
        if missing_roles:
            self.api.add_user_realm_roles(realm_name, identity, missing_roles)


class ComposeRecovery:
    def __init__(self, compose_file: Path, service: str):
        self.compose_file = compose_file
        self.service = service

    def command(self, arguments: list[str], environment: dict[str, str] | None = None) -> None:
        command = ["docker", "compose", "-f", str(self.compose_file), *arguments]
        result = subprocess.run(
            command,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            fail(f"docker compose command failed with exit code {result.returncode}")

    def start(self, username: str, password: str) -> None:
        self.command(["stop", self.service])
        environment = dict(os.environ)
        environment["BAUKIT_TEMP_ADMIN_USERNAME"] = username
        environment["BAUKIT_TEMP_ADMIN_PASSWORD"] = password
        self.command(
            [
                "run",
                "--rm",
                "--no-deps",
                "-e",
                "BAUKIT_TEMP_ADMIN_USERNAME",
                "-e",
                "BAUKIT_TEMP_ADMIN_PASSWORD",
                self.service,
                "bootstrap-admin",
                "user",
                "--no-prompt",
                "--username:env",
                "BAUKIT_TEMP_ADMIN_USERNAME",
                "--password:env",
                "BAUKIT_TEMP_ADMIN_PASSWORD",
            ],
            environment,
        )
        self.command(["up", "-d", "--wait", "--wait-timeout", "120", self.service])


def first_password(user: dict[str, Any]) -> dict[str, Any] | None:
    credentials = user.get("credentials")
    if not isinstance(credentials, list):
        return None
    for credential in credentials:
        if isinstance(credential, dict) and credential.get("type") == "password":
            return credential
    return None


def repair_master_admin(api: KeycloakApi, username: str, password: str) -> None:
    matches = api.find("master", "users", "username", username)
    credential = {"type": "password", "value": password, "temporary": False}
    if matches:
        identity = matches[0].get("id")
        if not isinstance(identity, str):
            fail("configured master administrator has no id")
        api.reset_password("master", identity, credential)
    else:
        api.create(
            "master",
            "users",
            {"username": username, "enabled": True, "credentials": [credential]},
        )
        matches = api.find("master", "users", "username", username)
        identity = matches[0].get("id") if matches else None
        if not isinstance(identity, str):
            fail("configured master administrator could not be recreated")
    roles = {role.get("name") for role in api.user_realm_roles("master", identity)}
    if "admin" not in roles:
        api.add_user_realm_roles("master", identity, [api.realm_role("master", "admin")])


def delete_master_user(api: KeycloakApi, username: str) -> None:
    matches = api.find("master", "users", "username", username)
    if len(matches) != 1 or not isinstance(matches[0].get("id"), str):
        fail("temporary recovery administrator could not be identified for cleanup")
    api.delete("master", "users", matches[0]["id"])


def run_with_recovery(
    authenticate: Callable[[str, str], str],
    administrator: tuple[str, str],
    action: Callable[[str], None],
    start_recovery: Callable[[str, str], None],
    repair: Callable[[str, str, str], None],
    cleanup: Callable[[str, str], None],
) -> bool:
    username, password = administrator
    try:
        token = authenticate(username, password)
    except AuthenticationError:
        temporary_username = f"baukit-recovery-{secrets.token_hex(10)}"
        temporary_password = secrets.token_urlsafe(32)
        start_recovery(temporary_username, temporary_password)
        token = authenticate(temporary_username, temporary_password)
        pending: BaseException | None = None
        try:
            action(token)
        except BaseException as error:
            pending = error
        try:
            repair(token, username, password)
        except BaseException as error:
            if pending is None:
                pending = error
        try:
            cleanup(token, temporary_username)
        except BaseException as error:
            cleanup_error = ReconcileError(
                "temporary recovery administrator cleanup failed"
            )
            if pending is not None:
                raise cleanup_error from pending
            raise cleanup_error from error
        if pending is not None:
            raise pending
        return True
    action(token)
    return False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--realm", type=Path, default=Path("keycloak/realm.json"))
    parser.add_argument("--policy", type=Path, default=Path("keycloak/realm-policy.json"))
    parser.add_argument("--config", type=Path, default=Path("keycloak/reconcile.json"))
    parser.add_argument(
        "--keycloak-url", default="http://localhost:{{ context.keycloak_host_port }}"
    )
    parser.add_argument("--compose-file", type=Path, default=Path("compose.yaml"))
    parser.add_argument("--compose-service", default="keycloak")
    parser.add_argument("--admin-username", default="admin")
    parser.add_argument("--admin-password-env", default="KC_BOOTSTRAP_ADMIN_PASSWORD")
    parser.add_argument("--client-origin", action="append", default=[])
    parser.add_argument("--client-redirect", action="append", default=[])
    parser.add_argument("--reset-password", action="append", default=[])
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        desired = keycloak_policy.load_json(arguments.realm)
        policy = keycloak_policy.load_json(arguments.policy)
        config = load_reconcile_config(arguments.config)
        apply_client_overrides(config, arguments.client_origin, arguments.client_redirect)
        validate_inputs(desired, policy, config)
        if arguments.check:
            print("Keycloak reconciliation inputs are valid.")
            return 0
        unknown_resets = sorted(set(arguments.reset_password) - set(config["users"]))
        if unknown_resets:
            fail(f"password reset names unselected users: {', '.join(unknown_resets)}")
        admin_password = os.environ.get(arguments.admin_password_env, "admin")
        unauthenticated_api = KeycloakApi(arguments.keycloak_url)
        recovery = ComposeRecovery(arguments.compose_file, arguments.compose_service)

        def authenticate(username: str, password: str) -> str:
            return unauthenticated_api.authenticate(username, password)

        def reconcile(token: str) -> None:
            RealmReconciler(KeycloakApi(arguments.keycloak_url, token)).reconcile(
                desired, config, set(arguments.reset_password)
            )

        recovered = run_with_recovery(
            authenticate,
            (arguments.admin_username, admin_password),
            reconcile,
            recovery.start,
            lambda token, username, password: repair_master_admin(
                KeycloakApi(arguments.keycloak_url, token), username, password
            ),
            lambda token, username: delete_master_user(
                KeycloakApi(arguments.keycloak_url, token), username
            ),
        )
    except (keycloak_policy.PolicyError, ReconcileError) as error:
        print(f"Keycloak reconciliation failed: {error}", file=sys.stderr)
        return 1
    print(
        "Keycloak development realm reconciled"
        + (" after administrator recovery." if recovered else ".")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
