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
  preferences-core ui-tokens data-contracts auth-native auth-node auth-web
  pwa-web sync-client integrations-client
  analytics-posthog-native analytics-posthog-web
  data-contracts-dexie data-contracts-expo-sqlite
)

version=$(python3 -c "
import json
print(json.load(open('typescript/packages/a11y-core/package.json'))['version'])
")

already_published() {
  local name=$1
  curl -sf "https://registry.npmjs.org/@baukit%2F${name}" \
    | python3 -c "
import json, sys
try:
    print('yes' if '$version' in json.load(sys.stdin).get('versions', {}) else 'no')
except Exception:
    print('no')
" | grep -q yes
}

for pkg in "${ORDER[@]}"; do
  dir="typescript/packages/$pkg"
  [[ -d $dir ]] || { echo "unknown package directory: $dir" >&2; exit 1; }

  if already_published "$pkg"; then
    echo "== @baukit/$pkg $version already on npm, skipping"
    continue
  fi

  echo "== publishing @baukit/$pkg $version"
  # --ignore-scripts: dist/ is already built by the caller; `prepare` would
  # rebuild it once per package and roughly double the job runtime.
  ( cd "$dir" && npm publish --access public --ignore-scripts )
done

echo "all ${#ORDER[@]} packages published at $version"
