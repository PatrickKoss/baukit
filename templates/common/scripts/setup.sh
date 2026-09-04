#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

found_example=false
for example in "$repository_root/.env.example" "$repository_root/web/.env.example" "$repository_root/mobile/.env.example"; do
  if [ ! -f "$example" ]; then
    continue
  fi
  found_example=true
  env_file=${example%.example}
  python3 "$repository_root/scripts/reconcile-env.py" "$example" "$env_file"
done

if [ "$found_example" = "false" ]; then
  echo "setup: no .env.example files found"
else
  echo "setup: local environment files are current"
fi
