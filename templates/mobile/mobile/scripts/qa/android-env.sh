#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
state_dir="${BAUKIT_QA_STATE_DIR:-$root/mobile/.qa}"
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
qa_emulator_args=()
emulator_accel="${BAUKIT_QA_ANDROID_ACCEL:-}"
qa_reverse_ports=()
qa_app_id="${BAUKIT_QA_APP_ID:-dev.baukit.{{ context.app_crate }}}"
qa_apk_variant="${BAUKIT_QA_APK_VARIANT:-release}"
qa_apk_path=""
api_port="${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_enabled %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}qa_build_apk() { bash "$root/mobile/scripts/qa/build-android.sh"; }
qa_prepare_device() { prepare_device "$@"; }
local_hooks="$root/mobile/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"
[[ "$qa_apk_variant" =~ ^(release|debug)$ ]] || { echo "qa: qa_apk_variant must be release or debug" >&2; exit 2; }
[[ -n "$qa_apk_path" ]] || qa_apk_path="$root/mobile/android/app/build/outputs/apk/$qa_apk_variant/app-$qa_apk_variant.apk"
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
# Bump when the saved quick-boot state changes; it keys the snapshot name.
snapshot_schema=2
gpu_mode="${BAUKIT_QA_ANDROID_GPU:-swiftshader_indirect}"
[[ "$gpu_mode" =~ ^[a-z_]+$ ]] || { echo "qa: BAUKIT_QA_ANDROID_GPU must be an emulator gpu mode" >&2; exit 2; }
[[ -z "$emulator_accel" || "$emulator_accel" =~ ^[a-z]+$ ]] || { echo "qa: BAUKIT_QA_ANDROID_ACCEL must be an emulator accel mode" >&2; exit 2; }
android_wifi="${BAUKIT_QA_ANDROID_WIFI:-$(manifest_quality android_wifi false)}"
case "$android_wifi" in
  true | True | 1) android_wifi=1 ;;
  false | False | 0) android_wifi=0 ;;
  *) echo "qa: quality.android_wifi must be true or false" >&2; exit 2 ;;
esac
serials=()
for ((index = 0; index < emulator_count; index++)); do
  serials+=("emulator-$((emulator_port + 2 * index))")
done
serial="${serials[0]}"

adb="$sdk_root/platform-tools/adb"
adb_bounded() {
  python3 "$root/mobile/scripts/qa/android-adb.py" "$adb" "$@"
}
# shellcheck source=android-broker.sh
source "$root/mobile/scripts/qa/android-broker.sh"

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

emulator_pid() {
  local pid_file="$state_dir/android-emulator-${1#emulator-}.pid"
  [[ -f "$pid_file" ]] && cat "$pid_file"
  return 0
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
  [[ "$owned_avd" == "$expected_avd" || "$owned_avd" =~ ^"$expected_avd"-[1-9][0-9]*$ ]] || {
    echo "qa: refusing to stop $owned_serial because it belongs to '$owned_avd'" >&2
    return 1
  }
  adb_bounded -s "$owned_serial" emu kill || return
  python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-stop "$adb" "$owned_serial" "$(emulator_pid "$owned_serial")"
}

clear_ownership() {
  rm -f "$state_dir/android-owned" "$state_dir/android-serial" "$state_dir/android-avd-name"
  rm -f "$state_dir"/android-emulator-*.pid
  rm -rf "$state_dir/renderer"
}

stop_all() {
  local status=0
  release_leased_devices || status=$?
  stop_android || status=$?
  bash "$root/mobile/scripts/qa/services.sh" stop || status=$?
  if [[ "$status" != 0 ]]; then
    echo "qa: Android cleanup failed; ownership state was kept" >&2
    return "$status"
  fi
  clear_ownership
}

stop_emulators() {
  release_leased_devices || return
  stop_android || {
    echo "qa: Android cleanup failed; ownership state was kept" >&2
    return 1
  }
  clear_ownership
}

if [[ "$action" == stop ]]; then
  stop_all
  echo "qa: Android environment stopped"
  exit 0
