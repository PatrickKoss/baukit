#!/usr/bin/env bash
set -euo pipefail

mobile_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state_dir="$mobile_dir/.qa"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/Sdk}}"
avd_home="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
avd_name="${BAUKIT_QA_ANDROID_AVD:-{{ context.app_name }}-qa}"
api_level="${BAUKIT_QA_ANDROID_API_LEVEL:-36}"
command_tools_version="13114758"

case "$(uname -m)" in
  arm64 | aarch64) default_architecture=arm64-v8a ;;
  *) default_architecture=x86_64 ;;
esac
architecture="${BAUKIT_QA_ANDROID_ARCHITECTURE:-$default_architecture}"

case "$(uname -s)" in
  Linux) command_tools_platform=linux ;;
  Darwin) command_tools_platform=mac ;;
  *)
    echo "qa: Android setup supports Linux and macOS" >&2
    exit 1
    ;;
esac

command_tools_url="https://dl.google.com/android/repository/commandlinetools-${command_tools_platform}-${command_tools_version}_latest.zip"
sdkmanager="$sdk_root/cmdline-tools/latest/bin/sdkmanager"

if [[ ! -x "$sdkmanager" ]]; then
  command -v curl >/dev/null || { echo "qa: curl is required" >&2; exit 1; }
  command -v unzip >/dev/null || { echo "qa: unzip is required" >&2; exit 1; }
  download_dir="$(mktemp -d)"
  trap 'rm -rf "$download_dir"' EXIT
  curl --fail --location --retry 3 \
    --output "$download_dir/command-line-tools.zip" "$command_tools_url"
  unzip -q "$download_dir/command-line-tools.zip" -d "$download_dir/unpacked"
  mkdir -p "$sdk_root/cmdline-tools/latest"
  cp -R "$download_dir/unpacked/cmdline-tools/." "$sdk_root/cmdline-tools/latest/"
fi

export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_AVD_HOME="$avd_home"
export PATH="$sdk_root/cmdline-tools/latest/bin:$sdk_root/emulator:$sdk_root/platform-tools:$PATH"

set +o pipefail
yes | sdkmanager --sdk_root="$sdk_root" --licenses >/dev/null
license_status=$?
set -o pipefail
if [[ $license_status -ne 0 ]]; then
  echo "qa: Android SDK license acceptance failed" >&2
  exit "$license_status"
fi

system_image="system-images;android-${api_level};google_apis;${architecture}"
sdkmanager --sdk_root="$sdk_root" \
  platform-tools \
  emulator \
  "platforms;android-${api_level}" \
  "build-tools;${api_level}.0.0" \
  "$system_image"

mkdir -p "$avd_home"
avd_config="$avd_home/$avd_name.avd/config.ini"
if [[ -f "$avd_config" ]] && ! grep -Fq "$architecture" "$avd_config"; then
  echo "qa: replacing '$avd_name' because its system image does not match $architecture"
  avdmanager delete avd --name "$avd_name" >/dev/null
fi
if ! avdmanager list avd 2>/dev/null | grep -Fq "Name: $avd_name"; then
  echo no | avdmanager create avd \
    --force \
    --name "$avd_name" \
    --package "$system_image" \
    --device pixel_7 >/dev/null
fi

if [[ -f "$avd_config" ]]; then
  python3 - "$avd_config" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = [
    line
    for line in path.read_text().splitlines()
    if not line.startswith(("hw.ramSize=", "hw.keyboard="))
]
lines.extend(("hw.ramSize=4096", "hw.keyboard=yes"))
path.write_text("\n".join(lines) + "\n")
PY
fi

mkdir -p "$state_dir"
printf '%s\n' "$sdk_root" > "$state_dir/android-home"
printf '%s\n' "$avd_home" > "$state_dir/android-avd-home"
printf '%s\n' "$architecture" > "$state_dir/android-architecture"
echo "qa: Android SDK at $sdk_root, AVD '$avd_name' ready for $architecture"
