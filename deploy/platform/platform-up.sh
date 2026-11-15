#!/usr/bin/env bash
set -euo pipefail

platform_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$platform_dir/../.." && pwd)
state_dir=${BAUKIT_PLATFORM_STATE_DIR:-$platform_dir/.local-state}
secrets_file=$state_dir/secrets.env
git_root=$state_dir/git
git_pid_file=$state_dir/git-daemon.pid

cluster_name=${BAUKIT_PLATFORM_CLUSTER:-baukit-local}
# Kubernetes 1.35 removed cgroup-v1 support. The current development host is a
# cgroup-v1 WSL2 Docker engine, so the local flavor uses the newest supported
# k3s 1.34 patch while testing/production retain the I0 stable-channel pin.
git_port=${BAUKIT_PLATFORM_GIT_PORT:-9418}
api_port=${BAUKIT_PLATFORM_API_PORT:-random}
http_port=${BAUKIT_PLATFORM_HTTP_PORT:-80}
https_port=${BAUKIT_PLATFORM_HTTPS_PORT:-443}
# Must sit inside nodeCIDR in overlays/local/cluster-network/cluster-network.yaml.
k3d_subnet=${BAUKIT_PLATFORM_SUBNET:-10.89.0.0/24}

# shellcheck source=deploy/platform/local-git-http.sh
source "$platform_dir/local-git-http.sh"
# shellcheck source=deploy/platform/flux-lockdown.sh
source "$platform_dir/flux-lockdown.sh"
# shellcheck source=deploy/platform/runtime-identity.sh
source "$platform_dir/runtime-identity.sh"
k3s_image=${BAUKIT_PLATFORM_K3S_IMAGE:-$(platform_k3s_image)}

export PATH="$HOME/.local/bin:$PATH"

usage() {
  printf 'usage: %s up|down|status\n' "$0" >&2
  exit 2
}

require_tools() {
  local missing=()
  local tool
  for tool in docker flux git k3d kubectl kustomize openssl python3 tar; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]} > 0)); then
    printf 'ERROR: missing required tools: %s\n' "${missing[*]}" >&2
    printf 'Pinned local tools belong in ~/.local/bin (k3d 5.8.3, kubectl 1.36.3, Flux 2.9.4).\n' >&2
    exit 2
  fi
}

cluster_exists() {
  k3d cluster list --no-headers 2>/dev/null | awk '{print $1}' | grep -qx "$cluster_name"
}

use_context() {
  kubectl config use-context "k3d-$cluster_name" >/dev/null
}

random_value() {
  openssl rand -hex "$1"
}

ensure_secret_state() {
  mkdir -p "$state_dir"
  chmod 0700 "$state_dir"
  if [[ ! -f $secrets_file ]]; then
    umask 077
    {
      printf 'POSTGRES_PRODUCT_PASSWORD=%q\n' "$(random_value 18)"
      printf 'GRAFANA_ADMIN_USER=%q\n' admin
      printf 'GRAFANA_ADMIN_PASSWORD=%q\n' "$(random_value 18)"
      printf 'RUSTFS_ACCESS_KEY=%q\n' "local-$(random_value 6)"
      printf 'RUSTFS_SECRET_KEY=%q\n' "$(random_value 18)"
      printf 'FIXTURE_DB_PASSWORD=%q\n' "$(random_value 18)"
    } >"$secrets_file"
    chmod 0600 "$secrets_file"
  fi
  # shellcheck disable=SC1090 # the secrets file is generated above, so its path is only known at run time
  source "$secrets_file"
  if [[ -z ${RUSTFS_ACCESS_KEY:-} || -z ${RUSTFS_SECRET_KEY:-} ]]; then
    RUSTFS_ACCESS_KEY="local-$(random_value 6)"
    RUSTFS_SECRET_KEY=$(random_value 18)
    {
      printf 'RUSTFS_ACCESS_KEY=%q\n' "$RUSTFS_ACCESS_KEY"
      printf 'RUSTFS_SECRET_KEY=%q\n' "$RUSTFS_SECRET_KEY"
    } >>"$secrets_file"
  fi
}

stop_git_daemon() {
  if [[ -f $git_pid_file ]]; then
    local daemon_pid
    daemon_pid=$(<"$git_pid_file")
    if [[ $daemon_pid =~ ^[0-9]+$ ]] && kill -0 "$daemon_pid" 2>/dev/null; then
      kill "$daemon_pid"
      for _ in {1..50}; do
        kill -0 "$daemon_pid" 2>/dev/null || break
        sleep 0.1
      done
    fi
    rm -f -- "$git_pid_file"
  fi
}

