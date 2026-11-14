#!/usr/bin/env bash
set -euo pipefail

e2e_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
platform_dir=$(cd -- "$e2e_dir/.." && pwd)
repo_root=$(cd -- "$platform_dir/../.." && pwd)

# shellcheck source=deploy/platform/local-git-http.sh
source "$platform_dir/local-git-http.sh"
# shellcheck source=deploy/lib/tools.sh
. "$repo_root/deploy/lib/tools.sh"

snapshot_port=9418
echo_image=localhost/baukit-e2e/echo:local
ready_timeout=30m
stats_interval_seconds=15

cluster_name=baukit-platform-e2e
keep_cluster=false
test_dirs=("$e2e_dir/tests")
without=()
heavy_slot_dir=${BAUKIT_HEAVY_SLOT_DIR:-/tmp/claude-heavy-slots}
heavy_slot_poll_seconds=${BAUKIT_HEAVY_SLOT_POLL_SECONDS:-30}
cluster_lock=${BAUKIT_E2E_CLUSTER_LOCK:-$HOME/.cache/baukit-orchestrator/platform-hardening/k3d-full.lock}
artifacts_dir=${BAUKIT_E2E_ARTIFACTS:-$repo_root/.ci-gate/platform-e2e}
state_parent=${BAUKIT_E2E_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/baukit-platform-e2e}
flux_version=

usage() {
  cat >&2 <<EOF
usage: ${0##*/} [--keep] [--suite DIR] [--cluster-name NAME] [--without NAME]...

Creates a disposable k3d cluster with the composed platform, runs the chainsaw
suites under deploy/platform/e2e/tests and deletes the cluster again.

  --keep               keep the cluster and snapshot server after the run
  --suite DIR          run one test directory, for example tests/smoke
  --cluster-name NAME  k3d cluster name (default: baukit-platform-e2e)
  --without NAME       leave out the Flux Kustomization e2e-NAME and drop it from
                       every dependsOn list; repeatable
EOF
  exit 2
}

parse_args() {
  while (($#)); do
    case $1 in
      --keep) keep_cluster=true ;;
      --suite)
        (($# >= 2)) || usage
        test_dirs=("$(suite_path "$2")")
        shift
        ;;
      --cluster-name)
        (($# >= 2)) || usage
        cluster_name=$2
        shift
        ;;
      --without)
        (($# >= 2)) || usage
        without+=("e2e-$2")
        shift
        ;;
      *) usage ;;
    esac
    shift
  done
  if [[ ! $cluster_name =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
    printf 'ERROR: --cluster-name must contain only lowercase letters, digits or hyphens\n' >&2
    exit 2
  fi
}

suite_path() {
  local candidate
  for candidate in "$1" "$e2e_dir/$1" "$e2e_dir/tests/$1"; do
    if [[ -f $candidate/chainsaw-test.yaml ]]; then
      (cd -- "$candidate" && pwd)
      return
    fi
  done
  printf 'ERROR: no chainsaw-test.yaml in suite %s\n' "$1" >&2
  exit 2
}

require_tools() {
  baukit_use_tools "$repo_root" chainsaw flux k3d kubectl
  flux_version=v$(baukit_tool_version "$repo_root" flux)
  local missing=() tool
  for tool in docker flock git openssl python3 tar; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]})); then
    printf 'ERROR: missing required tools: %s\n' "${missing[*]}" >&2
    exit 2
  fi
}

take_cluster_lock() {
  mkdir -p -- "$(dirname -- "$cluster_lock")"
  local fd
  exec {fd}>"$cluster_lock"
  printf 'Waiting for %s\n' "$cluster_lock"
  flock "$fd"
}

# Other projects on this host hold at most two heavy runs, one per slot directory.
wait_for_heavy_slot() {
  while [[ -e $heavy_slot_dir/slot-1 && -e $heavy_slot_dir/slot-2 ]]; do
    printf 'Waiting for a free heavy-run slot: %s/slot-1 and slot-2 both exist\n' "$heavy_slot_dir"
    sleep "$heavy_slot_poll_seconds"
  done
}

state_dir=
kubeconfig=
snapshot_container=
stats_pid=
cluster_created=false

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n $stats_pid ]]; then
    kill "$stats_pid" 2>/dev/null || true
    wait "$stats_pid" 2>/dev/null || true
  fi
  if [[ $cluster_created == true ]]; then
    collect_artifacts || printf 'WARNING: artifact collection failed\n' >&2
  fi
  if [[ $keep_cluster == true ]]; then
    printf 'Kept cluster %s. KUBECONFIG=%s\n' "$cluster_name" "$kubeconfig"
  else
    [[ $cluster_created == true ]] && k3d cluster delete "$cluster_name" >/dev/null
    [[ -n $snapshot_container ]] && docker rm -f -v "$snapshot_container" >/dev/null 2>&1
    [[ -n $state_dir ]] && rm -rf -- "$state_dir"
  fi
  printf 'Artifacts: %s\n' "$artifacts_dir"
  exit "$status"
}

