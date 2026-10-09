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

adb="$sdk_root/platform-tools/adb"
adb_bounded() {
  python3 "$root/mobile/scripts/qa/android-adb.py" "$adb" "$@"
}

stop_android() {
  [[ -f "$state_dir/android-owned" ]] || return 0
  [[ -n "$sdk_root" && -f "$state_dir/android-serial" ]] || {
    echo "qa: cannot locate the owned Android emulator; ownership state was kept" >&2
    return 1
  }
  local owned_serial devices owned_avd expected_avd
  owned_serial="$(<"$state_dir/android-serial")"
  [[ "$owned_serial" =~ ^emulator-[0-9]+$ ]] || return 1
  devices="$(adb_bounded devices)" || return
  if ! printf '%s\n' "$devices" | awk -v serial="$owned_serial" '$1 == serial { found = 1 } END { exit !found }'; then
    return 0
  fi
  expected_avd="$avd_name"
  if [[ -f "$state_dir/android-avd-name" ]]; then
    expected_avd="$(<"$state_dir/android-avd-name")"
  fi
  owned_avd="$(adb_bounded -s "$owned_serial" emu avd name)" || return
  owned_avd="$(printf '%s\n' "$owned_avd" | tr -d '\r' | sed -n '1p')"
  [[ "$owned_avd" == "$expected_avd" ]] || {
    echo "qa: refusing to stop $owned_serial because it belongs to '$owned_avd'" >&2
    return 1
  }
  adb_bounded -s "$owned_serial" emu kill || return
  python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-stop "$adb" "$owned_serial"
}

stop_all() {
  local status=0
  stop_android || status=$?
  bash "$root/mobile/scripts/qa/services.sh" stop || status=$?
  if [[ "$status" != 0 ]]; then
    echo "qa: Android cleanup failed; ownership state was kept" >&2
    return "$status"
  fi
  rm -f "$state_dir/android-owned" "$state_dir/android-serial" "$state_dir/android-avd-name"
  rm -rf "$state_dir/renderer"
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
emulator="$sdk_root/emulator/emulator"
[[ -x "$adb" && -x "$emulator" ]] || {
  echo "qa: Android SDK is incomplete; run 'make qa-android-setup'" >&2
  exit 1
}

renderer_threads="${BAUKIT_QA_RENDERER_THREADS:-4}"
[[ "$renderer_threads" =~ ^[1-9][0-9]*$ ]] || {
  echo "qa: BAUKIT_QA_RENDERER_THREADS must be a positive integer" >&2
  exit 2
}
stop_all
devices="$(adb_bounded devices)"
if printf '%s\n' "$devices" | awk '{print $1}' | grep -Fxq "$serial" && [[ ! -f "$state_dir/android-owned" ]]; then
  echo "qa: $serial is already in use by an emulator this target did not start" >&2
  exit 1
fi

mkdir -p "$state_dir"
bash "$root/mobile/scripts/qa/services.sh" start

emulator_args=(
  -avd "$avd_name"
  -port "$emulator_port"
  -no-snapshot-load
  -no-snapshot-save
  -no-boot-anim
  -gpu swiftshader
)
if [[ "${BAUKIT_QA_HEADLESS:-0}" == 1 ]]; then
  emulator_args+=(-no-window)
fi
mkdir -p "$state_dir/renderer"
printf '[Processor]\nThreadCount=%s\n[Testing]\nDisableServer=1\n' "$renderer_threads" > "$state_dir/renderer/SwiftShader.ini"
(
  cd "$state_dir/renderer"
  export LP_NUM_THREADS="$renderer_threads"
  exec nohup "$emulator" "${emulator_args[@]}"
) >"$state_dir/android-emulator.log" 2>&1 &
printf '%s\n' "$serial" > "$state_dir/android-serial"
printf '%s\n' "$avd_name" > "$state_dir/android-avd-name"
touch "$state_dir/android-owned"

if ! python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-boot "$adb" "$serial"; then
  echo "qa: Android emulator timed out; see $state_dir/android-emulator.log" >&2
  exit 1
fi

python3 "$root/mobile/scripts/qa/android-adb.py" --check-health "$adb" "$serial" "$state_dir/android-health.log"

"$adb" -s "$serial" shell settings put global window_animation_scale 0
"$adb" -s "$serial" shell settings put global transition_animation_scale 0
"$adb" -s "$serial" shell settings put global animator_duration_scale 0
"$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_API_PORT:-18080}" "tcp:${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_oidc %}"$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}" "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
"$adb" -s "$serial" shell am force-stop com.android.chrome
"$adb" -s "$serial" shell 'echo "chrome --disable-fre --no-first-run --no-default-browser-check" > /data/local/tmp/chrome-command-line'
"$adb" -s "$serial" shell settings put global debug_app com.android.chrome
{% endif %}
if [[ "${BAUKIT_QA_SKIP_BUILD:-0}" != 1 ]]; then
  bash "$root/mobile/scripts/qa/build-android.sh"
fi
apk="$root/mobile/android/app/build/outputs/apk/release/app-release.apk"
[[ -f "$apk" ]] || { echo "qa: missing $apk; run 'make qa-android-build'" >&2; exit 1; }
"$adb" -s "$serial" install -r "$apk" >/dev/null
"$adb" -s "$serial" shell am start -n dev.baukit.{{ context.app_crate }}/.MainActivity -W >/dev/null

echo "qa: Android app ready on $serial"
echo "qa: run 'make e2e-android-live' or inspect the emulator, then 'make qa-android-down'"
