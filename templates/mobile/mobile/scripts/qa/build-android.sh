#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="$root/mobile/.qa"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$sdk_root" && -f "$state_dir/android-home" ]]; then
  sdk_root="$(<"$state_dir/android-home")"
fi
[[ -n "$sdk_root" ]] || { echo "qa: run 'make qa-android-setup' first" >&2; exit 1; }
architecture="${BAUKIT_QA_ANDROID_ARCHITECTURE:-}"
if [[ -z "$architecture" && -f "$state_dir/android-architecture" ]]; then
  architecture="$(<"$state_dir/android-architecture")"
fi
[[ -n "$architecture" ]] || { echo "qa: Android architecture is unknown; run 'make qa-android-setup'" >&2; exit 1; }

export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export BAUKIT_QA_BUILD=1
export EXPO_PUBLIC_API_URL="http://localhost:${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_oidc %}export EXPO_PUBLIC_OIDC_ISSUER="http://localhost:${BAUKIT_QA_KEYCLOAK_PORT:-18081}/realms/{{ context.app_name }}"
export EXPO_PUBLIC_OIDC_CLIENT_ID="{{ context.app_name }}-mobile"
{% elif context.auth_enabled %}export EXPO_PUBLIC_LOCAL_OIDC_ISSUER="http://localhost:${BAUKIT_QA_KEYCLOAK_PORT:-18081}/realms/{{ context.app_name }}"
{% endif %}

if [[ -f "$root/mobile/.env" ]]; then
  set -a
  source "$root/mobile/.env"
  set +a
  export EXPO_PUBLIC_API_URL="http://localhost:${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_oidc %}  export EXPO_PUBLIC_OIDC_ISSUER="http://localhost:${BAUKIT_QA_KEYCLOAK_PORT:-18081}/realms/{{ context.app_name }}"
  export EXPO_PUBLIC_OIDC_CLIENT_ID="{{ context.app_name }}-mobile"
{% elif context.auth_enabled %}export EXPO_PUBLIC_LOCAL_OIDC_ISSUER="http://localhost:${BAUKIT_QA_KEYCLOAK_PORT:-18081}/realms/{{ context.app_name }}"
{% endif %}
fi
export BAUKIT_QA_BUILD=1

corepack pnpm@12.9.1 --dir "$root/mobile" install --frozen-lockfile
corepack pnpm@12.9.1 --dir "$root/mobile" run tokens
(cd "$root/mobile" && CI=1 ./node_modules/.bin/expo prebuild \
  --clean --platform android --no-install)
gradle=(bash "$root/mobile/scripts/android-java.sh" "$root/mobile/android/gradlew" -p "$root/mobile/android"
  --no-daemon --build-cache -PreactNativeDevServerIp=127.0.0.1)
if [[ -n "${BAUKIT_QA_GRADLE_WORKERS:-}" ]]; then
  gradle+=(--max-workers "$BAUKIT_QA_GRADLE_WORKERS")
fi
if [[ "${BAUKIT_QA_ASSEMBLE_DEBUG:-0}" == 1 ]]; then
  "${gradle[@]}" --stacktrace assembleDebug
fi
NODE_ENV=production "${gradle[@]}" -PreactNativeArchitectures="$architecture" assembleRelease

apk="$root/mobile/android/app/build/outputs/apk/release/app-release.apk"
[[ -f "$apk" ]] || { echo "qa: Android build did not produce $apk" >&2; exit 1; }
echo "qa: built $apk"
