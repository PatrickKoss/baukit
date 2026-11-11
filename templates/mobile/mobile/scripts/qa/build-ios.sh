#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
qa_build_env() { :; }
qa_install_dependencies() {
  corepack pnpm@12.9.1 --dir "$root/mobile" install --frozen-lockfile
}
api_port="${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_enabled %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}local_hooks="$root/mobile/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"
state_dir="${BAUKIT_QA_STATE_DIR:-$root/mobile/.qa}"
[[ "$(uname -s)" == Darwin ]] || { echo "qa: iOS builds require macOS and Xcode" >&2; exit 1; }

export BAUKIT_QA_BUILD=1
export EXPO_PUBLIC_API_URL="http://localhost:$api_port"
{% if context.auth_oidc %}export EXPO_PUBLIC_OIDC_ISSUER="http://localhost:$keycloak_port/realms/{{ context.app_name }}"
export EXPO_PUBLIC_OIDC_CLIENT_ID="{{ context.app_name }}-mobile"
{% elif context.auth_enabled %}export EXPO_PUBLIC_LOCAL_OIDC_ISSUER="http://localhost:$keycloak_port/realms/{{ context.app_name }}"
{% endif %}

if [[ -f "$root/mobile/.env" ]]; then
  set -a
  # The developer-owned .env is optional and absent when shellcheck runs.
  # shellcheck source=/dev/null
  source "$root/mobile/.env"
  set +a
fi
export BAUKIT_QA_BUILD=1
export EXPO_PUBLIC_API_URL="http://localhost:$api_port"
{% if context.auth_oidc %}export EXPO_PUBLIC_OIDC_ISSUER="http://localhost:$keycloak_port/realms/{{ context.app_name }}"
export EXPO_PUBLIC_OIDC_CLIENT_ID="{{ context.app_name }}-mobile"
{% elif context.auth_enabled %}export EXPO_PUBLIC_LOCAL_OIDC_ISSUER="http://localhost:$keycloak_port/realms/{{ context.app_name }}"
{% endif %}

qa_build_env
qa_install_dependencies
corepack pnpm@12.9.1 --dir "$root/mobile" run tokens
(cd "$root/mobile" && CI=1 ./node_modules/.bin/expo prebuild \
  --clean --platform ios --no-install)
(cd "$root/mobile/ios" && pod install)

workspace="$(find "$root/mobile/ios" -maxdepth 1 -name '*.xcworkspace' -print -quit)"
[[ -n "$workspace" ]] || { echo "qa: Expo did not produce an Xcode workspace" >&2; exit 1; }
scheme="${BAUKIT_QA_IOS_SCHEME:-}"
[[ -n "$scheme" ]] || scheme="$(xcodebuild -workspace "$workspace" -list -json | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspace"]["schemes"][0])')"
[[ -n "$scheme" ]] || { echo "qa: Xcode workspace has no build scheme" >&2; exit 1; }

mkdir -p "$state_dir"
NODE_ENV=production xcodebuild \
  -workspace "$workspace" \
  -scheme "$scheme" \
  -configuration Release \
  -sdk iphonesimulator \
  -derivedDataPath "$root/mobile/ios-build" \
  CODE_SIGNING_ALLOWED=NO \
  build 2>&1 | tee "$state_dir/ios-build.log"

app="$(find "$root/mobile/ios-build/Build/Products/Release-iphonesimulator" -maxdepth 1 -name '*.app' -print -quit)"
[[ -n "$app" ]] || { echo "qa: iOS build did not produce a Simulator app" >&2; exit 1; }
printf '%s\n' "$app" > "$state_dir/ios-app"
echo "qa: built $app"
