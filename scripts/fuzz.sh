#!/bin/sh
# Usage: sh scripts/fuzz.sh [target|all] [seconds]
# Runs each cargo-fuzz target in rust/fuzz for a fixed time and fails on any crash.
# BAUKIT_FUZZ_JOBS (default BAUKIT_GATE_WORKERS, else 1) targets run at once, each on one core.
# BAUKIT_FUZZ_CORPUS_DIR keeps the inputs a run discovers there (one subdirectory per target)
# instead of a throwaway directory, so a scheduled run can cache and grow its corpus.
set -eu

DEFAULT_SECONDS=60
target=${1:-all}
seconds=${2:-$DEFAULT_SECONDS}
jobs=${BAUKIT_FUZZ_JOBS:-${BAUKIT_GATE_WORKERS:-1}}

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fuzz_dir="$root/rust/fuzz"

require_positive_integer() {
  case $2 in
    '' | *[!0-9]* | 0)
      echo "fuzz.sh: $1 must be a positive integer, got '$2'" >&2
      exit 2
      ;;
  esac
}

require_positive_integer seconds "$seconds"
require_positive_integer BAUKIT_FUZZ_JOBS "$jobs"

cd "$fuzz_dir"

# cargo-fuzz defaults --target to the triple it was built for; prebuilt binaries are musl.
host=$(rustc +nightly -vV | sed -n 's/^host: //p')

if [ "$target" = all ]; then
  targets=$(cargo +nightly fuzz list)
else
  targets=$target
fi

cargo +nightly fuzz build --target "$host"

state=$(mktemp -d)
if [ -n "${BAUKIT_FUZZ_CORPUS_DIR:-}" ]; then
  scratch=$BAUKIT_FUZZ_CORPUS_DIR
  trap 'rm -rf "$state"' EXIT
else
  scratch=$(mktemp -d)
  trap 'rm -rf "$state" "$scratch"' EXIT
fi

fuzz_target() {
  mkdir -p "corpus/$1" "artifacts/$1" "$scratch/$1" || return
  echo "fuzz.sh: $1 for ${seconds}s"
  cargo +nightly fuzz run --target "$host" "$1" "$scratch/$1" "corpus/$1" -- \
    -max_total_time="$seconds" \
    -artifact_prefix="artifacts/$1/" \
    -print_final_stats=1
}

record() {
  if [ "$2" -eq 0 ]; then
    : >"$state/$1.passed"
  fi
}

# A directory is the only lock POSIX sh can take atomically; it keeps whole logs apart.
print_log() {
  until mkdir "$state/print.lock" 2>/dev/null; do
    sleep 1
  done
  cat "$state/$1.log"
  rmdir "$state/print.lock"
}

# Worker $1 of $jobs runs every target whose position modulo $jobs is $1.
run_worker() {
  position=0
  for name in $targets; do
    position=$((position + 1))
    [ $((position % jobs)) -eq "$1" ] || continue
    status=0
    if [ "$jobs" -eq 1 ]; then
      fuzz_target "$name" || status=$?
    else
      fuzz_target "$name" >"$state/$name.log" 2>&1 || status=$?
      print_log "$name"
    fi
    record "$name" "$status"
  done
}

worker=0
while [ "$worker" -lt "$jobs" ]; do
  run_worker "$worker" &
  worker=$((worker + 1))
done
wait

failed=''
for name in $targets; do
  [ -e "$state/$name.passed" ] || failed="$failed $name"
done

if [ -n "$failed" ]; then
  echo "fuzz.sh: crashes in:$failed (inputs under rust/fuzz/artifacts/)" >&2
  exit 1
fi
echo "fuzz.sh: no crashes"
