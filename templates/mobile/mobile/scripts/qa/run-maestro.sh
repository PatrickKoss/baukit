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
qa_maestro_args=()
local_hooks="$mobile_dir/scripts/qa/local.sh"
# shellcheck source=/dev/null
[[ ! -f "$local_hooks" ]] || source "$local_hooks"

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
mapfile -t devices < <(grep -v '^$' "$state_file")
[[ -n "${devices[*]:-}" ]] || { echo "qa: $state_file names no device" >&2; exit 1; }
if [[ "$platform" == android ]]; then
  sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
  if [[ -z "$sdk_root" && -f "$mobile_dir/.qa/android-home" ]]; then
    sdk_root="$(<"$mobile_dir/.qa/android-home")"
  fi
  [[ -n "$sdk_root" ]] || { echo "qa: Android SDK location is missing" >&2; exit 1; }
  adb="$sdk_root/platform-tools/adb"
  for device in "${devices[@]}"; do
    python3 "$mobile_dir/scripts/qa/android-adb.py" --check-health "$adb" "$device" "$mobile_dir/.qa/android-health.log"
  done
else
  devices=("${devices[0]}")
fi
cd "$mobile_dir"
mapfile -t flows < <(find .maestro -maxdepth 1 -type f \( -name '*.yaml' -o -name '*.yml' \) ! -name 'config.y*ml' | sort)
[[ -n "${flows[*]:-}" ]] || { echo "qa: mobile/.maestro has no flows" >&2; exit 1; }

all_attached() {
  local device
  for device in "${devices[@]}"; do
    if ! python3 "$mobile_dir/scripts/qa/android-adb.py" --attached "$adb" "$device"; then
      echo "qa: emulator $device died $1" >&2
      return 1
    fi
  done
}

stall_seconds="${BAUKIT_QA_MAESTRO_STALL_SECONDS:-300}"
[[ "$stall_seconds" =~ ^[1-9][0-9]*$ ]] || {
  echo "qa: BAUKIT_QA_MAESTRO_STALL_SECONDS must be a positive integer" >&2
  exit 2
}
mkdir -p "$debug_dir"
maestro_log="$debug_dir/maestro-output.log"

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
  local label="$1" check="$2" last="" now since=$SECONDS pid
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
flow_count="$(count "${flows[@]}")"
shards="$(count "${devices[@]}")"
if ((shards > flow_count)); then
  shards=$flow_count
fi
if ((shards < 2)); then
  for flow in "${flows[@]}"; do
    if [[ "$platform" == android ]]; then
      all_attached "before ${flow#.maestro/}" || exit 1
    fi
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
