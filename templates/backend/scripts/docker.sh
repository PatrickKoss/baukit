#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
if [ -z "${CARGO_TARGET_CACHE_PREFIX:-}" ]; then
  if command -v sha256sum >/dev/null 2>&1; then
    checkout_hash=$(printf %s "$repository_root" | sha256sum)
  else
    checkout_hash=$(printf %s "$repository_root" | shasum -a 256)
  fi
  checkout_hash=$(printf %.12s "$checkout_hash")
  CARGO_TARGET_CACHE_PREFIX="{{ context.app_name }}-$checkout_hash"
fi
export CARGO_TARGET_CACHE_PREFIX

case "${1:-}" in
  build)
    shift
    set -- build --build-arg "CARGO_TARGET_CACHE_PREFIX=$CARGO_TARGET_CACHE_PREFIX" "$@"
    ;;
  buildx)
    if [ "${2:-}" = build ]; then
      shift 2
      set -- buildx build --build-arg "CARGO_TARGET_CACHE_PREFIX=$CARGO_TARGET_CACHE_PREFIX" "$@"
    fi
    ;;
esac
exec docker "$@"
