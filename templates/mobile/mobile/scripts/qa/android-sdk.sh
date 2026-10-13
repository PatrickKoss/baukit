#!/usr/bin/env bash
set -euo pipefail

mobile_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state_dir="$mobile_dir/.qa"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/Sdk}}"
avd_home="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
avd_name="${BAUKIT_QA_ANDROID_AVD:-{{ context.app_name }}-qa}"
api_level="${BAUKIT_QA_ANDROID_API_LEVEL:-36}"
image_tag="${BAUKIT_QA_ANDROID_IMAGE_TAG:-google_apis}"
command_tools_version="16111833"
avd_overrides="${BAUKIT_QA_AVD_CONFIG:-}"

if [[ -n "$avd_overrides" ]]; then
  python3 - "$avd_overrides" <<'PYCONFIG'
import re
import sys

for entry in sys.argv[1].split():
    if not re.fullmatch(r"[A-Za-z0-9_.-]+=[^\s=]+", entry):
        print("qa: BAUKIT_QA_AVD_CONFIG must be a whitespace-separated list of key=value entries", file=sys.stderr)
        raise SystemExit(2)
PYCONFIG
fi

case "$(uname -m)" in
  arm64 | aarch64) default_architecture=arm64-v8a ;;
  *) default_architecture=x86_64 ;;
esac
architecture="${BAUKIT_QA_ANDROID_ARCHITECTURE:-$default_architecture}"

case "$(uname -s)" in
  Linux) command_tools_platform=linux ;;
  Darwin)
    case "$(uname -m)" in
      arm64 | aarch64) command_tools_platform=mac_arm64 ;;
      *) command_tools_platform=mac_x86_64 ;;
    esac
    ;;
  *)
    echo "qa: Android setup supports Linux and macOS" >&2
    exit 1
    ;;
esac

command_tools_url="https://dl.google.com/android/repository/commandlinetools-${command_tools_platform}-${command_tools_version}_latest.zip"
sdkmanager="$sdk_root/cmdline-tools/$command_tools_version/bin/sdkmanager"

if [[ ! -x "$sdkmanager" ]]; then
  command -v curl >/dev/null || { echo "qa: curl is required" >&2; exit 1; }
  command -v unzip >/dev/null || { echo "qa: unzip is required" >&2; exit 1; }
  download_dir="$(mktemp -d)"
  trap 'rm -rf "$download_dir"' EXIT
  curl --fail --location --retry 3 \
    --output "$download_dir/command-line-tools.zip" "$command_tools_url"
  unzip -q "$download_dir/command-line-tools.zip" -d "$download_dir/unpacked"
  mkdir -p "$sdk_root/cmdline-tools/$command_tools_version"
  cp -R "$download_dir/unpacked/cmdline-tools/." "$sdk_root/cmdline-tools/$command_tools_version/"
fi

export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_AVD_HOME="$avd_home"
export PATH="$sdk_root/cmdline-tools/$command_tools_version/bin:$sdk_root/emulator:$sdk_root/platform-tools:$PATH"

set +o pipefail
yes | sdkmanager --sdk_root="$sdk_root" --licenses >/dev/null
license_status=$?
set -o pipefail
if [[ $license_status -ne 0 ]]; then
  echo "qa: Android SDK license acceptance failed" >&2
  exit "$license_status"
fi

system_image="system-images;android-${api_level};${image_tag};${architecture}"
sdkmanager --sdk_root="$sdk_root" \
  platform-tools \
  emulator \
  "platforms;android-${api_level}" \
  "build-tools;${api_level}.0.0" \
  "$system_image"

mkdir -p "$avd_home"
avd_config="$avd_home/$avd_name.avd/config.ini"
image_directory="system-images/android-${api_level}/${image_tag}/${architecture}/"
if [[ -f "$avd_config" ]]; then
  replacement_reason="$(python3 - "$avd_config" "$image_directory" "${BAUKIT_QA_AVD_CONFIG+x}" <<'PYIMAGE'
from pathlib import Path
import sys

settings = {
    key.strip(): value.strip()
    for line in Path(sys.argv[1]).read_text().splitlines()
    if "=" in line
    for key, value in [line.split("=", 1)]
}
if settings.get("image.sysdir.1") != sys.argv[2]:
    print("its system image does not match the requested image")
elif settings.get("hw.device.name") != "pixel_6":
    print("its device is not pixel_6")
elif not sys.argv[3] and all(
    settings.get(key) == value
    for key, value in {"hw.ramSize": "4096", "hw.keyboard": "yes", "hw.cpu.ncore": "4"}.items()
):
    print("it still has the old forced RAM, keyboard, and CPU settings")
PYIMAGE
  )"
  if [[ -n "$replacement_reason" ]]; then
    echo "qa: replacing '$avd_name' because $replacement_reason"
    avdmanager delete avd --name "$avd_name" >/dev/null
  fi
fi
avd_list="$(avdmanager list avd)"
if ! grep -Fq "Name: $avd_name" <<< "$avd_list"; then
  avdmanager create avd \
    --force \
    --name "$avd_name" \
    --package "$system_image" \
    --device pixel_6 >/dev/null <<< no
fi

if [[ -n "$avd_overrides" && -f "$avd_config" ]]; then
  python3 - "$avd_config" "$avd_overrides" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
overrides = dict(entry.split("=", 1) for entry in sys.argv[2].split())
lines = [
    line
    for line in path.read_text().splitlines()
    if line.partition("=")[0].strip() not in overrides
]
lines.extend(f"{key}={value}" for key, value in overrides.items())
path.write_text("\n".join(lines) + "\n")
PY
fi

mkdir -p "$state_dir"
printf '%s\n' "$sdk_root" > "$state_dir/android-home"
printf '%s\n' "$avd_home" > "$state_dir/android-avd-home"
printf '%s\n' "$architecture" > "$state_dir/android-architecture"
echo "qa: Android SDK at $sdk_root, AVD '$avd_name' ready for $architecture"
