#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
mobile_dir="$root/mobile"
platform="${1:-}"
state_dir="${BAUKIT_QA_STATE_DIR:-$mobile_dir/.qa}"
api_port="${BAUKIT_QA_API_PORT:-18080}"
{% if context.auth_enabled %}keycloak_port="${BAUKIT_QA_KEYCLOAK_PORT:-18081}"
{% endif %}maestro_bin="$(command -v maestro || true)"
[[ -n "$maestro_bin" ]] || {
  echo "qa: Maestro is required; install it from https://docs.maestro.dev/getting-started/installing-maestro" >&2
  exit 1
}
[[ -d "$mobile_dir/.maestro" ]] || { echo "qa: mobile/.maestro is missing" >&2; exit 1; }
qa_maestro_args=()
qa_maestro_shards=()
qa_maestro_env() { :; }
qa_maestro_flows() { :; }
qa_ios_bundle_id="${BAUKIT_QA_IOS_BUNDLE_ID:-dev.baukit.{{ context.app_name }}}"
qa_app_id="${BAUKIT_QA_APP_ID:-dev.baukit.{{ context.app_crate }}}"
local_hooks="$mobile_dir/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"
export BAUKIT_QA_API_PORT="$api_port"
{% if context.auth_enabled %}export BAUKIT_QA_KEYCLOAK_PORT="$keycloak_port"
{% endif %}
case "$platform" in
  android)
    state_file="$state_dir/android-serial"
    app_id="$qa_app_id"
    debug_dir="$mobile_dir/maestro-debug"
    ;;
  ios)
    state_file="$state_dir/ios-udid"
    app_id="$qa_ios_bundle_id"
    debug_dir="$mobile_dir/maestro-debug-ios"
    ;;
  *)
    echo "usage: $0 [android|ios]" >&2
    exit 2
    ;;
esac

stall_seconds="${BAUKIT_QA_MAESTRO_STALL_SECONDS:-300}"
[[ "$stall_seconds" =~ ^[1-9][0-9]*$ ]] || {
  echo "qa: BAUKIT_QA_MAESTRO_STALL_SECONDS must be a positive integer" >&2
  exit 2
}
deadline_seconds="${BAUKIT_QA_MAESTRO_DEADLINE_SECONDS:-}"
[[ -z "$deadline_seconds" || "$deadline_seconds" =~ ^[1-9][0-9]*$ ]] || {
  echo "qa: BAUKIT_QA_MAESTRO_DEADLINE_SECONDS must be a positive integer" >&2
  exit 2
}

[[ -f "$state_file" ]] || { echo "qa: no live $platform QA environment; start it first" >&2; exit 1; }
mapfile -t devices < <(grep -v '^$' "$state_file")
[[ -n "${devices[*]:-}" ]] || { echo "qa: $state_file names no device" >&2; exit 1; }
if [[ "$platform" == android ]]; then
  sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
  if [[ -z "$sdk_root" && -f "$state_dir/android-home" ]]; then
    sdk_root="$(<"$state_dir/android-home")"
  fi
  [[ -n "$sdk_root" ]] || { echo "qa: Android SDK location is missing" >&2; exit 1; }
  adb="$sdk_root/platform-tools/adb"
  for device in "${devices[@]}"; do
    python3 "$mobile_dir/scripts/qa/android-adb.py" --check-health "$adb" "$device" "$state_dir/android-health.log"
  done
else
  devices=("${devices[0]}")
fi
cd "$mobile_dir"

evidence_dir=""
logcat_pids=()

fail_device() {
  mark_device_failed "$1"
  return "$2"
}

mark_device_failed() { [[ -z "$evidence_dir" ]] || : > "$evidence_dir/$1.failed"; }

