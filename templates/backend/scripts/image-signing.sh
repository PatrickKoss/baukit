# shellcheck shell=bash
# Sourced by release.sh and promote.sh. Signs and verifies images with the product cosign key.
# CI passes the key in COSIGN_PRIVATE_KEY and its password in COSIGN_PASSWORD. Set COSIGN_KEY to a
# key reference (a path or env://NAME) to sign from elsewhere.

signing_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
signing_config="$signing_root/deploy/cosign-signing-config.json"
signing_public_key=${COSIGN_PUBLIC_KEY:-"$signing_root/deploy/cosign.pub"}
signing_key=${COSIGN_KEY:-env://COSIGN_PRIVATE_KEY}
dry_run=${dry_run:-false}

# Prints the command in a dry run, runs it otherwise.
run() {
  if [[ $dry_run == true ]]; then
    printf '+ %s\n' "$*" >&2
    return 0
  fi
  "$@"
}

require_verification_key() {
  [[ -r $signing_public_key ]] || {
    printf 'missing %s; commit the public half of the product cosign key\n' "$signing_public_key" >&2
    return 2
  }
}

require_signing_key() {
  require_verification_key || return
  if [[ $signing_key == env://COSIGN_PRIVATE_KEY && -z ${COSIGN_PRIVATE_KEY:-} ]]; then
    printf 'COSIGN_PRIVATE_KEY is empty; set it from the release environment or set COSIGN_KEY\n' >&2
    return 2
  fi
  [[ -n ${COSIGN_PASSWORD+set} ]] || {
    printf 'COSIGN_PASSWORD is unset\n' >&2
    return 2
  }
}

image_digest() {
  if [[ $dry_run == true ]]; then
    printf 'sha256:%s\n' "$(printf %s "$1" | sha256sum | cut -c1-64)"
    return
  fi
  docker buildx imagetools inspect "$1" --format '{% raw %}{{.Manifest.Digest}}{% endraw %}'
}

# Fails unless the image carries a signature from the product key.
verify_image() {
  run cosign verify --key "$signing_public_key" --insecure-ignore-tlog=true "$1" >/dev/null
}

# Signs <repository>:<tag> by digest and prints the GitOps pin <tag>@<digest>.
sign_image() {
  local image=$1 digest
  digest=$(image_digest "$image")
  [[ $digest =~ ^sha256:[0-9a-f]{64}$ ]] || {
    printf 'could not resolve the digest of %s\n' "$image" >&2
    return 2
  }
  run cosign sign --yes --key "$signing_key" --signing-config "$signing_config" \
    "${image%:*}@${digest}" >&2
  verify_image "${image%:*}@${digest}"
  printf '%s@%s\n' "${image##*:}" "$digest"
}
