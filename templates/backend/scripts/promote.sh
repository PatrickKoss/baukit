#!/usr/bin/env bash
# Copies the signed testing pins to deploy/gitops/production after verifying every signature.
# Usage: scripts/promote.sh [--dry-run] [TAG]
# TAG, when given, must be the tag testing pins, so a stale dispatch cannot promote by accident.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
dry_run=false
if [[ ${1:-} == --dry-run ]]; then
  dry_run=true
  shift
fi
expected_tag=${1:-}
registry={{ context.gitops.registry }}
app={{ context.app_name }}
testing_file=deploy/gitops/testing/release.yaml
production_file=deploy/gitops/production/release.yaml
# shellcheck source=scripts/image-signing.sh
source "$repo_root/scripts/image-signing.sh"
# shellcheck source=scripts/gitops-git.sh
source "$repo_root/scripts/gitops-git.sh"
trap gitops_cleanup_key EXIT

required=(git python3)
[[ $dry_run == true ]] || required+=(cosign)
for command_name in "${required[@]}"; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'required command not found: %s\n' "$command_name" >&2
    exit 2
  }
done
[[ -z $(git -C "$repo_root" status --porcelain) ]] || {
  printf 'promotion requires a clean working tree\n' >&2
  exit 2
}
[[ $dry_run == true ]] || {
  require_verification_key
  python3 "$repo_root/scripts/gitops-pins.py" check-encrypted "$repo_root/deploy/gitops"
}

tag=$(python3 "$repo_root/scripts/gitops-pins.py" tag "$repo_root/$testing_file")
[[ -z $expected_tag || $expected_tag == "$tag" ]] || {
  printf 'testing pins %s, not the requested %s\n' "$tag" "$expected_tag" >&2
  exit 2
}
if [[ -n ${REGISTRY_TOKEN:-} ]]; then
  printf '%s' "$REGISTRY_TOKEN" | run cosign login "${registry%%/*}" \
    -u "${REGISTRY_USER:?REGISTRY_USER must name the registry user}" --password-stdin
fi
while IFS='=' read -r process pin; do
  verify_image "${registry}/${app}-${process}@${pin#*@}"
done < <(python3 "$repo_root/scripts/gitops-pins.py" list "$repo_root/$testing_file")

python3 "$repo_root/scripts/gitops-pins.py" copy "$repo_root/$testing_file" "$repo_root/$production_file"
if git -C "$repo_root" diff --quiet -- "$production_file"; then
  printf 'production already pins %s\n' "$tag"
  exit 0
fi
gitops_commit_and_push "$repo_root" "chore(release): promote ${tag} to production" "$production_file"
printf 'Promoted %s to production.\n' "$tag"
