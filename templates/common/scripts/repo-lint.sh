#!/bin/sh
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$repository_root"

usage() {
  echo "usage: sh scripts/repo-lint.sh workflow-lint|secret-scan|osv|deny|python-lint|shell-lint" >&2
  exit 2
}

in_git_repository() {
  [ "$(git rev-parse --show-toplevel 2>/dev/null)" = "$repository_root" ]
}

# Prints NUL-separated repository files matching the patterns, tracked or not, without ignored files.
repository_files() {
  if in_git_repository; then
    git ls-files -z --cached --others --exclude-standard -- "$@"
    return
  fi
  for pattern in "$@"; do
    find . -name node_modules -prune -o -name target -prune -o -name .git -prune \
      -o -type f -name "$pattern" -print0
  done
}

has_files() {
  [ -n "$(repository_files "$@" | tr -d '\0')" ]
}

# Prints files matching the patterns, one per line, without test fixtures, vendored code and node_modules.
lockfiles() {
  repository_files "$@" | tr '\0' '\n' | sed 's#^\./##' \
    | grep -Ev '(^|/)(fixtures|vendor|node_modules)/' | sort
}

cargo_lockfiles() {
  lockfiles '*Cargo.lock' | grep -E '(^|/)Cargo\.lock$' || true
}

js_lockfiles() {
  lockfiles '*pnpm-lock.yaml' '*package-lock.json' \
    | grep -E '(^|/)(pnpm-lock\.yaml|package-lock\.json)$' || true
}

# Prints backend/ when it has a Cargo.toml, then each other directory with a Cargo.toml next to a Cargo.lock.
cargo_workspaces() {
  if [ -f backend/Cargo.toml ]; then
    echo backend
  fi
  cargo_lockfiles | while IFS= read -r lockfile; do
    workspace=$(dirname -- "$lockfile")
    if [ "$workspace" != backend ] && [ -f "$workspace/Cargo.toml" ]; then
      echo "$workspace"
    fi
  done
}

workflow_lint() {
  actionlint .github/workflows/*.yml
  zizmor --offline --persona regular --config zizmor.yml .github/workflows
}

secret_scan() {
  if in_git_repository && git rev-parse --verify HEAD >/dev/null 2>&1; then
    gitleaks git --no-banner --redact .
  else
    gitleaks dir --no-banner --redact .
  fi
}

osv() {
  resolved=$(mktemp -d)
  trap 'rm -rf "$resolved"' EXIT
  set --
  while IFS= read -r lockfile; do
    [ -f "$lockfile" ] || continue
    workspace=$(dirname -- "$lockfile")
    if [ ! -f "$workspace/Cargo.toml" ]; then
      set -- "$@" -L "$lockfile"
      continue
    fi
    python3 scripts/osv-resolved-lock.py "$workspace/Cargo.toml" "$resolved/$workspace"
    set -- "$@" -L "$resolved/$workspace/Cargo.lock"
  done <<EOF
$(cargo_lockfiles)
EOF
  while IFS= read -r lockfile; do
    if [ -f "$lockfile" ]; then
      set -- "$@" -L "$lockfile"
    fi
  done <<EOF
$(js_lockfiles)
EOF
  if [ "$#" -eq 0 ]; then
    echo "repo lint: no lockfile to scan"
    return
  fi
  osv-scanner scan source --config osv-scanner.toml "$@"
}

deny() {
  workspaces=$(cargo_workspaces)
  if [ -z "$workspaces" ]; then
    echo "repo lint: no Rust workspace to check"
    return
  fi
  failed=""
  while IFS= read -r workspace; do
    config=backend/deny.toml
    if [ -f "$workspace/deny.toml" ]; then
      config=$workspace/deny.toml
    fi
    echo "repo lint: cargo deny $workspace with $config"
    cargo deny --manifest-path "$workspace/Cargo.toml" --config "$config" check || failed="$failed $workspace"
  done <<EOF
$workspaces
EOF
  if [ -n "$failed" ]; then
    echo "repo lint: cargo deny failed for:$failed" >&2
    return 1
  fi
}

python_lint() {
  if ! has_files '*.py'; then
    echo "repo lint: no Python files"
    return
  fi
  repository_files '*.py' | xargs -0 ruff check --force-exclude
  repository_files '*.py' | xargs -0 ruff format --check --force-exclude
}

shell_lint() {
  if ! has_files '*.sh'; then
    echo "repo lint: no shell scripts"
    return
  fi
  repository_files '*.sh' | xargs -0 shellcheck
}

[ "$#" -eq 1 ] || usage
case "$1" in
  workflow-lint) workflow_lint ;;
  secret-scan) secret_scan ;;
  osv) osv ;;
  deny) deny ;;
  python-lint) python_lint ;;
  shell-lint) shell_lint ;;
  *) usage ;;
esac
