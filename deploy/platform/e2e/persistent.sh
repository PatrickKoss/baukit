#!/usr/bin/env bash
# Runs chainsaw suites against an existing cluster with BAUKIT_E2E_PROFILE=persistent and undoes its setup on exit.
set -euo pipefail

e2e_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$e2e_dir/../../.." && pwd)
# shellcheck source=deploy/lib/tools.sh
. "$repo_root/deploy/lib/tools.sh"

echo_image=localhost/baukit-e2e/echo:local
echo_namespace=e2e-tenant
allowlist_kustomization=local-registry-allowlist
allowlist_configmap=baukit-registry-allowlist
context=k3d-platform-local
suites=()
state_dir=""

usage() {
  cat >&2 <<EOF
usage: ${0##*/} [--context CONTEXT] [--suite DIR]...

Runs chainsaw suites from deploy/platform/e2e/tests against a persistent k3d cluster
(default context: k3d-platform-local). It imports the echo image, suspends the Flux
Kustomization $allowlist_kustomization to admit localhost/baukit-e2e/, installs the echo app in
$echo_namespace, and reverts all of it on exit. Default suites: admission netpol falco alert-pipeline.
EOF
  exit 2
}

parse_args() {
  while (($#)); do
    (($# >= 2)) || usage
    case $1 in
      --context) context=$2 ;;
      --suite) suites+=("$2") ;;
      *) usage ;;
    esac
    shift 2
  done
  ((${#suites[@]})) || suites=(admission netpol falco alert-pipeline)
}

pin_context() {
  state_dir=$(mktemp -d)
  kubectl config view --minify --flatten --context "$context" >"$state_dir/kubeconfig"
  export KUBECONFIG=$state_dir/kubeconfig
}

admit_echo_registry() {
  flux suspend kustomization -n flux-system "$allowlist_kustomization"
  local registries
  registries=$(kubectl -n kyverno get configmap "$allowlist_configmap" -o jsonpath='{.data.registries}')
  kubectl -n kyverno patch configmap "$allowlist_configmap" --type merge \
    -p "$(python3 -c 'import json, sys; print(json.dumps({"data": {"registries": sys.argv[1].rstrip() + "\nlocalhost/baukit-e2e/\n"}}))' "$registries")"
}

install_echo() {
  docker build -q -t "$echo_image" "$e2e_dir/echo" >/dev/null
  k3d image import -c "${context#k3d-}" "$echo_image" >/dev/null
  # The tenant component without its Flux objects: the admission cases need the tenant namespace labels.
  kustomize build "$e2e_dir/overlay/components/tenant" |
    python3 -c '
import sys, yaml
kept = [doc for doc in yaml.safe_load_all(sys.stdin) if doc["kind"] in ("Namespace", "ServiceAccount", "RoleBinding")]
yaml.safe_dump_all(kept, sys.stdout)
' | kubectl apply -f -
  helm upgrade --install echo "$repo_root/deploy/chart/baukit-app" -n "$echo_namespace" --wait --timeout 5m \
    --set fullnameOverride=echo --set product=e2e-echo --set deploymentEnvironment=local \
    --set api.image.repository=localhost/baukit-e2e/echo --set api.image.tag=local \
    --set api.image.pullPolicy=Never --set 'api.command={nginx,-g,daemon off;}' \
    --set migration.enabled=false --set networkPolicy.dns.k3sCompatible=true
}

cleanup() {
  local status=$?
  set +e
  helm uninstall echo -n "$echo_namespace" --wait >/dev/null 2>&1
  kubectl delete namespace "$echo_namespace" --ignore-not-found --wait=true
  flux resume kustomization -n flux-system "$allowlist_kustomization"
  kubectl -n flux-system wait kustomization "$allowlist_kustomization" --for=condition=Ready --timeout=5m
  kubectl -n kyverno get configmap "$allowlist_configmap" -o jsonpath='{.data.registries}' |
    grep -q baukit-e2e && printf 'ERROR: %s still admits localhost/baukit-e2e/\n' "$allowlist_configmap" >&2 && status=1
  [[ -n $state_dir ]] && rm -rf -- "$state_dir"
  exit "$status"
}

run_suites() {
  local suite args=()
  for suite in "${suites[@]}"; do
    args+=(--test-dir "$e2e_dir/tests/$suite")
  done
  BAUKIT_E2E_PROFILE=persistent chainsaw test "${args[@]}" --config "$e2e_dir/chainsaw-config.yaml"
}

main() {
  parse_args "$@"
  baukit_use_tools "$repo_root" chainsaw flux helm k3d kubectl kustomize
  pin_context
  trap cleanup EXIT
  admit_echo_registry
  install_echo
  run_suites
}

main "$@"
