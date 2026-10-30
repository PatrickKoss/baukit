#!/usr/bin/env bash
# Publish the baukit TypeScript packages to npm in dependency order.
#
# Mirrors scripts/publish-crates.sh: packages already on the registry at the
# current version are skipped, so an interrupted run can simply be restarted.
# Publishing uses `npm publish` rather than `pnpm publish` because only the
# npm CLI implements the OIDC exchange that trusted publishing needs.
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

# Packages depending on another baukit package must follow it.
ORDER=(
  a11y-core analytics-core api-runtime events localization-core
  preferences-core ui-tokens navigation data-contracts auth-native auth-node auth-web
  pwa-web sync-client integrations-client suite-client
  analytics-posthog-native analytics-posthog-web
  data-contracts-dexie data-contracts-expo-sqlite
  notifications-core notifications-expo
)

PUBLISH_ATTEMPTS=3
RETRY_BACKOFF_SECONDS=20
REGISTRY_POLLS=6
REGISTRY_POLL_SECONDS=10

version=$(python3 -c "
import json
print(json.load(open('typescript/packages/a11y-core/package.json'))['version'])
")

already_published() {
  local name=$1
  curl -sf -H 'User-Agent: dependency-check' "https://registry.npmjs.org/@baukit%2F${name}" \
    | python3 -c "
import json, sys
try:
    print('yes' if '$version' in json.load(sys.stdin).get('versions', {}) else 'no')
except Exception:
    print('no')
" | grep -q yes
}

# npm can reject a publish whose upload the registry already committed, and
# its read side lags, so poll before deciding the version is missing.
landed_after_failure() {
  local name=$1 poll
  for (( poll = 1; poll <= REGISTRY_POLLS; poll++ )); do
    already_published "$name" && return 0
    sleep "$REGISTRY_POLL_SECONDS"
  done
  return 1
}

publish_package() {
  local name=$1 dir=$2 attempt
  for (( attempt = 1; attempt <= PUBLISH_ATTEMPTS; attempt++ )); do
    ( cd "$dir" && npm publish --access public --ignore-scripts ) && return 0
    if landed_after_failure "$name"; then
      echo "== @baukit/$name $version reached npm despite the error, continuing"
      return 0
    fi
    (( attempt < PUBLISH_ATTEMPTS )) || break
    echo "== retrying @baukit/$name in $(( RETRY_BACKOFF_SECONDS * attempt ))s (attempt $(( attempt + 1 ))/$PUBLISH_ATTEMPTS)" >&2
    sleep $(( RETRY_BACKOFF_SECONDS * attempt ))
  done
  echo "publishing @baukit/$name $version failed after $PUBLISH_ATTEMPTS attempts; see the output above" >&2
  return 1
}

for pkg in "${ORDER[@]}"; do
  dir="typescript/packages/$pkg"
  [[ -d $dir ]] || { echo "unknown package directory: $dir" >&2; exit 1; }

  if already_published "$pkg"; then
    echo "== @baukit/$pkg $version already on npm, skipping"
    continue
  fi

  python3 scripts/check-package-dependencies.py "$dir"
  echo "== publishing @baukit/$pkg $version"
  # --ignore-scripts: dist/ is already built by the caller; `prepare` would
  # rebuild it once per package and roughly double the job runtime.
  publish_package "$pkg" "$dir"
done

echo "all ${#ORDER[@]} packages published at $version"
