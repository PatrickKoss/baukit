#!/usr/bin/env bash
# Publish the baukit library crates to crates.io in dependency order.
#
# Crates already on the registry at the current version are skipped, so an
# interrupted run can simply be restarted. crates.io rate-limits new crate
# names to roughly one per ten minutes; a 429 makes this script wait until the
# time the registry names and then retry the same crate.
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

ORDER=(
  baukit-core baukit-events baukit-openapi baukit-sync
  baukit-config baukit-runtime baukit-telemetry baukit-credential-vault
  baukit-http baukit-jobs baukit-ops baukit-auth
  baukit-integrations baukit-push baukit-ratelimit baukit-test
)

version=$(python3 -c "
import tomllib
print(tomllib.load(open('rust/Cargo.toml','rb'))['workspace']['package']['version'])
")

already_published() {
  cargo info "$1@$version" --registry crates-io >/dev/null 2>&1
}

# crates.io reports the retry time as an RFC 2822 date in the 429 body.
sleep_until_retry_time() {
  local log=$1 retry_at wait_for
  retry_at=$(sed -n 's/.*Please try again after \([^ ].*GMT\).*/\1/p' "$log" | head -n 1)
  if [[ -z "$retry_at" ]]; then
    wait_for=600
  else
    wait_for=$(( $(date -d "$retry_at" +%s) - $(date +%s) + 15 ))
    (( wait_for < 15 )) && wait_for=15
  fi
  # Trusted-publishing tokens live 30 minutes. Sleeping past that turns a
  # rate-limit wait into an opaque 403 on the next crate, so stop while the
  # already-published crates are still a clean prefix to resume from.
  if [[ -n "${TOKEN_DEADLINE:-}" ]] && (( $(date +%s) + wait_for >= TOKEN_DEADLINE )); then
    cat >&2 <<MSG
rate limited for ${wait_for}s, which outlasts the crates.io token.
Crates published so far are on the registry; re-run this job to resume.
MSG
    exit 75
  fi
  echo "rate limited; waiting ${wait_for}s" >&2
  sleep "$wait_for"
}

log=$(mktemp)
trap 'rm -f "$log"' EXIT

for crate in "${ORDER[@]}"; do
  if already_published "$crate"; then
    echo "== $crate $version already on crates.io, skipping"
    continue
  fi

  while true; do
    echo "== publishing $crate $version"
    if cargo publish --manifest-path "rust/crates/$crate/Cargo.toml" 2>&1 | tee "$log"; then
      break
    fi
    if grep -q '429 Too Many Requests' "$log"; then
      sleep_until_retry_time "$log"
      continue
    fi
    echo "publishing $crate failed; see the output above" >&2
    exit 1
  done
done

echo "all ${#ORDER[@]} crates published at $version"
