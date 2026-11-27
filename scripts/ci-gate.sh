#!/bin/sh
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

usage() {
  echo "usage: scripts/ci-gate.sh [--affected BASE] [--plan] | --list-steps | --step NAME" >&2
  exit 2
}

gate_mode=${BAUKIT_GATE_MODE:-full}
workers=${BAUKIT_GATE_WORKERS:-2}
msrv=1.95
generated_flavors="backend web mobile mobile-pwa mobile-pwa-auth combined mcp auth clerk workos"

list_steps() {
  cat <<'STEPS'
version-coherence fast - always -
docs-links fast - always -
example-lockfiles fast - always -
rust-fmt fast - always -
cli-fmt fast - always -
cargo-deny fast - always -
workflow-lint fast - always -
secret-scan fast - always -
osv fast - always -
python-lint fast - always -
shell-lint fast - always -
metric-names fast - always -
observability-tests fast - always -
observability-rules fast - always -
platform-validate fast - always -
platform-images images - selected -
media-grants fast - always -
scripts-tests fast ts-install always -
rust-clippy rust - always -
rust-tests rust rust-clippy selected -
rust-check rust rust-tests selected -
suite-features rust rust-check selected -
rust-msrv rust suite-features selected -
examples-test rust rust-msrv selected -
cli-clippy cli - always -
cli-tests cli cli-clippy selected -
cli-check cli cli-tests selected -
baukit-check cli cli-check selected -
ts-install ts - always -
ts-check ts ts-install selected -
package-dependencies ts ts-check selected -
ts-browser ts package-dependencies selected -
fixture-1 fixtures-1 ts-check selected fixtures
fixture-2 fixtures-2 ts-check selected fixtures
affected-fixture fixtures-2 fixture-2 selected fixtures
mcp-fixture mcp ts-check selected -
native-android android ts-check selected -
expo-sqlite android native-android selected -
expo-notifications android expo-sqlite selected -
fuzz fuzz - selected -
workflows fast - always -
models models - selected -
kani kani - selected -
verus verus - selected -
migrations migrations - selected -
vectors vectors - selected -
platform-e2e platform-e2e - selected -
STEPS
}

typescript_built() {
  case " ${BAUKIT_GATE_STEPS:-} " in
    *" ts-check "*) export BAUKIT_TYPESCRIPT_BUILT=1 ;;
  esac
}

