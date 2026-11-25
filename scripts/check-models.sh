#!/bin/sh
# Model-checks every models/<name>/<Config>.cfg with TLC and fails on any violation.
#
# The spec is <Config>.tla next to the cfg, or the only .tla in that directory. TLC checks for
# deadlock unless the cfg has a line `\* tlc-args: -deadlock` (any TLC flags go there), which
# suits specs whose terminal states are intended.
# TLA2TOOLS_JAR overrides the jar; by default this installs the deploy/tools.toml pin into .tools/bin.
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

if [ -n "${TLA2TOOLS_JAR:-}" ]; then
  jar=$TLA2TOOLS_JAR
else
  scripts/install-deploy-tools.sh --only tla2tools
  jar=${BAUKIT_TOOLS_DIR:-$repository_root/.tools}/bin/tla2tools
fi
if [ ! -f "$jar" ]; then
  echo "check-models: TLA2TOOLS_JAR=$jar does not exist" >&2
  exit 2
fi

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

spec_for() {
  directory=$(dirname -- "$1")
  named="$directory/$(basename -- "$1" .cfg).tla"
  if [ -f "$named" ]; then
    printf '%s\n' "$named"
    return
  fi
  set -- "$directory"/*.tla
  if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    echo "check-models: no spec for $config (want $named or one .tla in $directory)" >&2
    return 1
  fi
  printf '%s\n' "$1"
}

checked=0
failed=''
for config in models/*/*.cfg; do
  [ -f "$config" ] || continue
  spec=$(spec_for "$config")
  args=$(sed -n 's/^\\\* tlc-args:[[:space:]]*//p' "$config" | head -n 1)
  echo "check-models: $config"
  # shellcheck disable=SC2086 # tlc-args holds space-separated TLC flags
  if ! java -XX:+UseParallelGC -cp "$jar" tlc2.TLC -workers auto -cleanup -noGenerateSpecTE \
    -metadir "$scratch/$checked" -config "$config" $args "$spec"; then
    failed="$failed $config"
  fi
  checked=$((checked + 1))
done

if [ -n "$failed" ]; then
  echo "check-models: violations in:$failed" >&2
  exit 1
fi
echo "check-models: $checked configurations passed"
