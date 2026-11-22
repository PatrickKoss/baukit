#!/usr/bin/env bash
# Builds, pushes and signs the backend images for HEAD, then pins them in deploy/gitops/testing.
# Usage: scripts/release.sh [--dry-run]
# A dry run builds and pushes nothing, invents digests and keeps the pin commit local.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
dry_run=false
case "${1:-}" in
  --dry-run) dry_run=true ;;
  "") ;;
  *) printf 'usage: %s [--dry-run]\n' "$0" >&2; exit 2 ;;
esac
registry={{ context.gitops.registry }}
{% if context.gitops.lab_registry %}lab_registry={{ context.gitops.lab_registry }}
{% endif %}app={{ context.app_name }}
processes=(api migrate{% if context.worker %} worker{% endif %})
release_file=deploy/gitops/testing/release.yaml
# shellcheck source=scripts/image-signing.sh
source "$repo_root/scripts/image-signing.sh"
# shellcheck source=scripts/gitops-git.sh
source "$repo_root/scripts/gitops-git.sh"
trap gitops_cleanup_key EXIT

required=(git python3)
[[ $dry_run == true ]] || required+=(cosign docker)
for command_name in "${required[@]}"; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'required command not found: %s\n' "$command_name" >&2
    exit 2
  }
done
[[ -z $(git -C "$repo_root" status --porcelain) ]] || {
  printf 'release requires a clean working tree\n' >&2
  exit 2
}
if [[ $dry_run == true ]]; then
  printf 'dry run: skipping the signing key and sealed-secret checks\n' >&2
else
  require_signing_key
  python3 "$repo_root/scripts/gitops-pins.py" check-encrypted "$repo_root/deploy/gitops"
fi

tag=$(git -C "$repo_root" rev-parse --short=12 HEAD)
commit=$(git -C "$repo_root" rev-parse HEAD)
if [[ -n ${REGISTRY_TOKEN:-} ]]; then
  printf '%s' "$REGISTRY_TOKEN" | run docker login "${registry%%/*}" \
    -u "${REGISTRY_USER:?REGISTRY_USER must name the registry user}" --password-stdin
fi
{% if context.gitops.lab_registry %}if [[ -n ${LAB_REGISTRY_TOKEN:-} ]]; then
  printf '%s' "$LAB_REGISTRY_TOKEN" | run cosign login "${lab_registry%%/*}" \
    -u "${LAB_REGISTRY_USER:?LAB_REGISTRY_USER must name the lab registry user}" --password-stdin
fi
{% endif %}
pins=()
for process in "${processes[@]}"; do
  image="${registry}/${app}-${process}:${tag}"
  printf 'Building %s\n' "$image" >&2
  run sh "$repo_root/scripts/docker.sh" buildx build \
    --file "$repo_root/backend/Dockerfile" --target "$process" \
    --build-arg BACKEND_CONTEXT=backend --build-arg "GIT_COMMIT=$commit" \
    --tag "$image" --push "$repo_root"
{% if context.gitops.lab_registry %}  pin=$(sign_image "$image")
  # cosign copy keeps the digest, so the signature and the pin stay valid in the lab registry.
  run cosign copy --force "${image%:*}@${pin#*@}" "${lab_registry}/${app}-${process}:${tag}" >&2
  verify_image "${lab_registry}/${app}-${process}@${pin#*@}"
  pins+=("${process}=${pin}")
{% else %}  pins+=("${process}=$(sign_image "$image")")
{% endif %}done

python3 "$repo_root/scripts/gitops-pins.py" set "$repo_root/$release_file" "${pins[@]}"
if git -C "$repo_root" diff --quiet -- "$release_file"; then
  printf 'testing already pins %s; images were republished, no commit needed\n' "$tag"
  exit 0
fi
gitops_commit_and_push "$repo_root" "chore(release): pin ${tag} in testing" "$release_file"
printf 'Released %s to testing.\n' "$tag"
