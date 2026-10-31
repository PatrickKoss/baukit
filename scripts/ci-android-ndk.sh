#!/usr/bin/env bash
set -euo pipefail

usage() { echo "usage: ci-android-ndk.sh version <expo-project> | install <ndk-version>" >&2; exit 2; }
[[ $# -eq 2 ]] || usage
sdk="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?ANDROID_HOME is required}}"

case "$1" in
  version)
    catalog="$2/node_modules/react-native/gradle/libs.versions.toml"
    version="$(sed -n 's/^ndkVersion = "\(.*\)"$/\1/p' "$catalog")"
    [[ -n "$version" ]] || { echo "ndkVersion is missing from $catalog" >&2; exit 1; }
    echo "version=$version"
    echo "path=$sdk/ndk/$version"
    ;;
  install)
    [[ "${GITHUB_ACTIONS:-}" == "true" ]] || { echo "ci-android-ndk.sh install only runs on GitHub-hosted runners" >&2; exit 2; }
    version="$2"
    for installed in "$sdk"/ndk/*; do
      if [[ -d "$installed" && "$(basename "$installed")" != "$version" ]]; then
        sudo rm -rf "$installed"
      fi
    done
    if [[ ! -f "$sdk/ndk/$version/source.properties" ]]; then
      "$sdk/cmdline-tools/latest/bin/sdkmanager" --sdk_root="$sdk" "ndk;$version"
    fi
    grep -Fqx "Pkg.Revision = $version" "$sdk/ndk/$version/source.properties"
    ;;
  *) usage ;;
esac