interrupted() {
  exit 130
}

cluster_exists() {
  k3d cluster list --no-headers 2>/dev/null | awk '{print $1}' | grep -Fqx -- "$cluster_name"
}

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

create_cluster() {
  if cluster_exists; then
    printf 'ERROR: cluster %s already exists; delete it or pass --cluster-name\n' "$cluster_name" >&2
    exit 2
  fi
  local image
  image=$(platform_k3s_image)
  printf 'Creating k3d cluster %s with %s\n' "$cluster_name" "$image"
  cluster_created=true
  k3d_create_local_cluster "$cluster_name" "$image" "$(free_port)" "$(free_port)" "$(free_port)" \
    --kubeconfig-update-default=false \
    --kubeconfig-switch-context=false >/dev/null
  k3d kubeconfig get "$cluster_name" >"$kubeconfig"
  chmod 0600 "$kubeconfig"
  kubectl wait node --all --for=condition=Ready --timeout=5m >/dev/null
}

serve_snapshot() {
  local tree=$state_dir/tree git_root=$state_dir/git image=$cluster_name-git-http
  mkdir -p "$tree" "$git_root"
  snapshot_tree "$repo_root" "$tree"
  (
    cd "$tree"
    git init -q -b main
    git add -A
    GIT_AUTHOR_DATE='2000-01-01T00:00:00Z' GIT_COMMITTER_DATE='2000-01-01T00:00:00Z' \
      git -c user.name='Baukit e2e' -c user.email='e2e@invalid' commit -q -m 'Baukit e2e snapshot'
  )
  git clone -q --bare "$tree" "$git_root/baukit.git"
  docker build -q -f "$platform_dir/local-git-http.Dockerfile" -t "$image" "$platform_dir" >/dev/null
  local gateway
  gateway=$(k3d_gateway "$cluster_name")
  snapshot_container=$cluster_name-git
  docker rm -f -v "$snapshot_container" >/dev/null 2>&1 || true
  run_snapshot_container "$snapshot_container" "$image" "$git_root" "$snapshot_port" "$gateway" >/dev/null
  local attempt
  for ((attempt = 0; attempt < 100; attempt++)); do
    git ls-remote "http://$gateway:$snapshot_port/baukit.git" HEAD >/dev/null 2>&1 && return
    sleep 0.1
  done
  printf 'ERROR: snapshot server did not become ready; inspect docker logs %s\n' "$snapshot_container" >&2
  exit 1
}

import_echo_image() {
  docker build -q -t "$echo_image" "$e2e_dir/echo" >/dev/null
  k3d image import -c "$cluster_name" "$echo_image" >/dev/null
}

