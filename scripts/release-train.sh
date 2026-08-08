#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/release-train.sh <patch|minor>" >&2
  exit 2
}

if [[ $# -ne 1 ]]; then
  usage
fi

bump=$1
if [[ "$bump" != "patch" && "$bump" != "minor" ]]; then
  usage
fi

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "release train preparation requires a clean working tree" >&2
  exit 1
fi

command -v pnpm >/dev/null || {
  echo "pnpm is required" >&2
  exit 1
}

scripts/check-version-coherence.py

current=$(python3 -c 'import tomllib; print(tomllib.load(open("rust/Cargo.toml", "rb"))["workspace"]["package"]["version"])')
IFS=. read -r major minor patch <<< "$current"

case "$bump" in
  patch) next="$major.$minor.$((patch + 1))" ;;
  minor) next="$major.$((minor + 1)).0" ;;
esac

if [[ "$major" -ne 0 ]]; then
  echo "baukit remains on the 0.x train until the go-public readiness decision" >&2
  exit 1
fi

train_changeset=typescript/.changeset/release-train.md
if [[ -e "$train_changeset" ]]; then
  echo "$train_changeset already exists; remove or rename it before retrying" >&2
  exit 1
fi

{
  echo '---'
  for package in \
    '@baukit/analytics-core' \
    '@baukit/analytics-posthog-web' \
    '@baukit/analytics-posthog-native' \
    '@baukit/api-runtime' \
    '@baukit/data-contracts' \
    '@baukit/ui-tokens'; do
    printf "'%s': %s\n" "$package" "$bump"
  done
  echo '---'
  echo
  echo "Release the coordinated baukit $next train."
} > "$train_changeset"

(cd typescript && pnpm version-packages)

actual_ts=$(node -p "require('./typescript/packages/analytics-core/package.json').version")
if [[ "$actual_ts" != "$next" ]]; then
  echo "Changesets selected TypeScript version $actual_ts, expected $next for a $bump train" >&2
  echo "Review pending changesets; a higher bump may already be requested." >&2
  exit 1
fi

TRAIN_VERSION="$next" perl -0pi -e \
  's{(\[workspace\.package\]\nversion = ")[^"]+(")}{$1$ENV{TRAIN_VERSION}$2}' \
  rust/Cargo.toml
TRAIN_VERSION="$next" perl -0pi -e \
  's{^(baukit-[a-z-]+ = \{ version = ")=[^"]+(".*)$}{$1."=".$ENV{TRAIN_VERSION}.$2}egm' \
  rust/Cargo.toml
cargo update --manifest-path rust/Cargo.toml --workspace

release_date=${RELEASE_DATE:-$(date -u +%F)}
for changelog in rust/crates/*/CHANGELOG.md; do
  TRAIN_VERSION="$next" RELEASE_DATE="$release_date" perl -0pi -e \
    's{## \[Unreleased\]\n\n}{"## [Unreleased]\n\n## [".$ENV{TRAIN_VERSION}."] - ".$ENV{RELEASE_DATE}."\n\n"}e' \
    "$changelog"
done

# The template manifest and generated baukit.toml files use the bare semantic
# version. The corresponding immutable source version is baukit-vX.Y.Z.
printf '%s\n' "$next" > templates/VERSION

scripts/check-version-coherence.py

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'version=%s\n' "$next" >> "$GITHUB_OUTPUT"
fi
printf 'Prepared baukit-v%s. Review changelogs and the compatibility matrix before committing.\n' "$next"
