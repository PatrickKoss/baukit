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
manifest_quality() {
  python3 - "$root/baukit.toml" "$1" "$2" <<'QUALITY'
import sys, tomllib
from pathlib import Path
path = Path(sys.argv[1])
quality = tomllib.loads(path.read_text()).get("quality", {}) if path.is_file() else {}
print(quality.get(sys.argv[2], sys.argv[3]))
QUALITY
}
emulator_port="${BAUKIT_QA_ANDROID_PORT:-$(manifest_quality android_emulator_port 5556)}"
[[ "$emulator_port" =~ ^[0-9]+$ ]] || { echo "qa: Android emulator port must be a number" >&2; exit 2; }
emulator_count="${BAUKIT_QA_ANDROID_EMULATORS:-$(manifest_quality android_emulators 2)}"
[[ "$emulator_count" =~ ^[1-9][0-9]*$ ]] || { echo "qa: Android emulator count must be a positive integer" >&2; exit 2; }
snapshot_name=baukit-clean
serials=()
for ((index = 0; index < emulator_count; index++)); do
  serials+=("emulator-$((emulator_port + 2 * index))")
done
serial="${serials[0]}"

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
  local owned_serial status=0
  while IFS= read -r owned_serial; do
    [[ -n "$owned_serial" ]] || continue
    stop_emulator "$owned_serial" || status=$?
  done <"$state_dir/android-serial"
  return "$status"
}

stop_emulator() {
  local owned_serial="$1" devices owned_avd expected_avd
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
for candidate in "${serials[@]}"; do
  if printf '%s\n' "$devices" | awk '{print $1}' | grep -Fxq "$candidate" && [[ ! -f "$state_dir/android-owned" ]]; then
    echo "qa: $candidate is already in use by an emulator this target did not start" >&2
    exit 1
  fi
done

mkdir -p "$state_dir"
bash "$root/mobile/scripts/qa/services.sh" start

snapshot_dir="$avd_home/$avd_name.avd/snapshots/$snapshot_name"
mkdir -p "$state_dir/renderer"
printf '[Processor]\nThreadCount=%s\n[Testing]\nDisableServer=1\n' "$renderer_threads" > "$state_dir/renderer/SwiftShader.ini"
: > "$state_dir/android-serial"
printf '%s\n' "$avd_name" > "$state_dir/android-avd-name"
touch "$state_dir/android-owned"

launch_emulator() {
  local index="$1" port=$((emulator_port + 2 * $1)) log="$state_dir/android-emulator.log"
  local emulator_args=(
    -avd "$avd_name"
    -port "$port"
    -no-snapshot-save
    -no-audio
    -no-boot-anim
    -gpu swiftshader
  )
  if [[ -d "$snapshot_dir" ]]; then
    emulator_args+=(-snapshot "$snapshot_name")
  else
    emulator_args+=(-no-snapshot-load)
  fi
  if ((emulator_count > 1)) && [[ -d "$snapshot_dir" ]]; then
    emulator_args+=(-read-only)
  fi
  if ((index > 0)); then
    log="$state_dir/android-emulator-$port.log"
  fi
  if [[ "${BAUKIT_QA_HEADLESS:-0}" == 1 ]]; then
    emulator_args+=(-no-window)
  fi
  (
    cd "$state_dir/renderer"
    export LP_NUM_THREADS="$renderer_threads"
    exec nohup "$emulator" "${emulator_args[@]}"
  ) >"$log" 2>&1 &
  printf 'emulator-%s\n' "$port" >> "$state_dir/android-serial"
}

await_emulator() {
  local device="$1"
  if ! python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-boot "$adb" "$device"; then
    echo "qa: Android emulator $device timed out; see $state_dir/android-emulator*.log" >&2
    exit 1
  fi
  if ! python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-idle "$adb" "$device"; then
    echo "qa: systemui did not become idle on $device" >&2
    exit 1
  fi
  python3 "$root/mobile/scripts/qa/android-adb.py" --check-health "$adb" "$device" "$state_dir/android-health.log"
}

if [[ ! -d "$snapshot_dir" ]]; then
  launch_emulator 0
  await_emulator "$serial"
  adb_bounded -s "$serial" emu avd snapshot save "$snapshot_name" >/dev/null
  echo "qa: saved quick-boot snapshot $snapshot_name"
  if ((emulator_count > 1)); then
    adb_bounded -s "$serial" emu kill >/dev/null
    python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-stop "$adb" "$serial"
    : > "$state_dir/android-serial"
  fi
fi
for ((index = 0; index < emulator_count; index++)); do
  if ((index > 0)) || [[ ! -s "$state_dir/android-serial" ]]; then
    launch_emulator "$index"
  fi
done
for device in "${serials[@]}"; do
  await_emulator "$device"
done

if [[ "${BAUKIT_QA_SKIP_BUILD:-0}" != 1 ]]; then
  bash "$root/mobile/scripts/qa/build-android.sh"
fi
apk="$root/mobile/android/app/build/outputs/apk/release/app-release.apk"
[[ -f "$apk" ]] || { echo "qa: missing $apk; run 'make qa-android-build'" >&2; exit 1; }
for serial in "${serials[@]}"; do
  "$adb" -s "$serial" shell settings put global window_animation_scale 0
  "$adb" -s "$serial" shell settings put global transition_animation_scale 0
  "$adb" -s "$serial" shell settings put global animator_duration_scale 0
  "$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_API_PORT:-18080}" "tcp:${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_oidc %}  "$adb" -s "$serial" reverse "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}" "tcp:${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
  "$adb" -s "$serial" shell am force-stop com.android.chrome
  "$adb" -s "$serial" shell 'echo "chrome --disable-fre --no-first-run --no-default-browser-check" > /data/local/tmp/chrome-command-line'
  "$adb" -s "$serial" shell settings put global debug_app com.android.chrome
{% endif %}
  "$adb" -s "$serial" install -r "$apk" >/dev/null
  "$adb" -s "$serial" shell am start -n dev.baukit.{{ context.app_crate }}/.MainActivity -W >/dev/null
done

echo "qa: Android app ready on ${serials[*]}"
echo "qa: run 'make e2e-android-live' or inspect the emulator, then 'make qa-android-down'"
