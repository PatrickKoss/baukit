#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="$root/mobile/.qa"
action="${1:-start}"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$sdk_root" && -f "$state_dir/android-home" ]]; then
  sdk_root="$(<"$state_dir/android-home")"
fi
avd_home="${ANDROID_AVD_HOME:-}"
if [[ -z "$avd_home" && -f "$state_dir/android-avd-home" ]]; then
  avd_home="$(<"$state_dir/android-avd-home")"
fi
avd_name="${BAUKIT_QA_ANDROID_AVD:-{{ context.app_name }}-qa}"
emulator_port="${BAUKIT_QA_ANDROID_PORT:-5556}"
serial="emulator-$emulator_port"

stop_android() {
  if [[ -f "$state_dir/android-owned" && -n "$sdk_root" ]]; then
    "$sdk_root/platform-tools/adb" -s "$serial" emu kill >/dev/null 2>&1 || true
  fi
  rm -f "$state_dir/android-owned" "$state_dir/android-serial"
}

stop_all() {
  stop_android
  bash "$root/mobile/scripts/qa/services.sh" stop
}

if [[ "$action" == stop ]]; then
  stop_all
  echo "qa: Android environment stopped"
  exit 0
fi
[[ "$action" == start ]] || { echo "usage: $0 [start|stop]" >&2; exit 2; }
[[ -n "$sdk_root" && -n "$avd_home" ]] || {
  echo "qa: run 'make qa-android-setup' first" >&2
  exit 1
}

export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_AVD_HOME="$avd_home"
adb="$sdk_root/platform-tools/adb"
emulator="$sdk_root/emulator/emulator"
[[ -x "$adb" && -x "$emulator" ]] || {
  echo "qa: Android SDK is incomplete; run 'make qa-android-setup'" >&2
  exit 1
}

if "$adb" devices | awk 'NR > 1 {print $1}' | grep -Fxq "$serial" && [[ ! -f "$state_dir/android-owned" ]]; then
  echo "qa: $serial is already in use by an emulator this target did not start" >&2
  exit 1
fi

stop_all
mkdir -p "$state_dir"
bash "$root/mobile/scripts/qa/services.sh" start

emulator_args=(
  -avd "$avd_name"
  -port "$emulator_port"
  -no-snapshot-load
  -no-snapshot-save
  -no-boot-anim
  -gpu auto
)
if [[ "${BAUKIT_QA_HEADLESS:-0}" == 1 ]]; then
  emulator_args+=(-no-window)
fi
nohup "$emulator" "${emulator_args[@]}" >"$state_dir/android-emulator.log" 2>&1 &
printf '%s\n' "$serial" > "$state_dir/android-serial"
touch "$state_dir/android-owned"

ready=0
for _ in $(seq 1 180); do
  if [[ "$("$adb" -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; then
    ready=1
    break
  fi
  sleep 2
done
if [[ "$ready" != 1 ]]; then
  echo "qa: Android emulator timed out; see $state_dir/android-emulator.log" >&2
  exit 1
fi

"$adb" -s "$serial" shell settings put global window_animation_scale 0
"$adb" -s "$serial" shell settings put global transition_animation_scale 0
"$adb" -s "$serial" shell settings put global animator_duration_scale 0
"$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_API_PORT:-18080}" "tcp:${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_oidc %}"$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}" "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}
if [[ "${BAUKIT_QA_SKIP_BUILD:-0}" != 1 ]]; then
  bash "$root/mobile/scripts/qa/build-android.sh"
fi
apk="$root/mobile/android/app/build/outputs/apk/release/app-release.apk"
[[ -f "$apk" ]] || { echo "qa: missing $apk; run 'make qa-android-build'" >&2; exit 1; }
"$adb" -s "$serial" install -r "$apk" >/dev/null
"$adb" -s "$serial" shell monkey -p dev.baukit.{{ context.app_crate }} 1 >/dev/null

echo "qa: Android app ready on $serial"
echo "qa: run 'make e2e-android-live' or inspect the emulator, then 'make qa-android-down'"