start_device_evidence() {
  local device
  [[ "$platform" == android ]] || return 0
  evidence_dir="$(mktemp -d "$state_dir/logcat.XXXXXX")"
  for device in "${devices[@]}"; do
    python3 "$mobile_dir/scripts/qa/android-adb.py" "$adb" -s "$device" shell dumpsys package com.android.chrome \
      2>&1 | awk -v device="$device" '/versionName=/ {print "chrome " $0; found=1; exit} END {if (!found) print "chrome versionName not found on " device}' \
      > "$evidence_dir/$device.chrome" || true
    python3 "$mobile_dir/scripts/qa/android-adb.py" "$adb" -s "$device" logcat -c || true
    : > "$evidence_dir/$device.logcat"
    "$adb" -s "$device" logcat -v threadtime >> "$evidence_dir/$device.logcat" 2>&1 &
    logcat_pids+=("$!")
  done
}

finish_device_evidence() {
  local status="$1" pid device
  [[ -n "$evidence_dir" ]] || return 0
  for pid in "${logcat_pids[@]}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  if ((status != 0)) && [[ -z "$(find "$evidence_dir" -name '*.failed' -print -quit)" ]]; then
    for device in "${devices[@]}"; do mark_device_failed "$device"; done
  fi
  for device in "${devices[@]}"; do
    [[ ! -f "$evidence_dir/$device.failed" ]] || {
      mkdir -p "$debug_dir"
      mv "$evidence_dir/$device.logcat" "$debug_dir/logcat-$device.txt"
      mv "$evidence_dir/$device.chrome" "$debug_dir/chrome-version-$device.txt"
      echo "qa: kept $debug_dir/logcat-$device.txt" >&2
    }
  done
  rm -rf "$evidence_dir"
}

finish_run() {
  local status=$?
  trap - EXIT
  finish_device_evidence "$status"
  exit "$status"
}

trap finish_run EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
start_device_evidence

all_attached() {
  local device
  for device in "${devices[@]}"; do
    if ! python3 "$mobile_dir/scripts/qa/android-adb.py" --attached "$adb" "$device"; then
      echo "qa: emulator $device died $1" >&2
      return 1
    fi
  done
}

prepare_flow_device() {
  local device="$1" flow="$2"
  [[ "$platform" == android ]] || return 0
  if ! python3 "$mobile_dir/scripts/qa/android-adb.py" --attached "$adb" "$device"; then
    echo "qa: emulator $device died before $flow" >&2
    return 1
  fi
  bash "$mobile_dir/scripts/qa/android-env.sh" reverse "$device"
}

progress() {
  { wc -c < "$maestro_log"; find "$debug_dir" -type f ! -name maestro-output.log -exec ls -ln {} + 2>/dev/null; } | cksum
}

stop_maestro() {
  kill -TERM -- "-$1" 2>/dev/null || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    kill -0 "$1" 2>/dev/null || break
    sleep 1
  done
  kill -KILL -- "-$1" 2>/dev/null || true
  wait "$1" 2>/dev/null || true
}

run_maestro() {
  local label="$1" check="$2" last="" now since=$SECONDS started=$SECONDS pid
  shift 2
  : > "$maestro_log"
  set -m
  "$maestro_bin" "$@" > >(tee -a "$maestro_log") 2>&1 &
  pid=$!
  set +m
  while kill -0 "$pid" 2>/dev/null; do
    if [[ -n "$check" ]] && ! all_attached "$check"; then
      stop_maestro "$pid"
      return 1
    fi
    if [[ -n "$deadline_seconds" ]] && ((SECONDS - started > deadline_seconds)); then
      echo "qa: $label exceeded $deadline_seconds s; stopped it and kept $debug_dir" >&2
      stop_maestro "$pid"
      return 124
    fi
    now="$(progress)"
    if [[ "$now" != "$last" ]]; then
      last="$now"
      since=$SECONDS
    elif ((SECONDS - since > stall_seconds)); then
      echo "qa: $label made no progress for $stall_seconds s; stopped it and kept $debug_dir" >&2
      stop_maestro "$pid"
      return 124
    fi
    sleep "${BAUKIT_QA_POLL_SECONDS:-5}"
  done
  wait "$pid"
}

count() { echo "$#"; }

