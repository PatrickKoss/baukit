# Shared steps for the Expo device conformance apps under examples/.
# Source it from a run script, then call expo_android_init first.
# shellcheck shell=bash

EMULATOR_EXIT_SECONDS=30
EXPO_ANDROID_SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

expo_android_init() {
  example_dir="$1"
  app_id="$2"
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
  artifacts="$example_dir/artifacts"
  mkdir -p "$artifacts"
  : > "$artifacts/emulator.log"
  : > "$artifacts/metro.log"
  : > "$artifacts/logcat.txt"

  android_serial="${ANDROID_SERIAL:-}"
  if [[ -n "$android_serial" && ! "$android_serial" =~ ^emulator-[0-9]+$ ]]; then
    echo "ANDROID_SERIAL must name an emulator, such as emulator-5554" >&2
    exit 1
  fi
  emulator_pid=""
  docker_emulator=""
  metro_pid=""
  trap expo_android_cleanup EXIT
}

# Invoked indirectly by the EXIT trap.
# shellcheck disable=SC2329
expo_android_cleanup() {
  if [[ -n "$metro_pid" ]]; then kill -- "-$metro_pid" 2>/dev/null || true; fi
  if [[ -n "$docker_emulator" ]]; then
    docker stop "$docker_emulator" >/dev/null 2>&1 || true
  elif [[ -n "$emulator_pid" ]]; then
    expo_adb emu kill >/dev/null 2>&1 || true
  fi
  if [[ -n "$emulator_pid" ]]; then expo_android_reap "$emulator_pid"; fi
}

expo_android_reap() {
  local pid="$1"
  local deadline=$((SECONDS + EMULATOR_EXIT_SECONDS))
  while kill -0 "$pid" 2>/dev/null && (( SECONDS < deadline )); do sleep 1; done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

expo_adb() {
  adb -s "$android_serial" "$@"
}

# Picks the first emulator console port that no attached device uses.
expo_android_pick_serial() {
  [[ -n "$android_serial" ]] && return 0
  local attached port
  attached="$(adb devices 2>/dev/null || true)"
  for (( port = 5554; port <= 5584; port += 2 )); do
    if ! grep -q "^emulator-$port[[:space:]]" <<< "$attached"; then
      android_serial="emulator-$port"
      return 0
    fi
  done
  echo "No free Android emulator port between 5554 and 5584" >&2
  exit 1
}

# Arguments: the workspace packages the app links with file: dependencies.
expo_android_prepare() {
  case "${BAUKIT_ANDROID_PHASE:-all}" in
    run)
      if [[ ! -x "$example_dir/android/gradlew" ]]; then
        echo "Prepare the Android project before running device conformance" >&2
        return 1
      fi
      return 0
      ;;
    all|prepare) ;;
    *)
      echo "BAUKIT_ANDROID_PHASE must be all, prepare, or run" >&2
      return 2
      ;;
  esac

  local filters=()
  local package
  for package in "$@"; do filters+=(--filter "$package"); done

  "$repo_dir/scripts/android-sdk-setup.sh"
  corepack pnpm --dir "$repo_dir/typescript" install --frozen-lockfile
  corepack pnpm --dir "$repo_dir/typescript" "${filters[@]}" run build
  CI=1 corepack pnpm --dir "$example_dir" install --frozen-lockfile
  corepack pnpm --dir "$example_dir" run typecheck
  CI=1 corepack pnpm --dir "$example_dir" exec expo prebuild --clean --platform android
  if [[ "${BAUKIT_ANDROID_PHASE:-all}" == "prepare" ]]; then exit 0; fi
}

expo_android_boot() {
  expo_android_pick_serial
  export ANDROID_SERIAL="$android_serial"
  local emulator_args=(
    -avd "$avd_name"
    -port "${android_serial#emulator-}"
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
    docker_emulator="baukit-conformance-emulator-$$"
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

  local boot_deadline=$((SECONDS + 240))
  until [[ "$(expo_adb get-state 2>/dev/null || true)" == "device" ]] && \
    [[ "$(expo_adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
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
}

expo_android_install() {
  JAVA_TOOL_OPTIONS="${JAVA_TOOL_OPTIONS:+$JAVA_TOOL_OPTIONS }--enable-native-access=ALL-UNNAMED" \
    "$example_dir/android/gradlew" -p "$example_dir/android" --no-daemon --stacktrace --warning-mode all -PreactNativeDevServerIp=127.0.0.1 assembleDebug \
    2>&1 | tee "$artifacts/assemble.log"
  python3 "$EXPO_ANDROID_SCRIPTS/check-gradle-deprecations.py" --project "$example_dir" "$artifacts/assemble.log"
  expo_adb install -r "$example_dir/android/app/build/outputs/apk/debug/app-debug.apk"
  expo_adb reverse tcp:8081 "tcp:$METRO_PORT"
  expo_adb shell "run-as $app_id sh -c 'mkdir -p shared_prefs && printf %s \"<?xml version=\\\"1.0\\\" encoding=\\\"utf-8\\\" standalone=\\\"yes\\\" ?><map><string name=\\\"debug_http_host\\\">localhost:8081</string></map>\" > shared_prefs/${app_id}_preferences.xml'"
}

expo_android_start_metro() {
  CI=1 setsid corepack pnpm --dir "$example_dir" exec expo start --dev-client --port "$METRO_PORT" >"$artifacts/metro.log" 2>&1 &
  metro_pid=$!
  local metro_deadline=$((SECONDS + 120))
  until curl --fail --silent "http://127.0.0.1:$METRO_PORT/status" | grep -q 'packager-status:running'; do
    if (( SECONDS >= metro_deadline )); then
      echo "Expo development server did not start" >&2
      exit 1
    fi
    sleep 2
  done
}

expo_android_launch() {
  expo_adb shell am force-stop "$app_id"
  expo_adb logcat -c
  expo_adb shell monkey -p "$app_id" -c android.intent.category.LAUNCHER 1 >/dev/null
}

# Arguments: the failure marker, then every marker this launch must log.
expo_android_await_markers() {
  local fail_marker="$1"
  shift
  local result_deadline=$((SECONDS + 240))
  local marker missing
  while (( SECONDS < result_deadline )); do
    expo_adb logcat -d -v threadtime > "$artifacts/logcat.txt"
    if grep -Fq "$fail_marker" "$artifacts/logcat.txt"; then
      grep -F "$fail_marker" "$artifacts/logcat.txt" | tail -n 1 >&2
      return 1
    fi
    missing=0
    for marker in "$@"; do
      grep -Fq "$marker" "$artifacts/logcat.txt" || missing=1
    done
    if (( missing == 0 )); then
      for marker in "$@"; do grep -F "$marker" "$artifacts/logcat.txt" | tail -n 1; done
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for: $*" >&2
  return 1
}
