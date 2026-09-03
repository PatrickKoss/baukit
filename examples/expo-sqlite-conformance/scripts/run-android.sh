#!/usr/bin/env bash
set -euo pipefail

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_dir="$(cd "$example_dir/../.." && pwd)"
ANDROID_SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/Sdk}}"
ANDROID_HOME="$ANDROID_SDK_ROOT"
ANDROID_AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
export ANDROID_AVD_HOME ANDROID_HOME ANDROID_SDK_ROOT
export PATH="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/emulator:$ANDROID_SDK_ROOT/platform-tools:$PATH"

avd_name="${BAUKIT_ANDROID_AVD:-baukit-api-36}"
METRO_PORT="${METRO_PORT:-8081}"
if [[ ! "$METRO_PORT" =~ ^[1-9][0-9]{0,4}$ ]] || (( METRO_PORT > 65535 )); then
  echo "METRO_PORT must be an integer between 1 and 65535" >&2
  exit 1
fi
app_id="dev.baukit.sqliteconformance"
artifacts="$example_dir/artifacts"
mkdir -p "$artifacts"
: > "$artifacts/emulator.log"
: > "$artifacts/metro.log"
: > "$artifacts/logcat.txt"

emulator_pid=""
docker_emulator=""
metro_pid=""
# Invoked indirectly by the EXIT trap.
# shellcheck disable=SC2329
cleanup() {
  if [[ -n "$metro_pid" ]]; then kill "$metro_pid" 2>/dev/null || true; fi
  if [[ -n "$docker_emulator" ]]; then
    docker stop "$docker_emulator" >/dev/null 2>&1 || true
  else
    adb emu kill >/dev/null 2>&1 || true
  fi
  if [[ -n "$emulator_pid" ]]; then wait "$emulator_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT

"$repo_dir/scripts/android-sdk-setup.sh"
corepack pnpm --dir "$repo_dir/typescript" install --frozen-lockfile
corepack pnpm --dir "$repo_dir/typescript" \
  --filter @baukit/data-contracts \
  --filter @baukit/data-contracts-expo-sqlite \
  run build
corepack pnpm --dir "$example_dir" install --frozen-lockfile
corepack pnpm --dir "$example_dir" run typecheck
CI=1 corepack pnpm --dir "$example_dir" exec expo prebuild --clean --platform android

emulator_args=(
  -avd "$avd_name"
  -no-window
  -no-audio
  -no-boot-anim
  -no-snapshot
  -no-metrics
  -gpu swiftshader_indirect
  -accel on
)

if [[ -r /dev/kvm && -w /dev/kvm ]]; then
  "$ANDROID_SDK_ROOT/emulator/emulator" \
    "${emulator_args[@]}" \
    -wipe-data >"$artifacts/emulator.log" 2>&1 &
elif command -v docker >/dev/null && docker info >/dev/null 2>&1; then
  # Some WSL/Linux hosts expose KVM without granting the current login group
  # access. A short-lived container keeps the current uid but selects KVM's
  # gid, avoiding chmod, setfacl, group edits, or other host-system mutation.
  docker_emulator="baukit-sqlite-emulator-$$"
  docker run --rm \
    --name "$docker_emulator" \
    --device /dev/kvm \
    --user "$(id -u):$(stat -c %g /dev/kvm)" \
    --network host \
    -v "$ANDROID_SDK_ROOT:$ANDROID_SDK_ROOT:ro" \
    -v "$HOME/.android:$HOME/.android" \
    -v /usr/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:ro \
    -e HOME="$HOME" \
    -e ANDROID_SDK_ROOT="$ANDROID_SDK_ROOT" \
    -e ANDROID_AVD_HOME="$ANDROID_AVD_HOME" \
    ubuntu:24.04 \
    "$ANDROID_SDK_ROOT/emulator/emulator" \
    "${emulator_args[@]}" \
    -read-only >"$artifacts/emulator.log" 2>&1 &
else
  echo "/dev/kvm is not accessible and no Docker fallback is available" >&2
  exit 1
fi
emulator_pid=$!

boot_deadline=$((SECONDS + 240))
until [[ "$(adb get-state 2>/dev/null || true)" == "device" ]] && \
  [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
  if ! kill -0 "$emulator_pid" 2>/dev/null; then
    echo "Android emulator exited before boot completed" >&2
    exit 1
  fi
  if (( SECONDS >= boot_deadline )); then
    echo "Android emulator did not finish booting" >&2
    exit 1
  fi
  sleep 2
done

"$example_dir/android/gradlew" -p "$example_dir/android" --no-daemon --stacktrace assembleDebug
adb install -r "$example_dir/android/app/build/outputs/apk/debug/app-debug.apk"
adb reverse tcp:8081 "tcp:$METRO_PORT"
adb shell "run-as $app_id sh -c 'mkdir -p shared_prefs && printf %s \"<?xml version=\\\"1.0\\\" encoding=\\\"utf-8\\\" standalone=\\\"yes\\\" ?><map><string name=\\\"debug_http_host\\\">localhost:8081</string></map>\" > shared_prefs/${app_id}_preferences.xml'"
adb logcat -c

CI=1 corepack pnpm --dir "$example_dir" exec expo start --dev-client --port "$METRO_PORT" >"$artifacts/metro.log" 2>&1 &
metro_pid=$!
metro_deadline=$((SECONDS + 120))
until curl --fail --silent "http://127.0.0.1:$METRO_PORT/status" | grep -q 'packager-status:running'; do
  if (( SECONDS >= metro_deadline )); then
    echo "Expo development server did not start" >&2
    exit 1
  fi
  sleep 2
done

adb shell am force-stop "$app_id"
adb shell monkey -p "$app_id" -c android.intent.category.LAUNCHER 1 >/dev/null

result_deadline=$((SECONDS + 240))
while (( SECONDS < result_deadline )); do
  adb logcat -d -v threadtime > "$artifacts/logcat.txt"
  if grep -Fq 'BAUKIT_SQLITE_CONFORMANCE_PASS' "$artifacts/logcat.txt"; then
    grep -F 'BAUKIT_SQLITE_CONFORMANCE_PASS' "$artifacts/logcat.txt" | tail -n 1
    exit 0
  fi
  if grep -Fq 'BAUKIT_SQLITE_CONFORMANCE_FAIL' "$artifacts/logcat.txt"; then
    grep -F 'BAUKIT_SQLITE_CONFORMANCE_FAIL' "$artifacts/logcat.txt" | tail -n 1 >&2
    exit 1
  fi
  sleep 2
done

echo "Timed out waiting for the Expo SQLite conformance marker" >&2
exit 1
