#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="$root/mobile/.qa"
action="${1:-start}"

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
xcrun simctl launch "$udid" dev.baukit.{{ context.app_name }}

echo "qa: iOS app ready on Simulator $udid"
echo "qa: run 'make e2e-ios-live' or inspect Simulator, then 'make qa-ios-down'"
