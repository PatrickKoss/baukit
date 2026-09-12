#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="$root/mobile/.qa"
action="${1:-start}"
compose_project="${BAUKIT_QA_COMPOSE_PROJECT:-{{ context.app_name }}-mobile-qa}"
postgres_port="${BAUKIT_QA_POSTGRES_PORT:-15432}"
redis_port="${BAUKIT_QA_REDIS_PORT:-16379}"
api_port="${BAUKIT_QA_API_PORT:-18080}"
ops_port="${BAUKIT_QA_OPS_PORT:-19090}"
{% if context.auth_oidc %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}
compose=(
  docker compose
  --project-name "$compose_project"
  --project-directory "$root"
  -f "$root/compose.yaml"
  -f "$root/mobile/scripts/qa/docker-compose.qa.yml"
)

stop_api() {
  if [[ -f "$state_dir/backend.pid" ]]; then
    pid="$(<"$state_dir/backend.pid")"
    if [[ "$pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      for _ in $(seq 1 30); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
      done
    fi
    rm -f "$state_dir/backend.pid"
  fi
}

stop_services() {
  stop_api
{% if context.backend %}  "${compose[@]}" down --volumes >/dev/null 2>&1 || true
{% endif %}}

if [[ "$action" == stop ]]; then
  stop_services
  echo "qa: disposable services stopped"
  exit 0
fi
[[ "$action" == start ]] || {
  echo "usage: $0 [start|stop]" >&2
  exit 2
}

mkdir -p "$state_dir"
stop_services

{% if context.backend %}command -v docker >/dev/null || { echo "qa: docker is required" >&2; exit 1; }
command -v cargo >/dev/null || { echo "qa: cargo is required" >&2; exit 1; }
command -v curl >/dev/null || { echo "qa: curl is required" >&2; exit 1; }

echo "qa: starting disposable PostgreSQL, Redis{% if context.auth_oidc %}, and Keycloak{% endif %}"
"${compose[@]}" up -d --wait

export {{ context.app_env }}__DATABASE__URL="postgres://postgres:postgres@127.0.0.1:${postgres_port}/{{ context.app_crate }}"
export {{ context.app_env }}__RATE_LIMIT__REDIS_URL="redis://127.0.0.1:${redis_port}/"
export {{ context.app_env }}__HTTP__BIND_ADDRESS=127.0.0.1
export {{ context.app_env }}__HTTP__PORT="$api_port"
export {{ context.app_env }}__OPS__BIND_ADDRESS=127.0.0.1
export {{ context.app_env }}__OPS__PORT="$ops_port"
{% if context.auth_oidc %}export {{ context.app_env }}__AUTH__ISSUER="http://localhost:${keycloak_port}/realms/{{ context.app_name }}"
export {{ context.app_env }}__AUTH__AUDIENCE="{{ context.app_name }}-backend"
{% endif %}

cargo build --locked --manifest-path "$root/backend/Cargo.toml" \
  -p {{ context.app_name }}-bin --bin migrate --bin api
"$root/backend/target/debug/migrate"

nohup "$root/backend/target/debug/api" \
  >"$state_dir/backend.log" 2>&1 &
backend_pid=$!
printf '%s\n' "$backend_pid" > "$state_dir/backend.pid"

for _ in $(seq 1 120); do
  if curl --fail --silent "http://127.0.0.1:${ops_port}/readyz" >/dev/null; then
    echo "qa: API ready at http://localhost:${api_port}"
    exit 0
  fi
  if ! kill -0 "$backend_pid" 2>/dev/null; then
    echo "qa: backend exited before becoming ready; see $state_dir/backend.log" >&2
    exit 1
  fi
  sleep 2
done

echo "qa: backend readiness timed out; see $state_dir/backend.log" >&2
exit 1
{% else %}echo "qa: this product has no generated backend; native QA will cover the app's error state"
{% endif %}
