#!/bin/sh
set -eu

observability=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if command -v promtool >/dev/null 2>&1; then
    promtool check rules "$observability/recording-rules/baukit-red.rules.yml" "$observability/alerts/baukit.rules.yml"
    promtool test rules "$observability/tests/worker.rules.test.yml"
else
    docker run --rm --entrypoint promtool --volume "$observability:/observability:ro" \
        "${PROMETHEUS_IMAGE:-prom/prometheus:v3.13.2}" check rules \
        /observability/recording-rules/baukit-red.rules.yml /observability/alerts/baukit.rules.yml
    docker run --rm --entrypoint promtool --volume "$observability:/observability:ro" \
        "${PROMETHEUS_IMAGE:-prom/prometheus:v3.13.2}" test rules /observability/tests/worker.rules.test.yml
fi