step_version_coherence() { scripts/check-version-coherence.py; }
step_docs_links() { python3 scripts/check_docs_links.py; }
step_example_lockfiles() { python3 scripts/check-example-lockfiles.py; }
step_rust_fmt() { cargo fmt --manifest-path rust/Cargo.toml --all --check; }
step_cli_fmt() { cargo fmt --manifest-path cli/Cargo.toml --all --check; }
step_cargo_deny() {
  cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check
  cargo deny --manifest-path cli/Cargo.toml --config cli/deny.toml check
}
step_workflow_lint() {
  actionlint
  zizmor --offline --persona regular --config zizmor.yml .github/workflows
}
step_secret_scan() { gitleaks git --no-banner --redact --config .gitleaks.toml .; }
step_osv() {
  resolved=$(mktemp -d)
  trap 'rm -rf "$resolved"' EXIT
  set -- -L typescript/pnpm-lock.yaml -L pnpm-lock.yaml
  for manifest in rust/Cargo.toml cli/Cargo.toml examples/*/Cargo.toml; do
    directory="$resolved/$(dirname "$manifest")"
    python3 scripts/osv-resolved-lock.py "$manifest" "$directory"
    set -- "$@" -L "$directory/Cargo.lock"
  done
  for lockfile in examples/*/pnpm-lock.yaml; do
    set -- "$@" -L "$lockfile"
  done
  osv-scanner scan source --config osv-scanner.toml "$@"
}
# templates/ hold Jinja sources that generated products lint after rendering. Their Python
# carries no Jinja, so python-lint also checks it with the products' ruff config.
tracked_sources() {
  git ls-files -z -- "$@" | grep -zvE '(^|/)(node_modules|target|vendor)/|^templates/'
}
step_python_lint() {
  tracked_sources '*.py' | xargs -0 ruff check --force-exclude
  tracked_sources '*.py' | xargs -0 ruff format --check --force-exclude
  git ls-files -z -- 'templates/*.py' | xargs -0 ruff check --config templates/common/ruff.toml
  git ls-files -z -- 'templates/*.py' | xargs -0 ruff format --check --config templates/common/ruff.toml
}
step_shell_lint() { tracked_sources '*.sh' | xargs -0 shellcheck; }
step_metric_names() {
  python3 deploy/observability/lint/check-metric-names.py
  python3 deploy/observability/lint/check-metric-names.py --observability-root deploy/observability-platform \
    --allowlist deploy/observability-platform/metrics.txt --no-catalog
  python3 deploy/observability/lint/check-dashboards.py deploy/observability/dashboards \
    deploy/observability-platform/dashboards
}
step_observability_tests() {
  scripts/install-deploy-tools.sh --only promtool,pint,helm
  python3 -m unittest discover -s deploy/observability/lint -p 'test_*.py'
}
step_observability_rules() {
  scripts/install-deploy-tools.sh --only promtool,pint
  deploy/observability/lint/check-rules.sh deploy/observability deploy/observability-platform
}
step_platform_validate() {
  scripts/install-deploy-tools.sh --only kustomize,kubeconform,helm,flux,kyverno,chainsaw,k3d,kubectl
  ./deploy/platform/validate.sh
}
step_platform_e2e() {
  scripts/install-deploy-tools.sh --only chainsaw,flux,helm,k3d,kubectl,kustomize
  ./deploy/platform/e2e/run.sh
}
step_platform_images() {
  scripts/install-deploy-tools.sh --only kustomize,kubeconform,helm,flux,kyverno,trivy
  scripts/platform-images.sh
}
step_media_grants() { make media-grants-test media-grants-njs-test; }
step_scripts_tests() {
  scripts_tests_tools=$(mktemp -d)
  scripts_tests_status=0
  BAUKIT_TOOLS_DIR=$scripts_tests_tools python3 -m unittest discover -s scripts -p 'test_*.py' || scripts_tests_status=$?
  rm -rf "$scripts_tests_tools"
  return "$scripts_tests_status"
}

step_rust_clippy() {
  cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
}

step_rust_tests() {
  set --
  if [ "$gate_mode" = affected ]; then
    for package in ${BAUKIT_GATE_RUST_PACKAGES:-}; do
      set -- "$@" --package "$package"
    done
  fi
  [ "$#" -gt 0 ] || set -- --workspace
  cargo test --manifest-path rust/Cargo.toml "$@" -- --include-ignored --test-threads "$workers"
}

step_rust_check() {
  cargo check --manifest-path rust/Cargo.toml --workspace --all-targets
  RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --workspace --all-features --no-deps
}
step_suite_features() { ./scripts/check-suite-features.sh; }
step_rust_msrv() {
  cargo "+$msrv" check --manifest-path rust/Cargo.toml --workspace --all-targets
}
step_fuzz() {
  cargo fmt --manifest-path rust/fuzz/Cargo.toml --all --check
  cargo clippy --manifest-path rust/fuzz/Cargo.toml --all-targets -- -D warnings
  sh scripts/fuzz.sh all "${BAUKIT_FUZZ_SECONDS:-60}"
}

