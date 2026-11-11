#!/bin/sh
set -eu

lockfile_run() {
  "$@"
}

extra_lockfiles() {
  :
}

if [ -f scripts/lockfiles.local.sh ]; then
  # shellcheck source=/dev/null
  . ./scripts/lockfiles.local.sh
fi

{% if context.backend %}lockfile_run cargo generate-lockfile --manifest-path backend/Cargo.toml
{% endif %}{% if context.web or context.mobile %}if ! command -v corepack >/dev/null 2>&1; then
  echo "pnpm lockfile generation requires current Node.js LTS with corepack." >&2
  exit 1
fi
{% endif %}{% if context.web %}(
  cd web
  lockfile_run corepack pnpm install --lockfile-only --ignore-scripts
)
{% endif %}{% if context.mobile %}(
  cd mobile
  lockfile_run corepack pnpm install --lockfile-only --ignore-scripts
)
{% endif %}extra_lockfiles
