#!/bin/sh
# Runs the previous release's store tests against a database that HEAD's migrations built, so a
# migration that breaks the binaries still running during a rollout fails CI (F-58, G-06).
#
# The tests to run are `[old_binary] tests` in rust/migrations.toml, as `crate:test-target` pairs.
# They take the database from DATABASE_URL, so only releases whose baukit-test honours it can be
# listed. With no tests listed the step reports that and passes.
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

tests=$(python3 -c '
import tomllib
data = tomllib.load(open("rust/migrations.toml", "rb"))
print(" ".join(data.get("old_binary", {}).get("tests", [])))
')
if [ -z "$tests" ]; then
  echo "migrations-old-binary: no old-binary tests listed in rust/migrations.toml; skipping"
  exit 0
fi

previous=$(git describe --tags --abbrev=0 --match 'v*.*.*' HEAD^ 2>/dev/null || true)
if [ -z "$previous" ]; then
  echo "migrations-old-binary: no earlier release tag; skipping"
  exit 0
fi

scratch=$(mktemp -d)
worktree=$scratch/release
fifo=$scratch/stdin
mkfifo "$fifo"
cleanup() {
  exec 3>&- 2>/dev/null || true
  git worktree remove --force "$worktree" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT INT TERM

git worktree add --detach "$worktree" "$previous" >/dev/null
python3 scripts/check-migrations.py serve <"$fifo" >"$scratch/url" &
exec 3>"$fifo"
while [ ! -s "$scratch/url" ]; do
  kill -0 "$!" 2>/dev/null || { echo "migrations-old-binary: migrating HEAD failed" >&2; exit 1; }
  sleep 1
done
DATABASE_URL=$(head -n 1 "$scratch/url")
export DATABASE_URL

echo "migrations-old-binary: $previous store tests against HEAD's schema"
for entry in $tests; do
  crate=${entry%%:*}
  target=${entry#*:}
  cargo test --manifest-path "$worktree/rust/Cargo.toml" -p "$crate" --test "$target" -- --include-ignored
done