# kani-full and fuzz-nightly are not in --list-steps: the nightly workflow calls them with
# --step, and the full gate would otherwise spend hours on them.
kani_crates() {
  grep -rl --include='*.rs' 'cfg(kani)' rust/crates 2>/dev/null | cut -d/ -f3 | sort -u
}
# Off-by-default features that compile a crate's harness modules.
kani_features() {
  case "$1" in
    baukit-core) echo media-grants ;;
    baukit-integrations) echo oauth-state ;;
    baukit-suite) echo platform ;;
  esac
}
kani_verified_total() {
  sed -n 's/^Complete - [0-9]* successfully verified harnesses, [0-9]* failures, \([0-9]*\) total\.$/\1/p' "$1" | tail -n 1
}
run_kani_crate() {
  crate=$1
  log=$2
  features=$(kani_features "$crate")
  expected=$(python3 scripts/list-proofs.py --count "$crate")
  # Kani 0.68 has no timeout flag; bound each crate's harness run instead.
  status=0
  (cd rust && timeout "${BAUKIT_KANI_TIMEOUT_SECONDS:-1800}" cargo kani -p "$crate" ${features:+--features "$features"}) >"$log" 2>&1 || status=$?
  cat "$log"
  [ "$status" -eq 0 ] || return "$status"
  verified=$(kani_verified_total "$log")
  if [ "${verified:-0}" -ne "$expected" ]; then
    echo "kani: $crate verified ${verified:-0} of its $expected listed harnesses; add its harness feature to kani_features" >&2
    return 1
  fi
}
run_kani() {
  if [ "$#" -eq 0 ]; then
    echo "kani: no crate has cfg(kani) harnesses"
    return 0
  fi
  # Idempotent; also installs the nightly toolchain Kani pins, which runner caches miss.
  cargo kani setup
  kani_log=$(mktemp)
  for crate in "$@"; do
    run_kani_crate "$crate" "$kani_log" || { rm -f "$kani_log"; return 1; }
  done
  rm -f "$kani_log"
}
step_kani() {
  python3 scripts/list-proofs.py --check
  set --
  for crate in $(kani_crates); do
    if [ "$gate_mode" = affected ]; then
      case " ${BAUKIT_GATE_RUST_PACKAGES:-} " in
        *" $crate "*) ;;
        *) continue ;;
      esac
    fi
    set -- "$@" "$crate"
  done
  run_kani "$@"
}
step_kani_full() {
  python3 scripts/list-proofs.py --check
  set --
  for crate in $(kani_crates); do
    set -- "$@" "$crate"
  done
  run_kani "$@"
}
step_verus() {
  if [ ! -d rust/verus ]; then
    echo "verus: rust/verus does not exist yet"
    return 0
  fi
  cargo verus verify --manifest-path rust/verus/Cargo.toml
  cargo clippy --locked --manifest-path rust/verus/Cargo.toml --all-targets -- -D warnings
  cargo clippy --locked --manifest-path rust/verus/equivalence/Cargo.toml --all-targets -- -D warnings
  cargo test --locked --manifest-path rust/verus/equivalence/Cargo.toml
  cargo kani setup
  (cd rust/verus/equivalence && timeout "${BAUKIT_KANI_TIMEOUT_SECONDS:-1800}" cargo kani)
}
# Prints TLA2TOOLS_JAR, or installs the deploy/tools.toml pin and prints its path.
tla2tools_jar() {
  if [ -n "${TLA2TOOLS_JAR:-}" ]; then
    printf '%s\n' "$TLA2TOOLS_JAR"
    return
  fi
  scripts/install-deploy-tools.sh --only tla2tools >&2
  printf '%s\n' "${BAUKIT_TOOLS_DIR:-$repository_root/.tools}/bin/tla2tools"
}
step_models() {
  jar=$(tla2tools_jar)
  JAVA_TOOL_OPTIONS="${JAVA_TOOL_OPTIONS:--Xmx4g}" TLC_WORKERS="${TLC_WORKERS:-4}" \
    TLA2TOOLS_JAR="$jar" sh models/check.sh
}
step_migrations() {
  python3 scripts/check-migrations.py lint
  python3 scripts/check-migrations.py double-apply
  sh scripts/migrations-old-binary.sh
}
step_workflows() {
  actionlint
  zizmor --offline --persona auditor --min-severity low --config zizmor.yml .github/workflows
  python3 scripts/check-release-shape.py .github/workflows
}
step_vectors() { python3 scripts/regenerate-vectors.py --check; }
step_vectors_regenerate() { python3 scripts/regenerate-vectors.py; }
step_fuzz_nightly() {
  cargo fmt --manifest-path rust/fuzz/Cargo.toml --all --check
  sh scripts/fuzz.sh "${BAUKIT_FUZZ_TARGET:-all}" "${BAUKIT_FUZZ_SECONDS:-1800}"
}

