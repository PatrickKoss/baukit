#!/usr/bin/env bash
# Usage: check-rules.sh <pack dir>...
# Runs promtool check rules, promtool test rules and pint on each observability pack. Every rule
# file must be loaded by one of the pack's tests/*.test.yml files.
set -euo pipefail

lint_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$lint_dir/../../.." && pwd)
# shellcheck source=deploy/lib/tools.sh
. "$repo_root/deploy/lib/tools.sh"
baukit_use_tools "$repo_root" promtool pint

if [ "$#" -eq 0 ]; then
    echo "usage: $0 <pack dir>..." >&2
    exit 2
fi

check_pack() {
    local pack=$1
    local -a rules tests
    local rule untested=0

    if [ ! -d "$pack" ]; then
        echo "ERROR: $pack is not a directory" >&2
        return 1
    fi
    mapfile -t rules < <(find "$pack/alerts" "$pack/recording-rules" -maxdepth 1 -type f \
        \( -name '*.yml' -o -name '*.yaml' \) 2>/dev/null | sort)
    mapfile -t tests < <(find "$pack/tests" -maxdepth 1 -type f -name '*.test.yml' 2>/dev/null | sort)

    if [ "${#rules[@]}" -eq 0 ]; then
        echo "$pack: no rule files"
        return 0
    fi
    for rule in "${rules[@]}"; do
        if [ "${#tests[@]}" -eq 0 ] || ! grep -Fq -- "../${rule#"$pack"/}" "${tests[@]}"; then
            echo "ERROR: $rule is not loaded by any $pack/tests/*.test.yml" >&2
            untested=1
        fi
    done
    [ "$untested" -eq 0 ] || return 1

    promtool check rules "${rules[@]}"
    promtool test rules "${tests[@]}"
    pint --no-color --config "$lint_dir/pint.hcl" --offline --log-level warn \
        lint --fail-on warning "${rules[@]}"
}

for pack in "$@"; do
    check_pack "${pack%/}"
done
