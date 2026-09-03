#!/usr/bin/env bash
set -euo pipefail

# Expo SDK 57 compiles against Android API 36. Keep this list intentionally
# small so local and CI native gates install the same supported baseline.
ANDROID_SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/Sdk}}"
ANDROID_HOME="$ANDROID_SDK_ROOT"
ANDROID_AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
export ANDROID_AVD_HOME ANDROID_HOME ANDROID_SDK_ROOT

command_tools_version="13114758"
command_tools_url="https://dl.google.com/android/repository/commandlinetools-linux-${command_tools_version}_latest.zip"
command_tools="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager"
avd_name="${BAUKIT_ANDROID_AVD:-baukit-api-36}"

if [[ ! -x "$command_tools" ]]; then
  command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }
  command -v unzip >/dev/null || { echo "unzip is required" >&2; exit 1; }
  download_dir="$(mktemp -d)"
  trap 'rm -rf "$download_dir"' EXIT
  curl --fail --location --retry 3 --output "$download_dir/command-line-tools.zip" "$command_tools_url"
  unzip -q "$download_dir/command-line-tools.zip" -d "$download_dir/unpacked"
  mkdir -p "$ANDROID_SDK_ROOT/cmdline-tools/latest"
  cp -R "$download_dir/unpacked/cmdline-tools/." "$ANDROID_SDK_ROOT/cmdline-tools/latest/"
fi

export PATH="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/emulator:$ANDROID_SDK_ROOT/platform-tools:$PATH"

# sdkmanager closes stdin after the final license; yes then exits with SIGPIPE.
set +o pipefail
yes | sdkmanager --sdk_root="$ANDROID_SDK_ROOT" --licenses >/dev/null
license_status=$?
set -o pipefail
if [[ $license_status -ne 0 ]]; then
  echo "Android SDK license acceptance failed" >&2
  exit "$license_status"
fi

sdkmanager --sdk_root="$ANDROID_SDK_ROOT" \
  "platform-tools" \
  "emulator" \
  "platforms;android-36" \
  "build-tools;36.0.0" \
  "system-images;android-36;google_apis;x86_64"

mkdir -p "$ANDROID_AVD_HOME"
if [[ ! -f "$ANDROID_AVD_HOME/$avd_name.ini" ]]; then
  echo "no" | avdmanager create avd \
    --force \
    --name "$avd_name" \
    --package "system-images;android-36;google_apis;x86_64" \
    --device "pixel_6"
fi

if [[ ! -f "$ANDROID_AVD_HOME/$avd_name.ini" ]]; then
  echo "Android AVD was not created at $ANDROID_AVD_HOME/$avd_name.ini" >&2
  exit 1
fi

cat <<EOF
Android SDK is ready.
export ANDROID_HOME="$ANDROID_HOME"
export ANDROID_SDK_ROOT="$ANDROID_SDK_ROOT"
export ANDROID_AVD_HOME="$ANDROID_AVD_HOME"
export PATH="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/emulator:$ANDROID_SDK_ROOT/platform-tools:\$PATH"
AVD: $avd_name
EOF
