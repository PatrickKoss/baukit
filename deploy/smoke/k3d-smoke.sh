#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
deploy_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
chart=${BAUKIT_SMOKE_CHART:-$deploy_root/chart/baukit-app}
observability_chart=${BAUKIT_SMOKE_OBSERVABILITY_CHART:-$deploy_root/observability}

product=${BAUKIT_SMOKE_PRODUCT:?BAUKIT_SMOKE_PRODUCT is required}
values_file=${BAUKIT_SMOKE_VALUES_FILE:?BAUKIT_SMOKE_VALUES_FILE is required}
images=${BAUKIT_SMOKE_IMAGES:?BAUKIT_SMOKE_IMAGES is required}
token_command=${BAUKIT_SMOKE_TOKEN_COMMAND:?BAUKIT_SMOKE_TOKEN_COMMAND is required}

cluster_name=${BAUKIT_SMOKE_CLUSTER:-$product-baukit-smoke}
namespace=${BAUKIT_SMOKE_NAMESPACE:-$product}
release=${BAUKIT_SMOKE_RELEASE:-$product}
resource_name=${BAUKIT_SMOKE_RESOURCE_NAME:-$release}
observability_namespace=${BAUKIT_SMOKE_OBSERVABILITY_NAMESPACE:-observability}
runbook_base_url=${BAUKIT_SMOKE_RUNBOOK_BASE_URL:-https://github.com/PatrickKoss/baukit/blob/main/docs/runbooks/app}
dependencies_file=${BAUKIT_SMOKE_DEPENDENCIES_FILE:-}
dependencies_namespace=${BAUKIT_SMOKE_DEPENDENCIES_NAMESPACE:-$namespace-deps}
dependencies_pod_security_level=${BAUKIT_SMOKE_DEPENDENCIES_POD_SECURITY_LEVEL:-baseline}
dependency_deployments=${BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS:-}
secret_manifests=${BAUKIT_SMOKE_SECRET_MANIFESTS:-}
worker_enabled=${BAUKIT_SMOKE_WORKER_ENABLED:-false}
k3s_dns_enabled=${BAUKIT_SMOKE_K3S_DNS_ENABLED:-true}
k3d_ports=${BAUKIT_SMOKE_K3D_PORTS:-127.0.0.1:8081:30081@server:0}
k3d_api_port=${BAUKIT_SMOKE_K3D_API_PORT:-}
pod_security_level=${BAUKIT_SMOKE_POD_SECURITY_LEVEL:-restricted}
policy_dir=${BAUKIT_SMOKE_POLICY_DIR:-}
api_check_path=${BAUKIT_SMOKE_API_CHECK_PATH:-/me}
api_check_status=${BAUKIT_SMOKE_API_CHECK_STATUS:-200}
known_gaps_file=${BAUKIT_SMOKE_KNOWN_GAPS_FILE:-}
extra_forbidden_file=${BAUKIT_SMOKE_FORBIDDEN_VALUES_FILE:-}
product_check=${BAUKIT_SMOKE_PRODUCT_CHECK:-}
prometheus_image=${BAUKIT_SMOKE_PROMETHEUS_IMAGE:-prom/prometheus:v3.13.2}

temporary_dir=$(mktemp -d)
forward_dir=$temporary_dir/port-forward
KUBECONFIG=$temporary_dir/kubeconfig
export KUBECONFIG
forbidden_file=$temporary_dir/forbidden-values
created_cluster=false
api_forward_pid=""
api_ops_forward_pid=""
worker_ops_forward_pid=""

cleanup() {
    status=$?
    trap - EXIT INT TERM
    for pid in "$worker_ops_forward_pid" "$api_ops_forward_pid" "$api_forward_pid"; do
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    if [ "$created_cluster" = true ]; then
        k3d cluster delete "$cluster_name" >/dev/null 2>&1 || true
    fi
    rm -r -- "$temporary_dir"
    exit "$status"
}
trap cleanup EXIT INT TERM

for command_name in curl docker helm k3d kubectl python3; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "required command not found: $command_name" >&2
        exit 2
    }
done
[ -f "$chart/Chart.yaml" ] || {
    echo "Baukit application chart not found: $chart" >&2
    exit 2
}
[ -f "$observability_chart/Chart.yaml" ] || {
    echo "Baukit observability chart not found: $observability_chart" >&2
    exit 2
}
[ -f "$values_file" ] || {
    echo "application values file not found: $values_file" >&2
    exit 2
}
if k3d cluster list --no-headers 2>/dev/null | awk '{print $1}' | grep -qx "$cluster_name"; then
    echo "refusing to reuse existing cluster $cluster_name" >&2
    exit 1
