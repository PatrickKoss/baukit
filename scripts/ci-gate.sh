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
generated_flavors="backend web mobile mobile-pwa combined mcp auth clerk workos"

list_steps() {
  cat <<'STEPS'
version-coherence fast - always -
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
generated-fixture fixtures ts-check selected -
platform-fixture fixtures generated-fixture selected -
affected-fixture fixtures platform-fixture selected -
mcp-fixture mcp ts-check selected -
native-android android ts-check selected -
expo-sqlite android native-android selected -
expo-notifications android expo-sqlite selected -
fuzz fuzz - selected -
STEPS
}

typescript_built() {
  case " ${BAUKIT_GATE_STEPS:-} " in
    *" ts-check "*) export BAUKIT_TYPESCRIPT_BUILT=1 ;;
  esac
}

step_version_coherence() { scripts/check-version-coherence.py; }
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
  set -- -L rust/Cargo.lock -L cli/Cargo.lock -L typescript/pnpm-lock.yaml -L pnpm-lock.yaml
  for lockfile in examples/*/Cargo.lock examples/*/pnpm-lock.yaml; do
    set -- "$@" -L "$lockfile"
  done
  osv-scanner scan source --config osv-scanner.toml "$@"
}
# templates/ hold Jinja sources that generated products lint after rendering.
tracked_sources() {
  git ls-files -z -- "$@" | grep -zvE '(^|/)(node_modules|target|vendor)/|^templates/'
}
step_python_lint() {
  tracked_sources '*.py' | xargs -0 ruff check
  tracked_sources '*.py' | xargs -0 ruff format --check
}
step_shell_lint() { tracked_sources '*.sh' | xargs -0 shellcheck; }
step_metric_names() { python3 deploy/observability/lint/check-metric-names.py; }
step_observability_tests() {
  python3 -m unittest discover -s deploy/observability/lint -p 'test_*.py'
}
step_observability_rules() { deploy/observability/lint/check-rules.sh; }
step_platform_validate() { ./deploy/platform/validate.sh; }
step_media_grants() { make media-grants-test media-grants-njs-test; }
step_scripts_tests() { python3 -m unittest discover -s scripts -p 'test_*.py'; }

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

step_generated_fixture() {
  typescript_built
  # shellcheck disable=SC2086 # the flavor list is space-separated words, one argument each
  ./scripts/generated-fixture-gate.sh ${BAUKIT_FIXTURE_FLAVORS:-$generated_flavors}
}
step_platform_fixture() {
  typescript_built
  ./scripts/generated-fixture-gate.sh platform-standalone platform
}
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
    exec python3 templates/common/__strict__/scripts/gate.py run --name "Baukit gate" \
      --script "sh scripts/ci-gate.sh" --settings scripts/ci-gate.toml \
      --log-dir "${BAUKIT_GATE_LOG_DIR:-.ci-gate}" --cargo-workspace rust/Cargo.toml "$@"
    ;;
  *)
    usage
    ;;
esac
