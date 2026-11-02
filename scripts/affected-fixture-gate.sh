#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"
mkdir -p .generated-fixture
fixture_parent=$(mktemp -d "$repo_root/.generated-fixture/affected-XXXXXX")
trap 'rm -rf "$fixture_parent"' EXIT

cargo build --manifest-path cli/Cargo.toml --bin baukit --target-dir cli/target
cli/target/debug/baukit new fixture --backend --quality strict --dir "$fixture_parent" --baukit-path rust
fixture="$fixture_parent/fixture"
commit() {
    git -C "$fixture" add -A
    git -C "$fixture" -c user.name=gate -c user.email=gate@example.invalid commit -qm "$1"
}
plan() {
    (cd "$fixture" && BAUKIT_GATE_CORES=12 sh scripts/quality-gate.sh --affected HEAD^ --plan)
}
expect() {
    grep -qF -- "$2" <<<"$1" || { echo "affected fixture: missing \`$2\`" >&2; exit 1; }
}
refuse() {
    if grep -qF -- "$2" <<<"$1"; then echo "affected fixture: unexpected \`$2\`" >&2; exit 1; fi
}

git -C "$fixture" init -q
commit base
echo "// affected" >> "$fixture/backend/crates/fixture-domain/src/lib.rs"
commit crate
crate_plan=$(plan)
echo "$crate_plan"
expect "$crate_plan" "gate: mode affected against HEAD^"
expect "$crate_plan" "gate: rust packages: fixture-"
expect "$crate_plan" "backend/rust-tests (selected by backend)"
expect "$crate_plan" "(fast check)"
refuse "$crate_plan" "images"
refuse "$crate_plan" "(full gate)"

echo "# affected" >> "$fixture/backend/Cargo.lock"
commit lockfile
lock_plan=$(plan)
echo "$lock_plan"
expect "$lock_plan" "gate: mode full against HEAD^"
expect "$lock_plan" "images (full gate)"
refuse "$lock_plan" "(selected by"