run_declared_shard() {
  local index="$1" debug_dir="$2/shard-$1" maestro_log
  qa_maestro_env "$index"
  mkdir -p "$debug_dir"
  maestro_log="$debug_dir/maestro-output.log"
  prepare_flow_device "${devices[index]}" "${qa_maestro_shards[index]}" || fail_device "${devices[index]}" "$?"
  run_maestro "${qa_maestro_shards[index]}" "" test \
    --platform "$platform" \
    --device "${devices[index]}" \
    --env "APP_ID=$app_id" \
    --debug-output "$debug_dir" \
    "${qa_maestro_args[@]}" \
    "${qa_maestro_shards[index]}" || fail_device "${devices[index]}" "$?"
}

run_declared_shards() {
  local index pid status=0 pids=() shard_count device_count
  shard_count="$(count "${qa_maestro_shards[@]}")"
  device_count="$(count "${devices[@]}")"
  if ((shard_count > device_count)); then
    echo "qa: $shard_count Maestro shards but only $device_count devices" >&2
    return 1
  fi
  for index in "${!qa_maestro_shards[@]}"; do
    run_declared_shard "$index" "$debug_dir" &
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do
    wait "$pid" || status=$?
  done
  return "$status"
}

run_device_flows() {
  local index="$1" flow device_flows debug_dir="$2/shard-$1" maestro_log
  mapfile -t device_flows < <(qa_maestro_flows "$index")
  qa_maestro_env "$index"
  mkdir -p "$debug_dir"
  maestro_log="$debug_dir/maestro-output.log"
  for flow in "${device_flows[@]}"; do
    prepare_flow_device "${devices[index]}" "$flow" || fail_device "${devices[index]}" "$?"
    run_maestro "$flow" "" test \
      --platform "$platform" \
      --device "${devices[index]}" \
      --env "APP_ID=$app_id" \
      --debug-output "$debug_dir" \
      "${qa_maestro_args[@]}" \
      "$flow" || fail_device "${devices[index]}" "$?"
  done
}

run_hooked_flows() {
  local index pid status=0 pids=()
  for index in "${!devices[@]}"; do
    run_device_flows "$index" "$debug_dir" &
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do
    wait "$pid" || status=$?
  done
  return "$status"
}

hooked_flows_declared() {
  local index
  for index in "${!devices[@]}"; do
    [[ -z "$(qa_maestro_flows "$index")" ]] || return 0
  done
  return 1
}

if hooked_flows_declared; then
  [[ -z "${qa_maestro_shards[*]:-}" ]] || {
    echo "qa: set qa_maestro_flows or qa_maestro_shards, not both" >&2
    exit 1
  }
  run_hooked_flows
  exit $?
fi
if [[ -n "${qa_maestro_shards[*]:-}" ]]; then
  run_declared_shards
  exit $?
fi
qa_maestro_env 0
mkdir -p "$debug_dir"
maestro_log="$debug_dir/maestro-output.log"
cd "$mobile_dir"
mapfile -t flows < <(find .maestro -maxdepth 1 -type f \( -name '*.yaml' -o -name '*.yml' \) ! -name 'config.y*ml' | sort)
[[ -n "${flows[*]:-}" ]] || { echo "qa: mobile/.maestro has no flows" >&2; exit 1; }

flow_count="$(count "${flows[@]}")"
shards="$(count "${devices[@]}")"
if ((shards > flow_count)); then
  shards=$flow_count
fi
if ((shards < 2)); then
  for flow in "${flows[@]}"; do
    prepare_flow_device "${devices[0]}" "${flow#.maestro/}" || exit 1
    run_maestro "${flow#.maestro/}" "" test \
      --platform "$platform" \
      --device "${devices[0]}" \
      --env "APP_ID=$app_id" \
      --debug-output "$debug_dir" \
      "${qa_maestro_args[@]}" \
      "$flow"
  done
  exit 0
fi

devices=("${devices[@]:0:shards}")
device_list="$(IFS=,; printf '%s' "${devices[*]}")"
echo "qa: sharding $flow_count flows across $device_list"
run_maestro "the sharded run" "during the sharded run" test \
  --platform "$platform" \
  --env "APP_ID=$app_id" \
  --debug-output "$debug_dir" \
  --device "$device_list" \
  --shard-split "$shards" \
  "${qa_maestro_args[@]}" \
  "${flows[@]}"
