#!/usr/bin/env bash
# Runs deploy/smoke/api-load.js with the k6 image pinned in deploy/tools.toml, on the host network so
# it reaches an API on 127.0.0.1, then checks the admitted p99 with admitted_latency.py. Exits with
# k6's status when k6 fails, else with the p99 check's: 99 when a threshold fails.
set -euo pipefail

smoke_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$smoke_dir/../.." && pwd)
settings=(API_URL OPS_URL SMOKE_PATH SMOKE_TOKEN PEAK_RPS RAMP_SECONDS HOLD_SECONDS MAX_VUS)
# The 1 s queue timeout baukit-http sheds after, plus 500 ms for the handler.
admitted_p99_ms=${ADMITTED_P99_MS:-1500}

image=$(python3 -c '
import sys, tomllib
with open(sys.argv[1], "rb") as manifest:
    print(tomllib.load(manifest)["images"]["k6"])
' "$repo_root/deploy/tools.toml")
image="docker.io/${image%%:*}@${image##*@}"

command -v docker >/dev/null 2>&1 || {
  printf 'ERROR: the load smoke runs k6 in Docker; docker is not on PATH\n' >&2
  exit 2
}

env_args=()
for name in "${settings[@]}"; do
  [[ -n ${!name:-} ]] && env_args+=(-e "$name")
done

out_dir=$(mktemp -d)
trap 'rm -rf "$out_dir"' EXIT

status=0
docker run --rm --network host --user "$(id -u):$(id -g)" -v "$smoke_dir:/smoke:ro" -v "$out_dir:/out" \
  -e K6_CSV_TIME_FORMAT=unix_milli "${env_args[@]}" "$image" \
  run --quiet --out csv=/out/samples.csv.gz /smoke/api-load.js "$@" || status=$?
((status == 0)) || exit "$status"
python3 "$smoke_dir/admitted_latency.py" "$out_dir/samples.csv.gz" --p99-ms "$admitted_p99_ms"