step_examples_test() {
  cargo test --locked --manifest-path examples/minimal-api/Cargo.toml -- --test-threads "$workers"
}

step_cli_clippy() {
  cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings
}
step_cli_tests() {
  cargo test --manifest-path cli/Cargo.toml -- --include-ignored --test-threads "$workers"
}
step_cli_check() {
  cargo check --manifest-path cli/Cargo.toml --all-targets
  RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path cli/Cargo.toml --no-deps
}
step_baukit_check() {
  cargo run --manifest-path cli/Cargo.toml --bin baukit -- check --root .
}

step_ts_install() { corepack pnpm --dir typescript install --frozen-lockfile; }
step_ts_check() {
  corepack pnpm --dir typescript run test:source-maps
  corepack pnpm --dir typescript exec turbo run build lint test format:check --concurrency="$workers"
}
step_package_dependencies() { python3 scripts/check-package-dependencies.py; }
step_ts_browser() {
  PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-$repository_root/typescript/.playwright-browsers}
  export PLAYWRIGHT_BROWSERS_PATH
  corepack pnpm --dir typescript --filter @baukit/data-contracts-dexie exec playwright install chromium webkit
  scripts/with-loopback-network.sh corepack pnpm --dir typescript --filter @baukit/data-contracts-dexie run test:browser
  scripts/with-loopback-network.sh corepack pnpm --dir typescript --filter @baukit/navigation run test:browser
}

fixture_shard_1="clerk auth backend mobile-pwa-auth web"
fixture_shard_2="workos combined platform platform-standalone mcp mobile-pwa mobile"

run_fixture_flavors() {
  typescript_built
  # shellcheck disable=SC2086 # the flavor list is space-separated words, one argument each
  ./scripts/generated-fixture-gate.sh $1
}
step_generated_fixture() { run_fixture_flavors "${BAUKIT_FIXTURE_FLAVORS:-$generated_flavors}"; }
step_platform_fixture() { run_fixture_flavors "platform-standalone platform"; }
step_fixture_1() { run_fixture_flavors "$fixture_shard_1"; }
step_fixture_2() { run_fixture_flavors "$fixture_shard_2"; }
step_affected_fixture() { ./scripts/affected-fixture-gate.sh; }
step_mcp_fixture() {
  typescript_built
  ./scripts/mcp-remote-fixture.sh
}
step_native_android() {
  typescript_built
  make native-android-gate
}
step_expo_sqlite() {
  typescript_built
  make expo-sqlite-conformance
}
step_expo_notifications() {
  typescript_built
  make expo-notifications-conformance
}

case "${1:-}" in
  --list-steps)
    list_steps
    ;;
  --step)
    [ "$#" -eq 2 ] || usage
    "step_$(printf %s "$2" | tr - _)"
    ;;
  "" | --affected | --plan)
    PYTHONPATH="templates/common/scripts${PYTHONPATH:+:$PYTHONPATH}" exec python3 templates/common/__strict__/scripts/gate.py run --name "Baukit gate" \
      --script "sh scripts/ci-gate.sh" --settings scripts/ci-gate.toml \
      --log-dir "${BAUKIT_GATE_LOG_DIR:-.ci-gate}" --cargo-workspace rust/Cargo.toml "$@"
    ;;
  *)
    usage
    ;;
esac
