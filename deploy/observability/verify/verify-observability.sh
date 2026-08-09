#!/bin/sh
set -eu

usage() {
    echo "usage: $0 PRODUCT_ENV_PREFIX" >&2
    echo "reads <PREFIX>_VERIFY_* settings; see deploy/observability/README.md" >&2
    exit 2
}

[ "$#" -eq 1 ] || usage
product_prefix=$1
case "$product_prefix" in
    '' | *[!A-Z0-9_]* | [0-9_]*) usage ;;
esac

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
observability=$(CDPATH= cd -- "$script_dir/.." && pwd)
temporary_dir=$(mktemp -d)

cleanup() {
    status=$?
    trap - EXIT INT TERM
    rm -r -- "$temporary_dir"
    exit "$status"
}
trap cleanup EXIT INT TERM

setting() {
    variable_name="${product_prefix}_VERIFY_$1"
    printenv "$variable_name" 2>/dev/null || true
}

api_public_url=$(setting API_PUBLIC_URL)
api_ops_url=$(setting API_OPS_URL)
worker_ops_url=$(setting WORKER_OPS_URL)
log_files=$(setting LOG_FILES)
forbidden_values_file=$(setting FORBIDDEN_VALUES_FILE)
known_gaps_file=$(setting KNOWN_GAPS_FILE)
prometheus_image=$(setting PROMETHEUS_IMAGE)
wait_attempts=$(setting WAIT_ATTEMPTS)

api_public_url=${api_public_url:-http://127.0.0.1:18080}
api_ops_url=${api_ops_url:-http://127.0.0.1:19090}
prometheus_image=${prometheus_image:-prom/prometheus:v3.13.2}
wait_attempts=${wait_attempts:-60}

[ -n "$log_files" ] || {
    echo "${product_prefix}_VERIFY_LOG_FILES must be a colon-delimited list" >&2
    exit 2
}
[ -n "$forbidden_values_file" ] || {
    echo "${product_prefix}_VERIFY_FORBIDDEN_VALUES_FILE is required" >&2
    exit 2
}
[ -f "$forbidden_values_file" ] || {
    echo "forbidden-values file not found: $forbidden_values_file" >&2
    exit 2
}

wait_for_url() {
    url=$1
    attempts=0
    until curl --fail --silent --show-error "$url" >/dev/null 2>&1; do
        attempts=$((attempts + 1))
        if [ "$attempts" -ge "$wait_attempts" ]; then
            echo "timed out waiting for $url" >&2
            return 1
        fi
        sleep 1
    done
}

python3 "$observability/lint/check-metric-names.py"
if command -v promtool >/dev/null 2>&1; then
    promtool check rules \
        "$observability/recording-rules/baukit-red.rules.yml" \
        "$observability/alerts/baukit.rules.yml"
else
    docker run --rm --entrypoint promtool \
        --volume "$observability:/observability:ro" \
        "$prometheus_image" check rules \
        /observability/recording-rules/baukit-red.rules.yml \
        /observability/alerts/baukit.rules.yml
fi

wait_for_url "${api_ops_url%/}/healthz"
wait_for_url "${api_ops_url%/}/readyz"
if [ -n "$worker_ops_url" ]; then
    wait_for_url "${worker_ops_url%/}/healthz"
    wait_for_url "${worker_ops_url%/}/readyz"
fi

public_metrics_status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
    "${api_public_url%/}/metrics")
if [ "$public_metrics_status" != "404" ]; then
    echo "public /metrics returned $public_metrics_status; expected 404" >&2
    exit 1
fi

curl --fail --silent --show-error "${api_ops_url%/}/metrics" >"$temporary_dir/api.metrics"
if [ -n "$worker_ops_url" ]; then
    curl --fail --silent --show-error "${worker_ops_url%/}/metrics" \
        >"$temporary_dir/worker.metrics"
fi

old_ifs=$IFS
IFS=:
for log_path in $log_files; do
    [ -f "$log_path" ] || {
        echo "runtime log not found: $log_path" >&2
        exit 2
    }
    while IFS= read -r forbidden || [ -n "$forbidden" ]; do
        [ -n "$forbidden" ] || continue
        if grep -F -- "$forbidden" "$log_path" >/dev/null; then
            echo "runtime log $log_path leaked a forbidden value" >&2
            exit 1
        fi
    done <"$forbidden_values_file"
done
IFS=$old_ifs

set -- --observability-root "$observability" --metrics "api=$temporary_dir/api.metrics"
if [ -n "$worker_ops_url" ]; then
    set -- "$@" --metrics "worker=$temporary_dir/worker.metrics"
fi
if [ -n "$known_gaps_file" ]; then
    set -- "$@" --known-gap-file "$known_gaps_file"
fi
python3 "$script_dir/check-observability-metrics.py" "$@"

echo "observability verification passed for environment prefix $product_prefix"