start_git_daemon() {
  local snapshot=$state_dir/git-snapshot
  local snapshot_tmp=$state_dir/git-snapshot.new
  local git_tmp=$state_dir/git.new

  stop_git_daemon
  rm -rf -- "$snapshot_tmp" "$git_tmp"
  mkdir -p "$snapshot_tmp" "$git_tmp"
  snapshot_tree "$repo_root" "$snapshot_tmp"
  (
    cd "$snapshot_tmp"
    git init -q -b main
    git config user.name 'Baukit local platform'
    git config user.email 'local-platform@invalid'
    git add -A
    git commit -q -m 'Disposable local platform snapshot'
  )
  git clone -q --bare "$snapshot_tmp" "$git_tmp/baukit.git"
  git --git-dir="$git_tmp/baukit.git" update-server-info
  rm -rf -- "$snapshot" "$git_root"
  mv -- "$snapshot_tmp" "$snapshot"
  mv -- "$git_tmp" "$git_root"

  local gateway
  gateway=$(k3d_gateway "$cluster_name")
  nohup python3 "$platform_dir/local-git-http.py" \
    --host "$gateway" --root "$git_root" --port "$git_port" \
    >"$state_dir/git-http.log" 2>&1 &
  printf '%s\n' "$!" >"$git_pid_file"

  for _ in {1..50}; do
    git ls-remote "http://$gateway:$git_port/baukit.git" HEAD >/dev/null 2>&1 && return
    sleep 0.1
  done
  printf 'ERROR: local git daemon did not become reachable on port %s\n' "$git_port" >&2
  exit 1
}

apply_runtime_identity() {
  platform_runtime_identity local "$POSTGRES_PRODUCT_PASSWORD" "$RUSTFS_ACCESS_KEY" \
    "$RUSTFS_SECRET_KEY" "$GRAFANA_ADMIN_USER" "$GRAFANA_ADMIN_PASSWORD"
  platform_ensure_namespace posthog
  platform_apply_secret posthog posthog-object-storage \
    --from-literal=root-user="$RUSTFS_ACCESS_KEY" \
    --from-literal=root-password="$RUSTFS_SECRET_KEY"
}

print_diagnostics() {
  printf '\nFlux reconciliation:\n'
  flux get sources git -A || true
  flux get kustomizations -A || true
  flux get helmreleases -A || true
  printf '\nNon-ready pods:\n'
  kubectl get pods -A --field-selector=status.phase!=Running,status.phase!=Succeeded || true
}

status() {
  require_tools
  ensure_secret_state
  if ! cluster_exists; then
    printf 'Cluster %s: absent\n' "$cluster_name"
  else
    use_context
    printf 'Cluster %s: running\n' "$cluster_name"
    kubectl get nodes -o wide
    print_diagnostics
  fi
  printf '\nLocal credentials (values are not printed):\n'
  printf '  All generated values: %s (mode 0600)\n' "$secrets_file"
  printf '  Grafana:              Secret observability/grafana-admin (admin-user, admin-password)\n'
  printf '  RustFS:               Secret postgres/rustfs-root (username, password)\n'
  printf '  Product database:     Secret postgres/product-db-credentials (username, password)\n'
}

up() {
  require_tools
  ensure_secret_state
  if ! cluster_exists; then
    printf 'Creating k3d cluster %s with %s\n' "$cluster_name" "$k3s_image"
    k3d_create_local_cluster "$cluster_name" "$k3s_image" "$api_port" "$http_port" "$https_port" \
      --subnet "$k3d_subnet"
  else
    printf 'Cluster %s already exists; converging it.\n' "$cluster_name"
  fi
  use_context
  start_git_daemon

  printf 'Installing Flux v2.9.4 controllers with the multi-tenancy lockdown\n'
  flux_lockdown_render "$platform_dir" v2.9.4 | kubectl apply --server-side -f - >/dev/null
  kubectl label namespace flux-system --overwrite \
    pod-security.kubernetes.io/enforce=restricted \
    pod-security.kubernetes.io/enforce-version=v1.34 \
    pod-security.kubernetes.io/audit=restricted \
    pod-security.kubernetes.io/warn=restricted >/dev/null
  flux_lockdown_wait
  flux_lockdown_verify "$platform_dir"

  apply_runtime_identity
  kubectl apply -k "$platform_dir/overlays/local" >/dev/null
  flux reconcile source git baukit --namespace flux-system --timeout=2m

  printf 'Waiting for the Flux-composed local platform (first pull can take several minutes)\n'
  if ! kubectl -n flux-system wait kustomization --all --for=condition=Ready --timeout=35m; then
    print_diagnostics >&2
    exit 1
  fi
  printf 'Local platform is Ready.\n'
  status
}

down() {
  require_tools
  if cluster_exists; then
    k3d cluster delete "$cluster_name"
  else
    printf 'Cluster %s is already absent.\n' "$cluster_name"
  fi
  stop_git_daemon
  printf 'Local platform stopped. Runtime credentials remain in %s for the next idempotent up.\n' "$state_dir"
}

main() {
  case ${1:-} in
    up) up ;;
    down) down ;;
    status) status ;;
    *) usage ;;
  esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
