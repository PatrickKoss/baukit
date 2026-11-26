#!/usr/bin/env bash
# Runs deploy/smoke/api-load.js with the k6 image pinned in deploy/tools.toml, on the host network so
# it reaches an API on 127.0.0.1. Exits with k6's status: 99 when a threshold fails.
set -euo pipefail

smoke_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$smoke_dir/../.." && pwd)
settings=(API_URL OPS_URL SMOKE_PATH SMOKE_TOKEN PEAK_RPS RAMP_SECONDS HOLD_SECONDS MAX_VUS ADMITTED_P99_MS)

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

exec docker run --rm --network host -v "$smoke_dir:/smoke:ro" "${env_args[@]}" "$image" \
  run --quiet /smoke/api-load.js "$@"
