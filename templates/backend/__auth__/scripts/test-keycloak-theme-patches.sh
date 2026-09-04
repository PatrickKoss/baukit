#!/bin/sh
set -eu

: "${KEYCLOAK_ADMIN_USERNAME:?Set KEYCLOAK_ADMIN_USERNAME for the disposable test realm}"
: "${KEYCLOAK_ADMIN_PASSWORD:?Set KEYCLOAK_ADMIN_PASSWORD for the disposable test realm}"
: "${KEYCLOAK_TEST_USERNAME:?Set KEYCLOAK_TEST_USERNAME for the disposable test user}"
: "${KEYCLOAK_TEST_PASSWORD:?Set KEYCLOAK_TEST_PASSWORD for the disposable test user}"

theme_project=
cleanup() {
  if [ -n "$theme_project" ]; then
    docker compose -p "$theme_project" down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

for keycloak_version in 26.7.0 26.7.1; do
  theme_project="{{ context.app_name }}-keycloak-theme-$(printf '%s' "$keycloak_version" | tr . -)"
  keycloak_image="quay.io/keycloak/keycloak:$keycloak_version"
  cleanup
  KEYCLOAK_IMAGE="$keycloak_image" docker compose -p "$theme_project" up -d --wait keycloak

  keycloak_container=$(KEYCLOAK_IMAGE="$keycloak_image" docker compose -p "$theme_project" ps -q keycloak)
  docker inspect "$keycloak_container" | python3 -c '
import json
import sys

mounts = json.load(sys.stdin)[0]["Mounts"]
theme = next((mount for mount in mounts if mount["Destination"] == "/opt/keycloak/themes"), None)
if theme is None or theme["RW"]:
    raise SystemExit("Keycloak theme mount is missing or writable")
'

  KEYCLOAK_BASE_URL="http://127.0.0.1:{{ context.keycloak_host_port }}" \
  KEYCLOAK_REALM="{{ context.app_name }}" \
  KEYCLOAK_CLIENT_ID="{{ context.app_name }}-web" \
  KEYCLOAK_REDIRECT_URI="http://localhost:5173/" \
  node scripts/keycloak-theme.browser.mjs
  printf 'PASS Keycloak %s browser suite\n' "$keycloak_version"
  cleanup
  theme_project=
done