fi
[[ "$action" =~ ^(start|boot|prepare|reverse)$ ]] || { echo "usage: $0 [start|boot|prepare|reverse DEVICE|stop]" >&2; exit 2; }
sdk_configured() { [[ -n "$sdk_root" ]] && { broker_enabled || [[ -n "$avd_home" ]]; }; }
sdk_complete() { [[ -x "$adb" ]] && { broker_enabled || [[ -x "$emulator" ]]; }; }
sdk_configured || {
  echo "qa: run 'make qa-android-setup' first" >&2
  exit 1
}

export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_AVD_HOME="$avd_home"
emulator="$sdk_root/emulator/emulator"
sdk_complete || {
  echo "qa: Android SDK is incomplete; run 'make qa-android-setup'" >&2
  exit 1
}

renderer_threads="${BAUKIT_QA_RENDERER_THREADS:-4}"
[[ "$renderer_threads" =~ ^[1-9][0-9]*$ ]] || {
  echo "qa: BAUKIT_QA_RENDERER_THREADS must be a positive integer" >&2
  exit 2
}

adb_checked() {
  local output status=0
  output="$(adb_bounded "$@" 2>&1)" || status=$?
  if ((status)); then
    printf '%s\n' "$output" >&2
    return "$status"
  fi
  CHECKED_OUTPUT="$output"
}

launch_app() {
  local device="$1" status=0
  adb_checked -s "$device" shell am start -n "$qa_app_id/.MainActivity" -W || status=$?
  if ((status)); then
    echo "qa: could not start $qa_app_id on $device" >&2
    return "$status"
  fi
  if grep -q '^Error' <<<"$CHECKED_OUTPUT"; then
    printf '%s\n' "$CHECKED_OUTPUT" >&2
    echo "qa: could not start $qa_app_id on $device" >&2
    return 1
  fi
}

apply_reverse() {
  local device="$1" port
  adb_bounded -s "$device" reverse "tcp:$api_port" "tcp:$api_port"
  for port in "${qa_reverse_ports[@]}"; do
    adb_bounded -s "$device" reverse "tcp:$port" "tcp:$port"
  done
{% if context.auth_enabled %}  adb_bounded -s "$device" reverse "tcp:$keycloak_port" "tcp:$keycloak_port"
{% endif %}}

{% if context.auth_enabled %}# Chrome's first start in a fresh profile keeps its main thread busy for seconds. Inside the
# login flow that work lands on the Custom Tab's first input, which under host load times out
# as an input dispatch ANR, so Chrome starts once here, before any flow.
warm_chrome() {
  local device="$1"
  adb_bounded -s "$device" shell am force-stop com.android.chrome
  adb_bounded -s "$device" shell 'echo "chrome --disable-fre --no-first-run --no-default-browser-check --force-renderer-accessibility=complete" > /data/local/tmp/chrome-command-line'
  adb_bounded -s "$device" shell settings put global debug_app com.android.chrome
  adb_checked -s "$device" shell am start -W -a android.intent.action.VIEW -d about:blank -p com.android.chrome || {
    echo "qa: could not start Chrome on $device" >&2
    return 1
  }
  adb_bounded -s "$device" shell input keyevent KEYCODE_HOME
}

{% endif %}prepare_device() {
  local device="$1" apk="$2"
  adb_bounded -s "$device" shell settings put global window_animation_scale 0
  adb_bounded -s "$device" shell settings put global transition_animation_scale 0
  adb_bounded -s "$device" shell settings put global animator_duration_scale 0
  apply_reverse "$device"
{% if context.auth_enabled %}  warm_chrome "$device"
{% endif %}  adb_checked -s "$device" install -r "$apk"
  launch_app "$device"
}

prepare_devices() {
  local device pid status=0 pids=()
  for device in "${serials[@]}"; do
    qa_prepare_device "$device" "$qa_apk_path" &
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do
    wait "$pid" || status=$?
  done
  return "$status"
}

