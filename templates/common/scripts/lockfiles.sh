#!/bin/sh
set -eu

{% if context.backend %}cargo generate-lockfile --manifest-path backend/Cargo.toml
{% endif %}{% if context.web or context.mobile or context.mcp %}if ! command -v corepack >/dev/null 2>&1; then
  echo "pnpm lockfile generation requires current Node.js LTS with corepack." >&2
  exit 1
fi
{% endif %}{% if context.web %}(
  cd web
  corepack pnpm@11.18.0 install --lockfile-only --ignore-scripts
)
{% endif %}{% if context.mobile %}(
  cd mobile
  corepack pnpm@11.18.0 install --lockfile-only --ignore-scripts
)
{% endif %}{% if context.mcp %}(
  cd mcp
  corepack pnpm@11.18.0 install --lockfile-only --ignore-scripts
)
{% endif %}
