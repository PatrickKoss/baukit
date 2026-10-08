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

command -v corepack >/dev/null || {
  echo "corepack is required" >&2
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

packages=$(python3 scripts/release_packages.py)
{
  echo '---'
  while IFS= read -r package; do
    printf "'%s': %s\n" "$package" "$bump"
  done <<< "$packages"
  echo '---'
  echo
  echo "Release the coordinated baukit $next train."
} > "$train_changeset"

(cd typescript && corepack pnpm version-packages)

actual_ts=$(node -p "require('./typescript/packages/analytics-core/package.json').version")
if [[ "$actual_ts" != "$next" ]]; then
  echo "Changesets selected TypeScript version $actual_ts; normalizing the private 0.x train to $next"
  for manifest in typescript/packages/*/package.json; do
    TRAIN_VERSION="$next" perl -0pi -e \
      's{("version": ")[^"]+}{$1.$ENV{TRAIN_VERSION}}e; s{("\@baukit/[^"]+": "\^)[^"]+}{$1.$ENV{TRAIN_VERSION}}eg' \
      "$manifest"
  done
  for changelog in typescript/packages/*/CHANGELOG.md; do
    ACTUAL_TS="$actual_ts" TRAIN_VERSION="$next" perl -0pi -e \
      '$actual = quotemeta($ENV{ACTUAL_TS}); s{^## $actual$}{"## ".$ENV{TRAIN_VERSION}}egm; s{(\@baukit/[a-z-]+\@)$actual}{$1.$ENV{TRAIN_VERSION}}eg' \
      "$changelog"
  done
fi

python3 scripts/release_packages.py --cut-changelogs "$next"
python3 scripts/check-example-lockfiles.py --refresh

TRAIN_VERSION="$next" perl -0pi -e \
  's{(\[workspace\.package\]\nversion = ")[^"]+(")}{$1$ENV{TRAIN_VERSION}$2}' \
  rust/Cargo.toml
TRAIN_VERSION="$next" perl -0pi -e \
  's{^(baukit-[a-z-]+ = \{ version = ")=[^"]+(".*)$}{$1."=".$ENV{TRAIN_VERSION}.$2}egm' \
  rust/Cargo.toml
cargo update --manifest-path rust/Cargo.toml --workspace

TRAIN_VERSION="$next" perl -0pi -e \
  's{(\[package\]\nname = "baukit-cli"\nversion = ")[^"]+(")}{$1$ENV{TRAIN_VERSION}$2}' \
  cli/Cargo.toml
cargo update --manifest-path cli/Cargo.toml --workspace

release_date=${RELEASE_DATE:-$(date -u +%F)}
mapfile -t template_changelogs < <(python3 - <<'PYTHON'
from pathlib import Path
for path in sorted(Path("templates").rglob("CHANGELOG.md")):
    print(path)
PYTHON
)
for changelog in rust/crates/*/CHANGELOG.md cli/CHANGELOG.md "${template_changelogs[@]}" deploy/chart/baukit-app/CHANGELOG.md; do
  TRAIN_VERSION="$next" RELEASE_DATE="$release_date" perl -0pi -e \
    's{(## (?:\[Unreleased\]|Unreleased)\n\n)}{$1."## [".$ENV{TRAIN_VERSION}."] - ".$ENV{RELEASE_DATE}."\n\n"}e' \
    "$changelog"
done

python3 - "$next" <<'PYTHON'
from pathlib import Path
import re
import sys

changelogs = [*Path("rust/crates").glob("*/CHANGELOG.md"),
              Path("cli/CHANGELOG.md"), *Path("templates").rglob("CHANGELOG.md"),
              *Path("typescript/packages").glob("*/CHANGELOG.md"),
              Path("deploy/chart/baukit-app/CHANGELOG.md")]
for changelog in changelogs:
    heading = f"{sys.argv[1]}\n" if changelog.parts[0] == "typescript" else f"[{sys.argv[1]}] - "
    sections = re.split(r"^## ", changelog.read_text(), flags=re.MULTILINE)
    unreleased = [section for section in sections if section.startswith(("Unreleased\n", "[Unreleased]\n"))]
    if len(unreleased) != 1 or unreleased[0].split("\n", 1)[1].strip() or not any(
        section.startswith(heading) for section in sections
    ):
        sys.exit(f"{changelog} still has uncut Unreleased entries or is missing the new release heading")
PYTHON

# The template manifest and generated baukit.toml files use the bare semantic
# version. The corresponding immutable source version is vX.Y.Z.
printf '%s\n' "$next" > templates/VERSION

for chart in deploy/chart/baukit-app/Chart.yaml deploy/observability/Chart.yaml; do
  TRAIN_VERSION="$next" perl -0pi -e \
    's{^version: .+$}{"version: ".$ENV{TRAIN_VERSION}}egm; s{^appVersion: .+$}{"appVersion: \"".$ENV{TRAIN_VERSION}."\""}egm' \
    "$chart"
done
TRAIN_VERSION="$next" perl -0pi -e \
  's{(^  - name: baukit-app\n    version: ).+$}{$1.$ENV{TRAIN_VERSION}}egm' \
  deploy/chart/baukit-app/README.md

python3 scripts/cli_install.py --update "$next"

scripts/check-version-coherence.py

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'version=%s\n' "$next" >> "$GITHUB_OUTPUT"
fi
printf 'Prepared v%s. Review changelogs and the compatibility matrix before committing.\n' "$next"