prepare_android() {
  if [[ "${BAUKIT_QA_SKIP_BUILD:-0}" != 1 ]]; then
    qa_build_apk
  fi
  [[ -f "$qa_apk_path" ]] || { echo "qa: missing $qa_apk_path; run 'make qa-android-build'" >&2; exit 1; }
  prepare_devices
  echo "qa: Android app ready on ${serials[*]}"
  echo "qa: run 'make e2e-android-live' or inspect the emulator, then 'make qa-android-down'"
}

reject_foreign_emulators() {
  local devices candidate
  devices="$(adb_bounded devices)"
  for candidate in "${serials[@]}"; do
    if printf '%s\n' "$devices" | awk '{print $1}' | grep -Fxq "$candidate" && [[ ! -f "$state_dir/android-owned" ]]; then
      echo "qa: $candidate is already in use by an emulator this target did not start" >&2
      exit 1
    fi
  done
}

close_inherited_fds() {
  local entry fd bash_script_fd=255
  for entry in /dev/fd/*; do
    fd="${entry##*/}"
    [[ "$fd" =~ ^[0-9]+$ ]] || continue
    ((fd > 2 && fd != bash_script_fd)) || continue
    eval "exec $fd>&-"
  done
}

avd_for_index() {
  if (($1 == 0)); then
    printf '%s\n' "$avd_name"
  else
    printf '%s-%s\n' "$avd_name" "$1"
  fi
}

select_avd() {
  index_avd="$(avd_for_index "$1")"
  avd_dir="$avd_home/$index_avd.avd"
  snapshot_name="$(python3 "$root/mobile/scripts/qa/android-adb.py" --snapshot-name "$avd_dir/config.ini" "$sdk_root" "$gpu_mode" "$emulator_features" "$snapshot_schema")"
  snapshot_dir="$avd_dir/snapshots/$snapshot_name"
  snapshot_ready="$avd_dir/$snapshot_name.ready"
}

# Concurrent -read-only instances of one AVD crash in the emulator's disk re-open path, so
# every extra emulator boots its own AVD, cloned from the base AVD's config.ini.
ensure_avd_clone() {
  local clone base_dir="$avd_home/$avd_name.avd" clone_dir
  clone="$(avd_for_index "$1")"
  clone_dir="$avd_home/$clone.avd"
  [[ -f "$base_dir/config.ini" ]] || { echo "qa: AVD '$avd_name' has no config.ini; run 'make qa-android-setup'" >&2; exit 1; }
  if cmp -s "$base_dir/config.ini" "$clone_dir/baukit-base-config.ini"; then
    return 0
  fi
  if [[ -d "$clone_dir" ]]; then
    echo "qa: recreating AVD '$clone' because '$avd_name' changed its config"
    rm -rf "$clone_dir"
  fi
  mkdir -p "$clone_dir"
  cp "$base_dir/config.ini" "$clone_dir/config.ini"
  {
    echo "avd.ini.encoding=UTF-8"
    echo "path=$clone_dir"
    echo "path.rel=avd/$clone.avd"
    grep '^target=' "$avd_home/$avd_name.ini" || true
  } >"$avd_home/$clone.ini"
  cp "$base_dir/config.ini" "$clone_dir/baukit-base-config.ini"
}

launch_emulator() {
  local index="$1" relaunch="${2:-}" port=$((emulator_port + 2 * $1)) log="$state_dir/android-emulator.log"
  local emulator_args=(
    -avd "$index_avd"
    -port "$port"
    -no-snapshot-save
    -no-audio
    -no-boot-anim
    -gpu "$gpu_mode"
  )
  if [[ -n "$emulator_features" ]]; then
    emulator_args+=(-feature "$emulator_features")
  fi
  if [[ -f "$snapshot_ready" ]]; then
    emulator_args+=(-snapshot "$snapshot_name")
  else
    emulator_args+=(-no-snapshot-load)
  fi
  if ((index > 0)); then
    log="$state_dir/android-emulator-$port.log"
  fi
  if [[ "${BAUKIT_QA_HEADLESS:-0}" == 1 ]]; then
    emulator_args+=(-no-window)
  fi
  if [[ -n "$emulator_accel" ]]; then
    emulator_args+=(-accel "$emulator_accel")
  fi
  emulator_args+=("${qa_emulator_args[@]}")
  if [[ -n "$relaunch" && -f "$log" ]]; then
    mv "$log" "$log.crashed"
  fi
  (
    close_inherited_fds
    cd "$state_dir/renderer"
    export LP_NUM_THREADS="$renderer_threads"
    exec nohup "$emulator" "${emulator_args[@]}"
  ) >"$log" 2>&1 &
  printf '%s\n' "$!" > "$state_dir/android-emulator-$port.pid"
  [[ -n "$relaunch" ]] || printf 'emulator-%s\n' "$port" >> "$state_dir/android-serial"
}

