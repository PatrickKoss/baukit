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
    "$maestro_bin" test \
      --platform "$platform" \
      --device "${devices[0]}" \
      --env "APP_ID=$app_id" \
      --debug-output "$debug_dir" \
      "$flow"
  done
  exit 0
fi

devices=("${devices[@]:0:shards}")
device_list="$(IFS=,; printf '%s' "${devices[*]}")"
echo "qa: sharding $flow_count flows across $device_list"
"$maestro_bin" test \
  --platform "$platform" \
  --env "APP_ID=$app_id" \
  --debug-output "$debug_dir" \
  --device "$device_list" \
  --shard-split "$shards" \
  "${flows[@]}" &
maestro_pid=$!
while kill -0 "$maestro_pid" 2>/dev/null; do
  if ! all_attached "during the sharded run"; then
    kill "$maestro_pid" 2>/dev/null || true
    wait "$maestro_pid" || true
    exit 1
  fi
  sleep "${BAUKIT_QA_POLL_SECONDS:-5}"
done
wait "$maestro_pid"
