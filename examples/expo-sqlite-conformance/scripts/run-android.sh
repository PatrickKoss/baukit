#!/usr/bin/env bash
set -euo pipefail

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=../../../scripts/expo-android-conformance.sh
source "$example_dir/../../scripts/expo-android-conformance.sh"

expo_android_init "$example_dir" dev.baukit.sqliteconformance
expo_android_prepare @baukit/data-contracts @baukit/data-contracts-expo-sqlite
expo_android_boot
expo_android_install
expo_android_start_metro
expo_android_launch
expo_android_await_markers BAUKIT_SQLITE_CONFORMANCE_FAIL BAUKIT_SQLITE_CONFORMANCE_PASS
