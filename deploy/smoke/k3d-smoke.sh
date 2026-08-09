#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
deploy_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
chart=${BAUKIT_SMOKE_CHART:-$deploy_root/chart/baukit-app}
observability_chart=${BAUKIT_SMOKE_OBSERVABILITY_CHART:-$deploy_root/observability}

product=${BAUKIT_SMOKE_PRODUCT:?BAUKIT_SMOKE_PRODUCT is required}
values_file=${BAUKIT_SMOKE_VALUES_FILE:?BAUKIT_SMOKE_VALUES_FILE is required}
images=${BAUKIT_SMOKE_IMAGES:?BAUKIT_SMOKE_IMAGES is required}
issuer=${BAUKIT_SMOKE_ISSUER:?BAUKIT_SMOKE_ISSUER is required}
client_id=${BAUKIT_SMOKE_OIDC_CLIENT_ID:?BAUKIT_SMOKE_OIDC_CLIENT_ID is required}
username=${BAUKIT_SMOKE_OIDC_USERNAME:?BAUKIT_SMOKE_OIDC_USERNAME is required}
oidc_password=${BAUKIT_SMOKE_OIDC_PASSWORD:?BAUKIT_SMOKE_OIDC_PASSWORD is required}

cluster_name=${BAUKIT_SMOKE_CLUSTER:-$product-baukit-smoke}
namespace=${BAUKIT_SMOKE_NAMESPACE:-$product}
release=${BAUKIT_SMOKE_RELEASE:-$product}
resource_name=${BAUKIT_SMOKE_RESOURCE_NAME:-$release}
observability_namespace=${BAUKIT_SMOKE_OBSERVABILITY_NAMESPACE:-observability}
dependencies_file=${BAUKIT_SMOKE_DEPENDENCIES_FILE:-}
dependency_deployments=${BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS:-}
secret_manifests=${BAUKIT_SMOKE_SECRET_MANIFESTS:-}
worker_enabled=${BAUKIT_SMOKE_WORKER_ENABLED:-false}
k3s_dns_enabled=${BAUKIT_SMOKE_K3S_DNS_ENABLED:-true}
k3d_ports=${BAUKIT_SMOKE_K3D_PORTS:-8081:30081@server:0}
resolve_host=${BAUKIT_SMOKE_RESOLVE_HOST:-host.k3d.internal=127.0.0.1}
api_check_path=${BAUKIT_SMOKE_API_CHECK_PATH:-/me}
api_check_status=${BAUKIT_SMOKE_API_CHECK_STATUS:-200}
known_gaps_file=${BAUKIT_SMOKE_KNOWN_GAPS_FILE:-}
extra_forbidden_file=${BAUKIT_SMOKE_FORBIDDEN_VALUES_FILE:-}
product_check=${BAUKIT_SMOKE_PRODUCT_CHECK:-}
prometheus_image=${BAUKIT_SMOKE_PROMETHEUS_IMAGE:-prom/prometheus:v3.13.2}

temporary_dir=$(mktemp -d)
forward_dir=$temporary_dir/port-forward
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

set -- cluster create "$cluster_name" \
    --servers 1 \
    --agents 0 \
    --k3s-arg '--disable=traefik@server:0' \
    --wait
for port_mapping in $k3d_ports; do
    set -- "$@" --port "$port_mapping"
done
created_cluster=true
k3d "$@"
# Image references cannot contain whitespace, so intentional shell splitting is
# the simplest portable interface for this disposable local harness.
k3d image import --cluster "$cluster_name" $images

kubectl create namespace "$namespace"
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
    kubectl -n "$namespace" apply -f "$dependencies_file"
fi
for deployment in $dependency_deployments; do
    kubectl -n "$namespace" rollout status "deployment/$deployment" --timeout=240s
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

echo "exercising a headless OIDC authorization-code + PKCE login"
token_file=$temporary_dir/access-token
BAUKIT_HEADLESS_PASSWORD=$oidc_password \
python3 "$script_dir/headless-pkce-login.py" \
    --issuer "$issuer" \
    --client-id "$client_id" \
    --username "$username" \
    --password-env BAUKIT_HEADLESS_PASSWORD \
    --resolve-host "$resolve_host" \
    --token-file "$token_file" \
    --check-url "http://127.0.0.1:18080$api_check_path" \
    --check-status "$api_check_status"
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
printf '%s\n' "$oidc_password" >"$forbidden_file"
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
echo "k3d smoke passed for $product; teardown follows"
