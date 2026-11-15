#!/usr/bin/env bash
set -euo pipefail

e2e_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
platform_dir=$(cd -- "$e2e_dir/.." && pwd)
repo_root=$(cd -- "$platform_dir/../.." && pwd)

# shellcheck source=deploy/platform/local-git-http.sh
source "$platform_dir/local-git-http.sh"
# shellcheck source=deploy/platform/flux-lockdown.sh
source "$platform_dir/flux-lockdown.sh"
# shellcheck source=deploy/platform/runtime-identity.sh
source "$platform_dir/runtime-identity.sh"
# shellcheck source=deploy/platform/registry-mirrors.sh
source "$platform_dir/registry-mirrors.sh"
# shellcheck source=deploy/lib/tools.sh
. "$repo_root/deploy/lib/tools.sh"

snapshot_port=9418
echo_image=localhost/baukit-e2e/echo:local
ready_timeout=30m
stats_interval_seconds=15
e2e_subnet_first=100
e2e_subnet_last=199

cluster_name=baukit-platform-e2e
keep_cluster=false
test_dirs=("$e2e_dir/tests")
without=()
heavy_slot_dir=${BAUKIT_HEAVY_SLOT_DIR:-/tmp/claude-heavy-slots}
ready_poll_seconds=30
dependency_requeue=5s
failed_poll_limit=3
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
  baukit_use_tools "$repo_root" chainsaw flux k3d kubectl kustomize
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
started=

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
    if [[ $cluster_created == true ]]; then
      registry_mirrors_detach "k3d-$cluster_name"
      k3d cluster delete "$cluster_name" >/dev/null
    fi
    [[ -n $snapshot_container ]] && docker rm -f -v "$snapshot_container" >/dev/null 2>&1
    [[ -n $state_dir ]] && rm -rf -- "$state_dir"
  fi
  printf 'Artifacts: %s\n' "$artifacts_dir"
  if [[ -n $started ]]; then
    printf 'platform-e2e cluster time %ss, exit %s\n' "$((SECONDS - started))" "$status"
  fi
  exit "$status"
}

interrupted() {
  exit 130
}

cluster_exists() {
  k3d cluster list --no-headers 2>/dev/null | awk '{print $1}' | grep -Fqx -- "$cluster_name"
}

