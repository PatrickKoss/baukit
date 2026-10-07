#!/usr/bin/env bash
set -euo pipefail

mobile_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
platform="${1:-}"
maestro_bin="$(command -v maestro || true)"
if [[ -z "$maestro_bin" && -x "$HOME/.maestro/bin/maestro" ]]; then
  maestro_bin="$HOME/.maestro/bin/maestro"
fi
[[ -n "$maestro_bin" ]] || {
  echo "qa: Maestro is required; install it from https://docs.maestro.dev/getting-started/installing-maestro" >&2
  exit 1
}
[[ -d "$mobile_dir/.maestro" ]] || { echo "qa: mobile/.maestro is missing" >&2; exit 1; }

case "$platform" in
  android)
    state_file="$mobile_dir/.qa/android-serial"
    app_id="dev.baukit.{{ context.app_crate }}"
    debug_dir="$mobile_dir/maestro-debug"
    ;;
  ios)
    state_file="$mobile_dir/.qa/ios-udid"
    app_id="dev.baukit.{{ context.app_name }}"
    debug_dir="$mobile_dir/maestro-debug-ios"
    ;;
  *)
    echo "usage: $0 [android|ios]" >&2
    exit 2
    ;;
esac

[[ -f "$state_file" ]] || { echo "qa: no live $platform QA environment; start it first" >&2; exit 1; }
device="$(<"$state_file")"
if [[ "$platform" == android ]]; then
  sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
  if [[ -z "$sdk_root" && -f "$mobile_dir/.qa/android-home" ]]; then
    sdk_root="$(<"$mobile_dir/.qa/android-home")"
  fi
  [[ -n "$sdk_root" ]] || { echo "qa: Android SDK location is missing" >&2; exit 1; }
  python3 "$mobile_dir/scripts/qa/android-adb.py" --check-health "$sdk_root/platform-tools/adb" "$device"
fi
cd "$mobile_dir"
"$maestro_bin" test \
  --platform "$platform" \
  --device "$device" \
  --env "APP_ID=$app_id" \
  --debug-output "$debug_dir" \
  .maestro