fi
for image in $images; do
    docker image inspect "$image" >/dev/null 2>&1 || {
        echo "image is not present locally: $image" >&2
        exit 2
    }
done

if [ -z "$k3d_api_port" ]; then
    k3d_api_port=127.0.0.1:$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
fi
set -- cluster create "$cluster_name" \
    --servers 1 \
    --agents 0 \
    --api-port "$k3d_api_port" \
    --kubeconfig-update-default=false \
    --kubeconfig-switch-context=false \
    --k3s-arg '--disable=traefik@server:0' \
    --wait
for port_mapping in $k3d_ports; do
    set -- "$@" --port "$port_mapping"
done
created_cluster=true
k3d "$@"
k3d kubeconfig get "$cluster_name" >"$KUBECONFIG"
# Image references cannot contain whitespace, so the list splits on spaces.
set -- image import --cluster "$cluster_name"
for image in $images; do
    set -- "$@" "$image"
done
k3d "$@"

install_admission_policies() {
    [ -n "$policy_dir" ] || return 0
    [ -d "$policy_dir" ] || {
        echo "admission policy directory not found: $policy_dir" >&2
        exit 2
    }
    echo "installing admission policies from $policy_dir"
    kubectl apply --recursive -f "$policy_dir"
}

create_namespace() {
    target=$1
    level=$2
    kubectl create namespace "$target" --dry-run=client -o yaml |
        kubectl label --local -f - -o yaml \
            "pod-security.kubernetes.io/enforce=$level" \
            pod-security.kubernetes.io/enforce-version=latest \
            "pod-security.kubernetes.io/audit=$level" \
            pod-security.kubernetes.io/audit-version=latest \
            "pod-security.kubernetes.io/warn=$level" \
            pod-security.kubernetes.io/warn-version=latest |
        kubectl create -f -
}

install_admission_policies
create_namespace "$namespace" "$pod_security_level"
create_namespace "$dependencies_namespace" "$dependencies_pod_security_level"
if [ "$observability_namespace" != "$namespace" ]; then
    kubectl create namespace "$observability_namespace"
fi

if [ -n "$secret_manifests" ]; then
    old_ifs=$IFS
    IFS=:
    for manifest in $secret_manifests; do
        kubectl -n "$namespace" apply -f "$manifest"
    done
    IFS=$old_ifs
fi
if [ -n "$dependencies_file" ]; then
    kubectl -n "$dependencies_namespace" apply -f "$dependencies_file"
fi
for deployment in $dependency_deployments; do
    kubectl -n "$dependencies_namespace" rollout status "deployment/$deployment" --timeout=240s
done


echo "installing $product with the Baukit application chart"
set -- upgrade --install "$release" "$chart" \
    --namespace "$namespace" \
    --values "$values_file" \
    --wait \
    --timeout 5m
if [ "$k3s_dns_enabled" = true ]; then
    set -- "$@" --set networkPolicy.dns.k3sCompatible=true
fi
if ! helm "$@"; then
    kubectl -n "$namespace" get pods,jobs >&2 || true
    kubectl -n "$namespace" logs -l baukit.dev/process=migrate \
        --all-containers --tail=100 >&2 || true
    kubectl -n "$namespace" logs -l baukit.dev/process=api \
        --all-containers --tail=100 >&2 || true
    exit 1
fi
kubectl -n "$namespace" rollout status "deployment/$resource_name-api" --timeout=180s
if [ "$worker_enabled" = true ]; then
    kubectl -n "$namespace" rollout status "deployment/$resource_name-worker" --timeout=180s
fi

echo "installing the Baukit observability ConfigMaps"
helm upgrade --install "$release-observability" "$observability_chart" \
    --namespace "$observability_namespace" \
    --set fullnameOverride=baukit-observability-pack \
    --set runbookBaseUrl="$runbook_base_url" \
    --wait
for suffix in dashboard recording-rules alert-rules; do
    kubectl -n "$observability_namespace" get configmap \
        "baukit-observability-pack-$suffix" >/dev/null
done
kubectl -n "$observability_namespace" get configmap baukit-observability-pack-dashboard \
    -o jsonpath='{.metadata.labels.grafana_dashboard}' | grep -qx 1

mkdir -p "$forward_dir"
kubectl -n "$namespace" port-forward "service/$resource_name" 18080:80 \
    >"$forward_dir/api.log" 2>&1 &
api_forward_pid=$!
kubectl -n "$namespace" port-forward "service/$resource_name-ops" 19090:9090 \
    >"$forward_dir/api-ops.log" 2>&1 &
