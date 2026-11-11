#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="${BAUKIT_QA_STATE_DIR:-$root/mobile/.qa}"
action="${1:-start}"
{% if context.backend %}compose_project="${BAUKIT_QA_COMPOSE_PROJECT:-{{ context.app_name }}-mobile-qa}"
postgres_port="${BAUKIT_QA_POSTGRES_PORT:-15432}"
redis_port="${BAUKIT_QA_REDIS_PORT:-16379}"
api_port="${BAUKIT_QA_API_PORT:-18080}"
ops_port="${BAUKIT_QA_OPS_PORT:-19090}"
{% if context.auth_enabled %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}
compose=(
  docker compose
  --project-name "$compose_project"
  --project-directory "$root"
  -f "$root/compose.yaml"
  -f "$root/mobile/scripts/qa/docker-compose.qa.yml"
)
if [[ -n "${BAUKIT_QA_COMPOSE_OVERRIDE:-}" ]]; then
  compose+=(-f "$BAUKIT_QA_COMPOSE_OVERRIDE")
fi
qa_compose_services=()
qa_cargo_bins=()
qa_services_env() { :; }
qa_before_api() { :; }
qa_after_api() { :; }
qa_stop_services() { :; }
{% endif %}qa_services_start() { start_default_services; }
qa_services_stop() { stop_default_services; }
local_hooks="$root/mobile/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"
{% if context.backend %}export BAUKIT_QA_POSTGRES_PORT="$postgres_port" BAUKIT_QA_REDIS_PORT="$redis_port"
export BAUKIT_QA_API_PORT="$api_port" BAUKIT_QA_OPS_PORT="$ops_port"
{% if context.auth_enabled %}export BAUKIT_QA_KEYCLOAK_PORT="$keycloak_port"
{% endif %}{% endif %}
api_running() {
  kill -0 "$1" 2>/dev/null && [[ "$(ps -o stat= -p "$1")" != Z* ]]
}

stop_pid_file() {
  local pid_file="$1" name="$2" pid
  [[ -f "$pid_file" ]] || return 0
  pid="$(<"$pid_file")"
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || {
    echo "qa: invalid $name PID; ownership state was kept" >&2
    return 1
  }
  if api_running "$pid"; then
    kill "$pid" || return
    for _ in $(seq 1 30); do
      api_running "$pid" || break
      sleep 1
    done
    if api_running "$pid"; then
      echo "qa: $name $pid did not stop; ownership state was kept" >&2
      return 1
    fi
  fi
  rm -f "$pid_file"
}

stop_api() {
  stop_pid_file "$state_dir/backend.pid" backend
}

stop_default_services() {
  local status=0
  stop_api || status=$?
{% if context.backend %}  qa_stop_services || status=$?
  "${compose[@]}" down --volumes || status=$?
{% endif %}  if [[ "$status" != 0 ]]; then
    echo "qa: service cleanup failed; ownership state was kept" >&2
    return "$status"
  fi
}

start_default_services() {
{% if context.backend %}  command -v docker >/dev/null || { echo "qa: docker is required" >&2; return 1; }
  command -v cargo >/dev/null || { echo "qa: cargo is required" >&2; return 1; }
  command -v curl >/dev/null || { echo "qa: curl is required" >&2; return 1; }

  echo "qa: starting disposable PostgreSQL, Redis{% if context.auth_enabled %}, and Keycloak{% endif %}"
  "${compose[@]}" up -d --wait "${qa_compose_services[@]}"

  export {{ context.app_env }}__DATABASE__URL="postgres://postgres:postgres@127.0.0.1:${postgres_port}/{{ context.app_crate }}"
  export {{ context.app_env }}__RATE_LIMIT__REDIS_URL="redis://127.0.0.1:${redis_port}/"
  export {{ context.app_env }}__HTTP__BIND_ADDRESS=127.0.0.1
  export {{ context.app_env }}__HTTP__PORT="$api_port"
  export {{ context.app_env }}__OPS__BIND_ADDRESS=127.0.0.1
  export {{ context.app_env }}__OPS__PORT="$ops_port"
{% if context.auth_oidc %}export {{ context.app_env }}__AUTH__ISSUER="http://localhost:${keycloak_port}/realms/{{ context.app_name }}"
  export {{ context.app_env }}__AUTH__AUDIENCE="{{ context.app_name }}-backend"
{% elif context.auth_enabled %}export {{ context.app_env }}__AUTH__ISSUER="http://localhost:${keycloak_port}/realms/{{ context.app_name }}"
  export {{ context.app_env }}__AUTH__JWKS_URI="http://localhost:${keycloak_port}/realms/{{ context.app_name }}/protocol/openid-connect/certs"
  export {{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET="${ {{- context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET:-qa-placeholder-not-a-provider-key}"
{% endif %}
  qa_services_env

  cargo_bins=(--bin migrate --bin api)
  for bin in "${qa_cargo_bins[@]}"; do
    cargo_bins+=(--bin "$bin")
  done
  cargo build --locked --manifest-path "$root/backend/Cargo.toml" \
    -p {{ context.app_name }}-bin "${cargo_bins[@]}"
  "$root/backend/target/debug/migrate"
  qa_before_api

  nohup "$root/backend/target/debug/api" \
    >"$state_dir/backend.log" 2>&1 &
  backend_pid=$!
  printf '%s\n' "$backend_pid" > "$state_dir/backend.pid"

  for _ in $(seq 1 120); do
    if curl --fail --silent "http://127.0.0.1:${ops_port}/readyz" >/dev/null; then
      echo "qa: API ready at http://localhost:${api_port}"
      qa_after_api
      return 0
    fi
    if ! kill -0 "$backend_pid" 2>/dev/null; then
      echo "qa: backend exited before becoming ready; see $state_dir/backend.log" >&2
      return 1
    fi
    sleep 2
  done

  echo "qa: backend readiness timed out; see $state_dir/backend.log" >&2
  return 1
{% else %}  echo "qa: this product has no generated backend; native QA will cover the app's error state"
{% endif %}}

if [[ "$action" == stop ]]; then
  qa_services_stop
  echo "qa: disposable services stopped"
  exit 0
fi
[[ "$action" == start ]] || {
  echo "usage: $0 [start|stop]" >&2
  exit 2
}
mkdir -p "$state_dir"
qa_services_stop
qa_services_start
