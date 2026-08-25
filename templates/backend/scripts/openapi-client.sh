#!/bin/sh
set -eu

schema=backend/openapi.json
output=generated/openapi.d.ts
mkdir -p generated

if command -v corepack >/dev/null 2>&1; then
  exec corepack pnpm dlx openapi-typescript "$schema" -o "$output"
fi
if command -v npx >/dev/null 2>&1; then
  exec npx --yes openapi-typescript "$schema" -o "$output"
fi

echo "OpenAPI client generation requires current Node.js LTS with corepack or npx." >&2
exit 1
