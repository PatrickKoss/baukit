#!/usr/bin/env bash
set -euo pipefail

mobile_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state_dir="$mobile_dir/.qa"
action="${1:-prepare}"
device_name="${BAUKIT_QA_IOS_DEVICE:-{{ context.app_name }}-qa}"

require_macos() {
  [[ "$(uname -s)" == Darwin ]] || {
    echo "qa: iOS Simulator requires macOS and Xcode" >&2
    exit 1
  }
  command -v xcrun >/dev/null || { echo "qa: Xcode command-line tools are required" >&2; exit 1; }
  command -v python3 >/dev/null || { echo "qa: python3 is required" >&2; exit 1; }
}

find_device() {
  xcrun simctl list devices available -j | python3 -c '
import json, sys
name = sys.argv[1]
devices = json.load(sys.stdin).get("devices", {})
print(next((item["udid"] for values in devices.values() for item in values if item.get("name") == name and item.get("isAvailable", True)), ""))
' "$device_name"
}

prepare() {
  require_macos
  command -v xcodebuild >/dev/null || { echo "qa: Xcode is required" >&2; exit 1; }
  command -v pod >/dev/null || { echo "qa: CocoaPods is required" >&2; exit 1; }
  command -v corepack >/dev/null || { echo "qa: Node.js with Corepack is required" >&2; exit 1; }

  udid="$(find_device)"
  if [[ -z "$udid" ]]; then
    runtime_id="$(xcrun simctl list runtimes -j | python3 -c '
import json, re, sys
runtimes = [
    value for value in json.load(sys.stdin).get("runtimes", [])
    if value.get("isAvailable") and ".SimRuntime.iOS-" in value.get("identifier", "")
]
def version(value):
    return tuple(int(part) for part in re.findall(r"\d+", value.get("version", "0")))
print(max(runtimes, key=version).get("identifier", "") if runtimes else "")
')"
    device_type="$(xcrun simctl list devicetypes -j | python3 -c '
import json, sys
devices = json.load(sys.stdin).get("devicetypes", [])
preferred = next((item for item in devices if item.get("name") == "iPhone 16"), None)
fallback = next((item for item in reversed(devices) if item.get("name", "").startswith("iPhone")), None)
print((preferred or fallback or {}).get("identifier", ""))
')"
    [[ -n "$runtime_id" ]] || { echo "qa: Xcode has no available iOS Simulator runtime" >&2; exit 1; }
    [[ -n "$device_type" ]] || { echo "qa: Xcode has no available iPhone device type" >&2; exit 1; }
    udid="$(xcrun simctl create "$device_name" "$device_type" "$runtime_id")"
  fi

  mkdir -p "$state_dir"
  printf '%s\n' "$udid" > "$state_dir/ios-udid"
  echo "qa: iOS Simulator '$device_name' ready ($udid)"
}

case "$action" in
  prepare)
    prepare
    ;;
  boot)
    prepare
    udid="$(<"$state_dir/ios-udid")"
    open -a Simulator --args -CurrentDeviceUDID "$udid"
    xcrun simctl boot "$udid" >/dev/null 2>&1 || true
    xcrun simctl bootstatus "$udid" -b
    touch "$state_dir/ios-owned"
    echo "qa: iOS Simulator booted ($udid)"
    ;;
  stop)
    if [[ "$(uname -s)" == Darwin && -f "$state_dir/ios-owned" && -f "$state_dir/ios-udid" ]]; then
      xcrun simctl shutdown "$(<"$state_dir/ios-udid")" >/dev/null 2>&1 || true
    fi
    rm -f "$state_dir/ios-owned"
    echo "qa: iOS Simulator stopped"
    ;;
  *)
    echo "usage: $0 [prepare|boot|stop]" >&2
    exit 2
    ;;
esac