# The cluster network must sit inside nodeCIDR (10.89.0.0/16) from overlays/local/cluster-network, so
# the node-cidr NetworkPolicies match the node. Picks the first /24 no Docker network uses.
free_subnet() {
  if [[ -n ${BAUKIT_E2E_SUBNET:-} ]]; then
    printf '%s\n' "$BAUKIT_E2E_SUBNET"
    return
  fi
  local used third
  used=$(docker network ls -q | xargs -r docker network inspect --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}')
  for ((third = e2e_subnet_first; third <= e2e_subnet_last; third++)); do
    if [[ " $used " != *" 10.89.$third.0/24 "* ]]; then
      printf '10.89.%s.0/24\n' "$third"
      return
    fi
  done
  printf 'ERROR: no free /24 in 10.89.%s-%s.0; set BAUKIT_E2E_SUBNET\n' "$e2e_subnet_first" "$e2e_subnet_last" >&2
  exit 1
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
  registry_mirrors_start
  registry_mirrors_k3s_config >"$state_dir/registries.yaml"
  local mirror_args=()
  mapfile -t mirror_args < <(registry_mirrors_cluster_args)
  k3d_create_local_cluster "$cluster_name" "$image" "$(free_port)" "$(free_port)" "$(free_port)" \
    --subnet "$(free_subnet)" \
    --registry-config "$state_dir/registries.yaml" \
    "${mirror_args[@]}" \
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

apply_runtime_identity() {
  platform_runtime_identity e2e "$(openssl rand -hex 18)" "e2e-$(openssl rand -hex 6)" \
    "$(openssl rand -hex 18)" admin "$(openssl rand -hex 18)"
}

# The platform is a dependsOn chain, and each hop otherwise waits the controllers' 30s default.
shorten_dependency_requeue() {
  local controller
  for controller in kustomize-controller helm-controller; do
    kubectl -n flux-system patch deployment "$controller" --type json -p \
      "[{\"op\":\"add\",\"path\":\"/spec/template/spec/containers/0/args/-\",\"value\":\"--requeue-dependency=${dependency_requeue}\"}]" \
      >/dev/null
  done
}

install_flux() {
  printf 'Installing Flux %s with the multi-tenancy lockdown\n' "$flux_version"
  flux_lockdown_render "$platform_dir" "$flux_version" | kubectl apply --server-side -f - >/dev/null
  kubectl label namespace flux-system --overwrite \
    pod-security.kubernetes.io/enforce=restricted \
    pod-security.kubernetes.io/enforce-version=v1.37 \
    pod-security.kubernetes.io/audit=restricted \
    pod-security.kubernetes.io/warn=restricted >/dev/null
  shorten_dependency_requeue
  flux_lockdown_wait
  flux_lockdown_verify "$platform_dir"
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

# Prints the HelmReleases whose remediation retries are used up; Flux marks them Stalled.
stalled_releases() {
  kubectl get helmreleases.helm.toolkit.fluxcd.io -A -o json | python3 -c '
import json, sys
for item in json.load(sys.stdin)["items"]:
    conditions = item.get("status", {}).get("conditions", [])
    if any(c["type"] == "Stalled" and c["status"] == "True" for c in conditions):
        print(item["metadata"]["namespace"] + "/" + item["metadata"]["name"])
'
}

# Prints the Kustomizations whose Ready=False reason will not clear on its own.
failed_kustomizations() {
  kubectl get kustomizations.kustomize.toolkit.fluxcd.io -A -o json | python3 -c '
import json, sys
for item in json.load(sys.stdin)["items"]:
    for condition in item.get("status", {}).get("conditions", []):
        if condition["type"] == "Ready" and condition["status"] == "False" and condition.get(
            "reason"
        ) in ("ReconciliationFailed", "BuildFailed"):
            print(item["metadata"]["namespace"] + "/" + item["metadata"]["name"])
'
}

# Usage: persistent_failures <count-file> <failed names>. Bumps each name's streak, drops names that
# recovered, and prints the names failed for failed_poll_limit polls in a row.
persistent_failures() {
  local counts=$1 failed=$2 previous name streak
  previous=$(cat -- "$counts" 2>/dev/null || true)
  : >"$counts"
  while read -r name; do
    [[ -n $name ]] || continue
    streak=$(awk -v n="$name" '$1 == n {print $2}' <<<"$previous")
    streak=$((${streak:-0} + 1))
    printf '%s %s\n' "$name" "$streak" >>"$counts"
    ((streak >= failed_poll_limit)) && printf '%s\n' "$name"
  done <<<"$failed"
  return 0
}

wait_for_platform() {
  render_overlay | kubectl apply -f - >/dev/null
  flux reconcile source git baukit --namespace flux-system --timeout=2m >/dev/null
  printf 'Waiting up to %s for every Kustomization and HelmRelease\n' "$ready_timeout"
  local deadline=$((SECONDS + ${ready_timeout%m} * 60)) stalled failed counts=$state_dir/failed-polls
  until kubectl wait kustomizations.kustomize.toolkit.fluxcd.io -A --all \
    --for=condition=Ready --timeout="${ready_poll_seconds}s" >/dev/null 2>&1; do
    stalled=$(stalled_releases)
    failed=$(persistent_failures "$counts" "$(failed_kustomizations)")
    if [[ -n $stalled || -n $failed ]] || ((SECONDS >= deadline)); then
      [[ -n $stalled ]] && printf 'Stalled HelmReleases, retries exhausted:\n%s\n' "$stalled"
      [[ -n $failed ]] && printf 'Kustomizations failing for %s polls:\n%s\n' "$failed_poll_limit" "$failed"
      report_not_ready
      return 1
    fi
  done
  if ! kubectl wait helmreleases.helm.toolkit.fluxcd.io -A --all \
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
    --config "$e2e_dir/chainsaw-config.yaml" \
    --report-format JUNIT-TEST \
    --report-path "$artifacts_dir" \
    --report-name chainsaw-report
}

collect_artifacts() {
  printf 'Collecting artifacts into %s\n' "$artifacts_dir"
  kubectl get events -A --sort-by=.lastTimestamp >"$artifacts_dir/events.txt" 2>&1 || true
  kubectl get policyreports,clusterpolicyreports -A -o wide >"$artifacts_dir/policy-reports.txt" 2>&1 || true
  flux get all -A >"$artifacts_dir/flux-status.txt" 2>&1 || true
  kubectl get nodes,pods -A -o wide >"$artifacts_dir/pods.txt" 2>&1 || true
  kubectl get networkpolicies -A -o yaml >"$artifacts_dir/network-policies.yaml" 2>&1 || true
  kubectl get jobs -A -o yaml >"$artifacts_dir/jobs.yaml" 2>&1 || true
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

  started=$SECONDS
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
