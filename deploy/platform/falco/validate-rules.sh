#!/usr/bin/env bash
set -euo pipefail

falco_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$falco_dir/../../.." && pwd)

if (($# == 0)); then
  set -- "$falco_dir"/rules/*.yaml "$falco_dir"/../components/falco-shared-kernel/*.rules.yaml
fi

if ! command -v docker >/dev/null 2>&1; then
  printf 'ERROR: falco rule validation needs docker\n' >&2
  exit 2
fi

image=$(python3 -c '
import sys, tomllib
with open(sys.argv[1], "rb") as manifest:
    print(tomllib.load(manifest)["images"]["falco"])
' "$repo_root/deploy/tools.toml")
image="docker.io/${image%%:*}@${image##*@}"

mounts=()
arguments=(-V /etc/falco/falco_rules.yaml)
index=0
for rules_file in "$@"; do
  absolute=$(cd -- "$(dirname -- "$rules_file")" && pwd)/$(basename -- "$rules_file")
  mounts+=(-v "$absolute:/baukit-rules/$index-$(basename -- "$rules_file"):ro")
  arguments+=(-V "/baukit-rules/$index-$(basename -- "$rules_file")")
  ((index += 1))
done

docker run --rm --network none "${mounts[@]}" "$image" falco "${arguments[@]}"
