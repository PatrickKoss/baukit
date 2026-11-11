#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="${BAUKIT_QA_STATE_DIR:-$root/mobile/.qa}"
action="${1:-start}"
api_port="${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_enabled %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}qa_ios_bundle_id="${BAUKIT_QA_IOS_BUNDLE_ID:-dev.baukit.{{ context.app_name }}}"
local_hooks="$root/mobile/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"
export BAUKIT_QA_API_PORT="$api_port"
{% if context.auth_enabled %}export BAUKIT_QA_KEYCLOAK_PORT="$keycloak_port"
{% endif %}
stop_all() {
  bash "$root/mobile/scripts/qa/ios-simulator.sh" stop
  bash "$root/mobile/scripts/qa/services.sh" stop
}

if [[ "$action" == stop ]]; then
  stop_all
  echo "qa: iOS environment stopped"
  exit 0
fi
[[ "$action" == start ]] || { echo "usage: $0 [start|stop]" >&2; exit 2; }

stop_all
bash "$root/mobile/scripts/qa/services.sh" start
bash "$root/mobile/scripts/qa/ios-simulator.sh" boot
if [[ "${BAUKIT_QA_SKIP_BUILD:-0}" != 1 ]]; then
  bash "$root/mobile/scripts/qa/build-ios.sh"
fi

[[ -f "$state_dir/ios-udid" ]] || { echo "qa: missing iOS Simulator state" >&2; exit 1; }
[[ -f "$state_dir/ios-app" ]] || { echo "qa: missing iOS app; run 'make qa-ios-build'" >&2; exit 1; }
udid="$(<"$state_dir/ios-udid")"
app="$(<"$state_dir/ios-app")"
[[ -d "$app" ]] || { echo "qa: iOS app no longer exists at $app" >&2; exit 1; }
xcrun simctl install "$udid" "$app"
xcrun simctl launch "$udid" "$qa_ios_bundle_id"

echo "qa: iOS app ready on Simulator $udid"
echo "qa: run 'make e2e-ios-live' or inspect Simulator, then 'make qa-ios-down'"
