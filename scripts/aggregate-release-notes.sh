#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/aggregate-release-notes.sh <X.Y.Z>" >&2
  exit 2
fi

version=$1
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

echo "# baukit v$version"
echo
echo "One release train for the Rust crates, TypeScript packages, and templates."

print_sections() {
  local heading=$1
  shift
  local printed_heading=false
  local file name section section_and_marker

  for file in "$@"; do
    [[ -f "$file" ]] || continue
    section_and_marker=$(awk -v version="$version" '
      /^## / {
        if (capture) exit
        if (index($0, "[" version "]") > 0 || $0 == "## " version) {
          print "__BAUKIT_SECTION__"
          capture = 1
          next
        }
      }
      capture { print }
    ' "$file")
    [[ "$section_and_marker" == __BAUKIT_SECTION__* ]] || continue
    section=${section_and_marker#__BAUKIT_SECTION__}
    while [[ "$section" == $'\n'* ]]; do
      section=${section#$'\n'}
    done
    if [[ -z "$section" ]]; then
      section='- No component-specific changes in this train.'
    fi
    if [[ "$printed_heading" == false ]]; then
      echo
      echo "## $heading"
      printed_heading=true
    fi
    name=$(basename "$(dirname "$file")")
    echo
    echo "### $name"
    echo
    printf '%s\n' "$section"
  done
}

print_sections "Rust crates" rust/crates/*/CHANGELOG.md
print_sections "TypeScript packages" typescript/packages/*/CHANGELOG.md

current_tag="baukit-v$version"
previous_tag=$(git tag --list 'baukit-v*' --sort=-v:refname | grep -Fvx "$current_tag" | head -n 1 || true)
echo
echo "## Commits"
echo
if [[ -n "$previous_tag" ]]; then
  git log --pretty='- %s (`%h`)' "$previous_tag..$current_tag"
else
  git log --pretty='- %s (`%h`)' "$current_tag"
fi
