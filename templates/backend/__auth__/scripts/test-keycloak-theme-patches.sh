#!/bin/sh
set -eu

: "${KEYCLOAK_ADMIN_USERNAME:?Set KEYCLOAK_ADMIN_USERNAME for the disposable test realm}"
: "${KEYCLOAK_ADMIN_PASSWORD:?Set KEYCLOAK_ADMIN_PASSWORD for the disposable test realm}"
: "${KEYCLOAK_TEST_USERNAME:?Set KEYCLOAK_TEST_USERNAME for the disposable test user}"
: "${KEYCLOAK_TEST_PASSWORD:?Set KEYCLOAK_TEST_PASSWORD for the disposable test user}"

theme_project=
theme_compose=$(mktemp)
cat > "$theme_compose" <<'YAML'
services:
  keycloak:
    ports: !override
      - "127.0.0.1::8080"
YAML

compose() {
  KEYCLOAK_IMAGE="$keycloak_image" docker compose \
    -f compose.yaml -f "$theme_compose" -p "$theme_project" "$@"
}

cleanup() {
  if [ -n "$theme_project" ]; then
    compose down --volumes --remove-orphans >/dev/null
    theme_project=
  fi
}
trap 'cleanup; rm -f "$theme_compose"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for keycloak_version in 26.7.5 26.8.0; do
  theme_project="{{ context.app_name }}-keycloak-theme-$$-$(printf '%s' "$keycloak_version" | tr . -)"
  keycloak_image="quay.io/keycloak/keycloak:$keycloak_version"
  compose up -d --wait keycloak

  keycloak_container=$(compose ps -q keycloak)
  docker inspect "$keycloak_container" | python3 -c '
import json
import sys

mounts = json.load(sys.stdin)[0]["Mounts"]
theme = next((mount for mount in mounts if mount["Destination"] == "/opt/keycloak/themes"), None)
if theme is None or theme["RW"]:
    raise SystemExit("Keycloak theme mount is missing or writable")
'

  keycloak_binding=$(compose port keycloak 8080)
  KEYCLOAK_BASE_URL="http://$keycloak_binding" \
  KEYCLOAK_REALM="{{ context.app_name }}" \
  KEYCLOAK_CLIENT_ID="{{ context.app_name }}-web" \
  KEYCLOAK_REDIRECT_URI="http://localhost:5173/" \
  node scripts/keycloak-theme.browser.mjs
  printf 'PASS Keycloak %s browser suite\n' "$keycloak_version"
  cleanup
done
