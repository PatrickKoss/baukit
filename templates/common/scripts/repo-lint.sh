#!/bin/sh
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

usage() {
  echo "usage: sh scripts/repo-lint.sh workflow-lint|secret-scan|osv|python-lint|shell-lint" >&2
  exit 2
}

in_git_repository() {
  git rev-parse --is-inside-work-tree >/dev/null 2>&1
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

workflow_lint() {
  actionlint
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
  set --
  for lockfile in backend/Cargo.lock web/pnpm-lock.yaml mobile/pnpm-lock.yaml; do
    if [ -f "$lockfile" ]; then
      set -- "$@" -L "$lockfile"
    fi
  done
  if [ "$#" -eq 0 ]; then
    echo "repo lint: no lockfile to scan"
    return
  fi
  osv-scanner scan source --config osv-scanner.toml "$@"
}

python_lint() {
  if ! has_files '*.py'; then
    echo "repo lint: no Python files"
    return
  fi
  repository_files '*.py' | xargs -0 ruff check
  repository_files '*.py' | xargs -0 ruff format --check
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
  python-lint) python_lint ;;
  shell-lint) shell_lint ;;
  *) usage ;;
esac