apply_secret() {
  local namespace=$1 name=$2
  shift 2
  kubectl -n "$namespace" create secret generic "$name" "$@" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

apply_runtime_identity() {
  local namespace
  for namespace in postgres observability; do
    kubectl create namespace "$namespace" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
    kubectl label namespace "$namespace" app.kubernetes.io/part-of=baukit-platform --overwrite >/dev/null
  done
  local access_key secret_key
  access_key="e2e-$(openssl rand -hex 6)"
  secret_key=$(openssl rand -hex 18)
  apply_secret postgres product-db-credentials --type=kubernetes.io/basic-auth \
    --from-literal=username=product_owner --from-literal=password="$(openssl rand -hex 18)"
  apply_secret postgres postgres-backup-credentials \
    --from-literal=ACCESS_KEY_ID="$access_key" --from-literal=ACCESS_SECRET_KEY="$secret_key"
  apply_secret postgres rustfs-root \
    --from-literal=username="$access_key" --from-literal=password="$secret_key"
  apply_secret observability rustfs-root \
    --from-literal=username="$access_key" --from-literal=password="$secret_key"
  apply_secret observability grafana-admin \
    --from-literal=admin-user=admin --from-literal=admin-password="$(openssl rand -hex 18)"
  kubectl -n observability create configmap observability-cluster \
    --from-literal=environment=e2e --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

install_flux() {
  printf 'Installing Flux %s\n' "$flux_version"
  flux install --version="$flux_version" --network-policy=false --timeout=5m >/dev/null
}

sample_stats() {
  local containers=()
  mapfile -t containers < <(docker ps -q --filter "label=k3d.cluster=$cluster_name")
  ((${#containers[@]})) || return 0
  while true; do
    printf '%s\n' "$(date -u +%FT%TZ)"
    docker stats --no-stream --format '{{.Name}} {{.CPUPerc}} {{.MemUsage}}' "${containers[@]}" || true
    sleep "$stats_interval_seconds"
  done >>"$artifacts_dir/docker-stats.log"
}

validate_without() {
  local rendered name
  rendered=$(kubectl kustomize "$e2e_dir/overlay")
  for name in "${without[@]}"; do
    if ! grep -Fqx "  name: $name" <<<"$rendered"; then
      printf 'ERROR: --without names no Kustomization in the overlay: %s\n' "$name" >&2
      exit 2
    fi
  done
}

# Needs the cluster: kubectl converts the YAML to JSON with the server's type mapping.
# Renders the overlay as a JSON List without the Kustomizations named in $without.
render_overlay() {
  kubectl kustomize "$e2e_dir/overlay" |
    kubectl create --dry-run=client --validate=false -o json -f - |
    python3 "$e2e_dir/without.py" "${without[@]}"
}

report_not_ready() {
  printf 'Flux objects that are not Ready:\n' >&2
  kubectl get kustomizations.kustomize.toolkit.fluxcd.io,helmreleases.helm.toolkit.fluxcd.io -A --no-headers \
    -o 'custom-columns=KIND:.kind,NAMESPACE:.metadata.namespace,NAME:.metadata.name,READY:.status.conditions[?(@.type=="Ready")].status,MESSAGE:.status.conditions[?(@.type=="Ready")].message' |
    awk '$4 != "True"' >&2 || true
}

wait_for_platform() {
  render_overlay | kubectl apply -f - >/dev/null
  flux reconcile source git baukit --namespace flux-system --timeout=2m >/dev/null
  printf 'Waiting up to %s for every Kustomization and HelmRelease\n' "$ready_timeout"
  if ! kubectl wait kustomizations.kustomize.toolkit.fluxcd.io -A --all \
    --for=condition=Ready --timeout="$ready_timeout" >/dev/null ||
    ! kubectl wait helmreleases.helm.toolkit.fluxcd.io -A --all \
      --for=condition=Ready --timeout=5m >/dev/null; then
    report_not_ready
    return 1
  fi
}

run_tests() {
  local test_dir args=()
  for test_dir in "${test_dirs[@]}"; do
    args+=(--test-dir "$test_dir")
  done
  chainsaw test "${args[@]}" \
    --report-format JUNIT-TEST \
    --report-path "$artifacts_dir" \
    --report-name chainsaw-report
}

collect_artifacts() {
  printf 'Collecting artifacts into %s\n' "$artifacts_dir"
  kubectl get events -A --sort-by=.lastTimestamp >"$artifacts_dir/events.txt" 2>&1 || true
  kubectl get policyreports,clusterpolicyreports -A -o wide >"$artifacts_dir/policy-reports.txt" 2>&1 || true
  flux get all -A >"$artifacts_dir/flux-status.txt" 2>&1 || true
  kubectl get pods -A -o wide >"$artifacts_dir/pods.txt" 2>&1 || true
  mkdir -p "$artifacts_dir/logs"
  local namespace pod
  while read -r namespace pod; do
    kubectl -n "$namespace" logs "$pod" --all-containers --prefix --tail=2000 \
      >"$artifacts_dir/logs/$namespace.$pod.log" 2>&1 || true
  done < <(kubectl get pods -A --no-headers -o custom-columns=NS:.metadata.namespace,NAME:.metadata.name)
}

main() {
  parse_args "$@"
  require_tools
  validate_without
  take_cluster_lock
  wait_for_heavy_slot
  rm -rf -- "$artifacts_dir"
  mkdir -p "$artifacts_dir"
  mkdir -p -- "$state_parent"
  state_dir=$(mktemp -d "$state_parent/run.XXXXXX")
  kubeconfig=$state_dir/kubeconfig
  export KUBECONFIG=$kubeconfig
  trap cleanup EXIT
  trap interrupted INT TERM

  local started=$SECONDS
  create_cluster
  sample_stats &
  stats_pid=$!
  serve_snapshot
  import_echo_image
  install_flux
  apply_runtime_identity
  wait_for_platform
  printf 'Platform Ready after %ss\n' "$((SECONDS - started))"
  run_tests
  printf 'platform-e2e passed in %ss\n' "$((SECONDS - started))"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