declare -A relaunched_emulators=()

wait_for_emulator_boot() {
  local device="$1" status=0
  python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-boot "$adb" "$device" "$(emulator_pid "$device")" || status=$?
  if ((status == 1)) && [[ "$device" == emulator-* && -z "${relaunched_emulators[$device]:-}" ]]; then
    relaunched_emulators[$device]=1
    echo "qa: emulator for $device exited before boot; relaunching it once" >&2
    select_avd "$(((${device#emulator-} - emulator_port) / 2))"
    launch_emulator "$(((${device#emulator-} - emulator_port) / 2))" relaunch
    status=0
    python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-boot "$adb" "$device" "$(emulator_pid "$device")" || status=$?
  fi
  return "$status"
}

started_emulator_pids() {
  local pid_file pid
  for pid_file in "$state_dir"/android-emulator-*.pid; do
    [[ -f "$pid_file" ]] || continue
    pid="$(<"$pid_file")"
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] && printf '%s\n' "$pid"
  done
  return 0
}

stop_started_emulators_on_failure() {
  local status=$? pid
  local -a pids
  trap - EXIT
  ((status == 0)) && return 0
  mapfile -t pids < <(started_emulator_pids)
  stop_emulators || true
  for pid in "${pids[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
  done
}

await_emulator() {
  local device="$1"
  if ! wait_for_emulator_boot "$device"; then
    echo "qa: Android emulator $device did not boot; see $state_dir/android-emulator*.log" >&2
    exit 1
  fi
  if ! python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-idle "$adb" "$device"; then
    echo "qa: systemui did not become idle on $device" >&2
    exit 1
  fi
  python3 "$root/mobile/scripts/qa/android-adb.py" --check-health "$adb" "$device" "$state_dir/android-health.log"
  ((android_wifi)) || return 0
  adb_bounded -s "$device" shell cmd connectivity airplane-mode disable >/dev/null
  adb_bounded -s "$device" shell svc wifi enable >/dev/null
  python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-wifi-route "$adb" "$device" || exit 1
}

lock_snapshot() {
  mkdir -p "$avd_dir"
  exec 9>"$avd_dir/baukit-snapshot.lock"
  python3 -c 'import fcntl; fcntl.flock(9, fcntl.LOCK_EX)'
}

delete_stale_snapshot_tags() {
  local serial="$1" listing tag
  listing="$(adb_bounded -s "$serial" emu avd snapshot list)" || return
  while IFS= read -r tag; do
    adb_bounded -s "$serial" emu avd snapshot delete "$tag" >/dev/null || return
    echo "qa: deleted stale qcow2 snapshot tag $tag"
  done < <(printf '%s\n' "$listing" | python3 "$root/mobile/scripts/qa/android-adb.py" --stale-snapshot-tags "$avd_dir" "$snapshot_name" "$index_avd")
}

save_snapshot() {
  local index="$1" serial="${serials[$1]}"
  launch_emulator "$index"
  await_emulator "$serial"
{% if context.auth_enabled %}  warm_chrome "$serial" || exit 1
{% endif %}  python3 "$root/mobile/scripts/qa/android-adb.py" --settle "$adb" "$serial" || exit 1
  python3 "$root/mobile/scripts/qa/android-adb.py" --check-health "$adb" "$serial" "$state_dir/android-health.log"
  adb_bounded -s "$serial" logcat -b all -c >/dev/null
  rm -rf "$snapshot_dir"
  adb_bounded -s "$serial" emu avd snapshot save "$snapshot_name" >/dev/null
  touch "$snapshot_ready"
  echo "qa: saved quick-boot snapshot $snapshot_name"
  python3 "$root/mobile/scripts/qa/android-adb.py" --prune-snapshots "$avd_dir" "$snapshot_name" "$index_avd"
  delete_stale_snapshot_tags "$serial"
}

