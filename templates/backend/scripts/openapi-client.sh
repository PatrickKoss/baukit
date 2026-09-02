#!/bin/sh
set -eu

schema=$(python3 - <<'PY'
import tomllib

with open("baukit.toml", "rb") as source:
    print(tomllib.load(source)["openapi"]["schema"])
PY
)
consumers=$(python3 - <<'PY'
import tomllib

with open("baukit.toml", "rb") as source:
    openapi = tomllib.load(source)["openapi"]
consumers = openapi.get("consumers")
if consumers is None:
    consumers = [openapi["typescript"]]
for consumer in consumers:
    print(consumer)
PY
)

if [ -z "$consumers" ]; then
  echo "baukit.toml lists no OpenAPI consumers." >&2
  exit 1
fi

for output in $consumers; do
  mkdir -p "$(dirname "$output")"
  if command -v corepack >/dev/null 2>&1; then
    corepack pnpm dlx openapi-typescript@7.13.0 "$schema" -o "$output"
  elif command -v npx >/dev/null 2>&1; then
    npx --yes openapi-typescript@7.13.0 "$schema" -o "$output"
  else
    echo "OpenAPI client generation requires current Node.js LTS with corepack or npx." >&2
    exit 1
  fi
done
