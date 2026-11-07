#!/bin/sh
# Usage: sh scripts/fuzz.sh [target|all] [seconds]
# Runs each cargo-fuzz target in rust/fuzz for a fixed time and fails on any crash.
set -eu

DEFAULT_SECONDS=60
target=${1:-all}
seconds=${2:-$DEFAULT_SECONDS}

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fuzz_dir="$root/rust/fuzz"

case $seconds in
  '' | *[!0-9]*)
    echo "fuzz.sh: seconds must be a positive integer, got '$seconds'" >&2
    exit 2
    ;;
esac

cd "$fuzz_dir"

if [ "$target" = all ]; then
  targets=$(cargo +nightly fuzz list)
else
  targets=$target
fi

cargo +nightly fuzz build

failed=''
for name in $targets; do
  mkdir -p "corpus/$name" "artifacts/$name"
  echo "fuzz.sh: $name for ${seconds}s"
  if ! cargo +nightly fuzz run "$name" "corpus/$name" -- \
    -max_total_time="$seconds" \
    -artifact_prefix="artifacts/$name/" \
    -print_final_stats=1; then
    failed="$failed $name"
  fi
done

if [ -n "$failed" ]; then
  echo "fuzz.sh: crashes in:$failed (inputs under rust/fuzz/artifacts/)" >&2
  exit 1
fi
echo "fuzz.sh: no crashes"
