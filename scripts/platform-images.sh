#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# shellcheck source=deploy/lib/tools.sh
. "$repo_root/deploy/lib/tools.sh"
baukit_use_tools "$repo_root" kustomize kubeconform helm flux kyverno trivy

cache_dir=${BAUKIT_TRIVY_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/baukit/trivy}
work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT

"$repo_root/deploy/platform/validate.sh" --render-to "$work_dir/rendered" >/dev/null
mkdir -p "$cache_dir"
# shellcheck source=deploy/platform/registry-mirrors.sh
source "$repo_root/deploy/platform/registry-mirrors.sh"
# Scans read through the local pull-through caches when they run, see registry-mirrors.sh.
registry_mirrors_trivy_config >"$work_dir/trivy.yaml"
PYTHONDONTWRITEBYTECODE=1 python3 "$repo_root/deploy/platform/lint/platform_images.py" \
  --rendered "$work_dir/rendered" --repo-root "$repo_root" --cache "$cache_dir" \
  --work "$work_dir" --trivy-config "$work_dir/trivy.yaml" --allowlist "$repo_root/deploy/platform/lint/image-findings.toml"
