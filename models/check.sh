#!/bin/sh
# Model-checks every TLA+ model under models/.
#   models/<Name>/<Name>.cfg          must pass (fixed design)
#   models/<Name>/<Name>*_current.cfg must report a violation (today's code)
# Other .cfg files (for example residual/*.cfg) are not run.
# Usage: models/check.sh [Name ...]
set -eu

root=$(cd "$(dirname "$0")" && pwd)
jar=${TLA2TOOLS_JAR:-$HOME/.local/share/tla/tla2tools.jar}
workers=${TLC_WORKERS:-4}
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM
failures=0

run_tlc() {
    dir=$1
    spec=$2
    cfg=$3
    log="$scratch/$spec-$(basename "$cfg" .cfg).log"
    (
        cd "$dir"
        java -XX:+UseParallelGC -cp "$jar" tlc2.TLC -workers "$workers" -deadlock \
            -cleanup -noGenerateSpecTE -metadir "$scratch/states-$(basename "$cfg" .cfg)" \
            -config "$cfg" "$spec.tla"
    ) >"$log" 2>&1 || true
    printf '%s\n' "$log"
}

summary() {
    grep -E '^[0-9]+ states generated|is violated|No error has been found|^Finished in' "$1" | tr '\n' ' '
}

check() {
    dir=$1
    spec=$2
    cfg=$3
    expect=$4
    log=$(run_tlc "$dir" "$spec" "$cfg")
    if [ "$expect" = pass ] && grep -q 'No error has been found' "$log"; then
        printf 'ok    %s/%s: %s\n' "$spec" "$cfg" "$(summary "$log")"
    elif [ "$expect" = violation ] && grep -q 'is violated' "$log"; then
        printf 'ok    %s/%s (expected violation): %s\n' "$spec" "$cfg" "$(summary "$log")"
    else
        printf 'FAIL  %s/%s expected %s\n' "$spec" "$cfg" "$expect"
        tail -n 40 "$log"
        failures=$((failures + 1))
    fi
}

check_model() {
    spec=$1
    dir="$root/$spec"
    if [ ! -f "$dir/$spec.tla" ] || [ ! -f "$dir/$spec.cfg" ]; then
        printf 'FAIL  %s: missing %s.tla or %s.cfg\n' "$spec" "$spec" "$spec"
        failures=$((failures + 1))
        return
    fi
    check "$dir" "$spec" "$spec.cfg" pass
    for cfg in "$dir/$spec"*_current.cfg; do
        [ -f "$cfg" ] || continue
        check "$dir" "$spec" "$(basename "$cfg")" violation
    done
}

if [ "$#" -eq 0 ]; then
    for dir in "$root"/*/; do
        check_model "$(basename "$dir")"
    done
else
    for spec in "$@"; do
        check_model "$spec"
    done
fi

[ "$failures" -eq 0 ]