# Extra AVDs copy the base snapshot instead of cold-booting one after another, so the first
# start saves the base snapshot and stops that emulator before the copies.
prepare_base_snapshot() {
  select_avd 0
  [[ ! -f "$snapshot_ready" ]] || return 0
  lock_snapshot
  if [[ ! -f "$snapshot_ready" ]]; then
    save_snapshot 0
    stop_emulator "${serials[0]}" || exit 1
    rm -f "$state_dir/android-emulator-${serials[0]#emulator-}.pid"
    : > "$state_dir/android-serial"
  fi
  exec 9>&-
}

seed_clone() {
  [[ -f "$snapshot_ready" ]] && return 0
  python3 "$root/mobile/scripts/qa/android-adb.py" --seed-clone "$avd_home/$avd_name.avd" "$avd_dir" "$snapshot_name" ||
    echo "qa: cold-booting $index_avd because $avd_name has no stopped $snapshot_name snapshot to copy"
}

seed_clones() {
  local index
  for ((index = 1; index < emulator_count; index++)); do
    ensure_avd_clone "$index"
    select_avd "$index"
    lock_snapshot
    seed_clone
    exec 9>&-
  done
}

boot_emulators() {
  local index device
  local -A awaited=()
  mkdir -p "$state_dir"
  trap stop_started_emulators_on_failure EXIT
  emulator_features="-Vulkan"
  if ((android_wifi)); then
    emulator_features="$emulator_features,-WiFiPacketStream"
  fi
  mkdir -p "$state_dir/renderer"
  printf '[Processor]\nThreadCount=%s\n[Testing]\nDisableServer=1\n' "$renderer_threads" > "$state_dir/renderer/SwiftShader.ini"
  : > "$state_dir/android-serial"
  printf '%s\n' "$avd_name" > "$state_dir/android-avd-name"
  touch "$state_dir/android-owned"
  if ((emulator_count > 1)); then
    prepare_base_snapshot
    seed_clones
  fi
  for ((index = 0; index < emulator_count; index++)); do
    ((index == 0)) || ensure_avd_clone "$index"
    select_avd "$index"
    if [[ -f "$snapshot_ready" ]]; then
      launch_emulator "$index"
      continue
    fi
    lock_snapshot
    if [[ -f "$snapshot_ready" ]]; then
      launch_emulator "$index"
    else
      save_snapshot "$index"
      awaited[${serials[index]}]=1
    fi
    exec 9>&-
  done
  for device in "${serials[@]}"; do
    [[ -n "${awaited[$device]:-}" ]] || await_emulator "$device"
  done
}

acquire_devices() {
  if broker_enabled; then
    lease_devices
    return
  fi
  reject_foreign_emulators
  python3 "$root/mobile/scripts/qa/android-adb.py" --wait-for-load || exit 1
  boot_emulators
}

case "$action" in
  start)
    stop_all
    mkdir -p "$state_dir"
    bash "$root/mobile/scripts/qa/services.sh" start
    acquire_devices
    prepare_android
    ;;
  boot)
    stop_emulators
    acquire_devices
    echo "qa: Android emulators booted on ${serials[*]}"
    ;;
  prepare)
    ! broker_enabled || mapfile -t serials <"$state_dir/android-serial"
    prepare_android
    ;;
  reverse)
    [[ "${2:-}" =~ ^(emulator-[0-9]+|[A-Za-z0-9.-]+:[0-9]+)$ ]] || { echo "usage: $0 reverse emulator-<port>|<host>:<port>" >&2; exit 2; }
    apply_reverse "$2"
    ;;
esac
