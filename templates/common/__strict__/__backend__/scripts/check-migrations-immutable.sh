#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
base_revision=${1:-}
head_revision=${2:-HEAD}

if [ -z "$base_revision" ]; then
  echo "usage: $0 <base-revision> [head-revision]" >&2
  exit 2
fi

for revision in "$base_revision" "$head_revision"; do
  if ! git -C "$repository_root" rev-parse --verify --quiet "${revision}^{commit}" >/dev/null; then
    echo "migration immutability check: unknown revision $revision" >&2
    exit 2
  fi
done

changed_migrations=$(
  git -C "$repository_root" diff \
    --no-renames \
    --name-only \
    --diff-filter=MDT \
    "$base_revision" \
    "$head_revision" \
    -- backend/migrations
)

if [ -n "$changed_migrations" ]; then
  echo "Applied migration files must not be modified, deleted, or renamed:" >&2
  printf '%s\n' "$changed_migrations" >&2
  echo "Add a new forward migration instead." >&2
  exit 1
fi

echo "migration immutability check: existing migration files are unchanged"
