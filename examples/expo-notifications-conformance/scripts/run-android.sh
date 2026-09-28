#!/usr/bin/env bash
set -euo pipefail

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=../../../scripts/expo-android-conformance.sh
source "$example_dir/../../scripts/expo-android-conformance.sh"

app_id="dev.baukit.notificationsconformance"
permission="android.permission.POST_NOTIFICATIONS"
fail_marker="BAUKIT_NOTIFICATIONS_CONFORMANCE_FAIL"

expo_android_init "$example_dir" "$app_id"
expo_android_prepare @baukit/localization-core @baukit/notifications-core @baukit/notifications-expo
expo_android_boot
expo_android_install
expo_android_start_metro

adb shell pm grant "$app_id" "$permission"
expo_android_launch
expo_android_await_markers "$fail_marker" BAUKIT_HERMES_VECTORS_PASS BAUKIT_NOTIFICATIONS_GRANTED_PASS
cp "$artifacts/logcat.txt" "$artifacts/logcat-granted.txt"

# user-fixed makes the runtime request answer "denied" without showing a prompt.
adb shell am force-stop "$app_id"
adb shell pm revoke "$app_id" "$permission"
adb shell pm set-permission-flags "$app_id" "$permission" user-fixed
expo_android_launch
expo_android_await_markers "$fail_marker" BAUKIT_HERMES_VECTORS_PASS BAUKIT_NOTIFICATIONS_DENIED_PASS