api_ops_forward_pid=$!
if [ "$worker_enabled" = true ]; then
    kubectl -n "$namespace" port-forward "service/$resource_name-worker-ops" 19091:9090 \
        >"$forward_dir/worker-ops.log" 2>&1 &
    worker_ops_forward_pid=$!
fi

wait_for_url() {
    url=$1
    attempt=0
    until curl --fail --silent "$url" >/dev/null 2>&1; do
        attempt=$((attempt + 1))
        if [ "$attempt" -ge 60 ]; then
            echo "timed out waiting for $url" >&2
            cat "$forward_dir"/*.log >&2 || true
            exit 1
        fi
        sleep 1
    done
}
wait_for_url http://127.0.0.1:19090/readyz
if [ "$worker_enabled" = true ]; then
    wait_for_url http://127.0.0.1:19091/readyz
fi

echo "requesting an access token from the product token command"
[ -x "$token_command" ] || {
    echo "product token command is not executable: $token_command" >&2
    exit 2
}
token_file=$temporary_dir/access-token
(umask 077 && : >"$token_file")
BAUKIT_SMOKE_ACCESS_TOKEN_FILE=$token_file "$token_command"
[ -s "$token_file" ] || {
    echo "product token command wrote no access token" >&2
    exit 1
}
check_status=$(
    printf 'Authorization: Bearer %s\n' "$(head -n 1 "$token_file")" |
        curl --silent --output /dev/null --write-out '%{http_code}' \
            --header @- "http://127.0.0.1:18080$api_check_path"
)
[ "$check_status" = "$api_check_status" ] || {
    echo "GET $api_check_path returned $check_status, expected $api_check_status" >&2
    exit 1
}
if [ -n "$product_check" ]; then
    [ -x "$product_check" ] || {
        echo "product smoke check is not executable: $product_check" >&2
        exit 2
    }
    BAUKIT_SMOKE_API_URL=http://127.0.0.1:18080 \
    BAUKIT_SMOKE_ACCESS_TOKEN_FILE=$token_file \
        "$product_check"
fi

kubectl -n "$namespace" logs "deployment/$resource_name-api" --all-containers \
    >"$temporary_dir/api-runtime.log"
runtime_logs=$temporary_dir/api-runtime.log
if [ "$worker_enabled" = true ]; then
    kubectl -n "$namespace" logs "deployment/$resource_name-worker" --all-containers \
        >"$temporary_dir/worker-runtime.log"
    runtime_logs=$runtime_logs:$temporary_dir/worker-runtime.log
fi
: >"$forbidden_file"
while IFS= read -r access_token || [ -n "$access_token" ]; do
    [ -n "$access_token" ] && printf '%s\n' "$access_token" >>"$forbidden_file"
done <"$token_file"
if [ -n "$extra_forbidden_file" ]; then
    [ -f "$extra_forbidden_file" ] || {
        echo "forbidden-values file not found: $extra_forbidden_file" >&2
        exit 2
    }
    while IFS= read -r forbidden || [ -n "$forbidden" ]; do
        [ -n "$forbidden" ] && printf '%s\n' "$forbidden" >>"$forbidden_file"
    done <"$extra_forbidden_file"
fi

worker_ops_url=""
if [ "$worker_enabled" = true ]; then
    worker_ops_url=http://127.0.0.1:19091
fi
env \
    BAUKIT_SMOKE_VERIFY_API_PUBLIC_URL=http://127.0.0.1:18080 \
    BAUKIT_SMOKE_VERIFY_API_OPS_URL=http://127.0.0.1:19090 \
    BAUKIT_SMOKE_VERIFY_WORKER_OPS_URL="$worker_ops_url" \
    BAUKIT_SMOKE_VERIFY_LOG_FILES="$runtime_logs" \
    BAUKIT_SMOKE_VERIFY_FORBIDDEN_VALUES_FILE="$forbidden_file" \
    BAUKIT_SMOKE_VERIFY_KNOWN_GAPS_FILE="$known_gaps_file" \
    BAUKIT_SMOKE_VERIFY_PROMETHEUS_IMAGE="$prometheus_image" \
    "$observability_chart/verify/verify-observability.sh" BAUKIT_SMOKE

kubectl -n "$namespace" get pods,services,networkpolicies
kubectl get namespace "$namespace" \
    -o jsonpath='{.metadata.labels.pod-security\.kubernetes\.io/enforce}' |
    grep -qx "$pod_security_level"
if kubectl -n "$namespace" get events -o jsonpath='{range .items[*]}{.message}{"\n"}{end}' |
    grep -i 'podsecurity\|violates'; then
    echo "namespace $namespace recorded Pod Security violations" >&2
    exit 1
fi
echo "k3d smoke passed for $product; teardown follows"
