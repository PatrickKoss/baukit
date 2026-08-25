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
k3s_image=${BAUKIT_PLATFORM_K3S_IMAGE:-rancher/k3s:v1.34.8-k3s1}
git_port=${BAUKIT_PLATFORM_GIT_PORT:-9418}

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
      printf 'KEYCLOAK_ADMIN_USER=%q\n' admin
      printf 'KEYCLOAK_ADMIN_PASSWORD=%q\n' "$(random_value 18)"
      printf 'KEYCLOAK_DB_PASSWORD=%q\n' "$(random_value 18)"
      printf 'POSTGRES_PRODUCT_PASSWORD=%q\n' "$(random_value 18)"
      printf 'GRAFANA_ADMIN_USER=%q\n' admin
      printf 'GRAFANA_ADMIN_PASSWORD=%q\n' "$(random_value 18)"
      printf 'MINIO_ROOT_USER=%q\n' "local-$(random_value 6)"
      printf 'MINIO_ROOT_PASSWORD=%q\n' "$(random_value 18)"
      printf 'FIXTURE_DB_PASSWORD=%q\n' "$(random_value 18)"
      printf 'FIXTURE_OIDC_PASSWORD=%q\n' "$(random_value 12)"
    } >"$secrets_file"
    chmod 0600 "$secrets_file"
  fi
  # shellcheck disable=SC1090
  source "$secrets_file"
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
  (
    cd "$repo_root"
    tar \
      --exclude=.git \
      --exclude=.generated-fixture \
      --exclude=deploy/platform/.local-state \
      --exclude='*/target' \
      --exclude='*/node_modules' \
      --exclude='*/.turbo' \
      -cf - .
  ) | tar -xf - -C "$snapshot_tmp"
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

  nohup python3 "$platform_dir/local-git-http.py" \
    --root "$git_root" --port "$git_port" \
    >"$state_dir/git-http.log" 2>&1 &
  printf '%s\n' "$!" >"$git_pid_file"

  for _ in {1..50}; do
    git ls-remote "http://127.0.0.1:$git_port/baukit.git" HEAD >/dev/null 2>&1 && return
    sleep 0.1
  done
  printf 'ERROR: local git daemon did not become reachable on port %s\n' "$git_port" >&2
  exit 1
}

apply_secret() {
  local namespace=$1
  local name=$2
  shift 2
  kubectl -n "$namespace" create secret generic "$name" "$@" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

apply_runtime_identity() {
  local namespace
  for namespace in postgres keycloak observability; do
    kubectl create namespace "$namespace" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
    kubectl label namespace "$namespace" app.kubernetes.io/part-of=baukit-platform --overwrite >/dev/null
  done

  apply_secret postgres product-db-credentials \
    --type=kubernetes.io/basic-auth \
    --from-literal=username=product_owner \
    --from-literal=password="$POSTGRES_PRODUCT_PASSWORD"
  apply_secret postgres postgres-backup-credentials \
    --from-literal=ACCESS_KEY_ID="$MINIO_ROOT_USER" \
    --from-literal=ACCESS_SECRET_KEY="$MINIO_ROOT_PASSWORD"
  apply_secret postgres minio-root \
    --from-literal=username="$MINIO_ROOT_USER" \
    --from-literal=password="$MINIO_ROOT_PASSWORD"
  apply_secret postgres keycloak-db-credentials \
    --type=kubernetes.io/basic-auth \
    --from-literal=username=keycloak_owner \
    --from-literal=password="$KEYCLOAK_DB_PASSWORD"
  apply_secret keycloak keycloak-db-credentials \
    --type=kubernetes.io/basic-auth \
    --from-literal=username=keycloak_owner \
    --from-literal=password="$KEYCLOAK_DB_PASSWORD"
  apply_secret keycloak keycloak-bootstrap-admin \
    --from-literal=username="$KEYCLOAK_ADMIN_USER" \
    --from-literal=password="$KEYCLOAK_ADMIN_PASSWORD"
  apply_secret observability grafana-admin \
    --from-literal=admin-user="$GRAFANA_ADMIN_USER" \
    --from-literal=admin-password="$GRAFANA_ADMIN_PASSWORD"
  kubectl -n observability create configmap observability-cluster \
    --from-literal=environment=local --dry-run=client -o yaml | kubectl apply -f - >/dev/null
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
  printf '\nLocal credentials (stored mode 0600 in %s):\n' "$secrets_file"
  printf '  Keycloak bootstrap: %s / %s\n' "$KEYCLOAK_ADMIN_USER" "$KEYCLOAK_ADMIN_PASSWORD"
  printf '  Grafana:            %s / %s\n' "$GRAFANA_ADMIN_USER" "$GRAFANA_ADMIN_PASSWORD"
  printf '  MinIO:              %s / %s\n' "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD"
  printf '  Fixture OIDC user:  test / %s\n' "$FIXTURE_OIDC_PASSWORD"
  printf '  Keycloak service:   http://keycloak-service.keycloak.svc.cluster.local:8080\n'
}

up() {
  require_tools
  ensure_secret_state
  start_git_daemon

  if ! cluster_exists; then
    printf 'Creating k3d cluster %s with %s\n' "$cluster_name" "$k3s_image"
    k3d cluster create "$cluster_name" \
      --image "$k3s_image" \
      --servers 1 \
      --agents 0 \
      --k3s-arg '--disable=traefik@server:0' \
      --port '80:80@loadbalancer' \
      --port '443:443@loadbalancer' \
      --wait
  else
    printf 'Cluster %s already exists; converging it.\n' "$cluster_name"
  fi
  use_context

  printf 'Installing Flux v2.9.4 controllers\n'
  flux install \
    --version=v2.9.4 \
    --components=source-controller,kustomize-controller,helm-controller \
    --network-policy=false \
    --timeout=5m >/dev/null
  kubectl -n flux-system rollout status deployment/source-controller --timeout=5m
  kubectl -n flux-system rollout status deployment/kustomize-controller --timeout=5m
  kubectl -n flux-system rollout status deployment/helm-controller --timeout=5m

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

case ${1:-} in
  up) up ;;
  down) down ;;
  status) status ;;
  *) usage ;;
esac
