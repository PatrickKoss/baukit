#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
filters=()
while read -r package; do
    [[ -z "$package" || "$package" == \#* ]] && continue
    filters+=(--filter "$package...")
done <"$repo_root/scripts/fixture-ts-packages.txt"
cd "$repo_root/typescript"
corepack pnpm "${filters[@]}" run build
