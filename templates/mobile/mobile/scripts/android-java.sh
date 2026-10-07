#!/usr/bin/env bash
set -euo pipefail

[[ $# -gt 0 ]] || { echo "usage: android-java.sh <command> [args...]" >&2; exit 2; }
export JAVA_TOOL_OPTIONS="${JAVA_TOOL_OPTIONS:+$JAVA_TOOL_OPTIONS }--enable-native-access=ALL-UNNAMED"
exec "$@"
