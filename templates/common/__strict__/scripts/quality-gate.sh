#!/bin/sh
set -eu

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

usage() {
  echo "usage: sh scripts/quality-gate.sh [--affected BASE] [--plan]" >&2
  exit 2
}

manifest_value() {
  python3 - "$@" <<'PY'
import sys
import tomllib

with open("baukit.toml", "rb") as source:
    value = tomllib.load(source)
try:
    for component in sys.argv[1].split("."):
        value = value[component]
except KeyError:
    if len(sys.argv) < 3:
        raise
    value = sys.argv[2]
if isinstance(value, bool):
    print(str(value).lower())
elif isinstance(value, list):
    for item in value:
        print(item)
else:
    print(value)
PY
}

if [ "$(manifest_value quality.profile)" != "strict" ]; then
  echo 'quality gate: baukit.toml does not select profile = "strict"' >&2
  exit 2
fi

temporary_paths=
remove_temporary_paths() {
  for temporary_path in $temporary_paths; do
    rm -rf "$temporary_path"
  done
}
trap remove_temporary_paths EXIT

baukit_root=
use_baukit_root() {
  if [ -n "$baukit_root" ]; then
    return
  fi
  if [ "$(manifest_value dependencies.baukit.source)" = "path" ]; then
    baukit_root=$(CDPATH='' cd -- "$(manifest_value dependencies.baukit.path)/.." && pwd)
  else
    baukit_root=$(mktemp -d)
    temporary_paths="$temporary_paths $baukit_root"
    git clone --branch v{{ context.template_version }} --depth 1 \
      https://github.com/PatrickKoss/baukit.git "$baukit_root"
  fi
  MISE_TRUSTED_CONFIG_PATHS="$baukit_root${MISE_TRUSTED_CONFIG_PATHS:+:$MISE_TRUSTED_CONFIG_PATHS}"
  export MISE_TRUSTED_CONFIG_PATHS
}

if git rev-parse --verify HEAD >/dev/null 2>&1; then
  has_git_history=true
else
  has_git_history=false
fi

base_revision=
use_base_revision() {
  base_revision=${BAUKIT_BASE_REVISION:-${BAUKIT_GATE_BASE:-}}
  if [ -z "$base_revision" ]; then
    base_revision=$(git merge-base HEAD origin/main 2>/dev/null || git rev-parse HEAD^ 2>/dev/null || git rev-list --max-parents=0 HEAD 2>/dev/null || true)
  fi
  if [ -z "$base_revision" ]; then
    echo "quality gate: set BAUKIT_BASE_REVISION to compare with a base revision" >&2
    exit 2
  fi
}

gate_mode=${BAUKIT_GATE_MODE:-full}
workers=${BAUKIT_GATE_WORKERS:-2}
{% if context.web or context.pwa %}PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-${XDG_CACHE_HOME:-$HOME/.cache}/ms-playwright}
export PLAYWRIGHT_BROWSERS_PATH
{% endif %}{% if context.web or context.mobile %}pnpm_package=$(python3 -c 'import json; print(json.load(open("{{ "web" if context.web else "mobile" }}/package.json"))["packageManager"].split("+")[0])')
{% endif %}
has_token() {
  case " ${BAUKIT_GATE_TOKENS:-} " in
    *" $1 "*) return 0 ;;
  esac
  return 1
}

list_steps() {
  cat <<'STEPS'
{% if context.backend %}openapi fast - always -
rust-fmt fast - always -
migrations fast - always -
{% if context.mcp %}mcp-drift fast openapi affected -
{% endif %}{% endif %}scripts-tests fast - always -
markdown-links fast - always -
preflight fast - always -
baukit-doctor fast - always -
baukit-check fast - always -
workflow-lint fast - always -
secret-scan fast - always -
osv fast - always -
{% if context.backend %}cargo-deny fast - always -
{% endif %}python-lint fast - always -
shell-lint fast - always -
metric-names fast - always -
{% if context.web %}spec-purposes fast - always -
{% endif %}{% if context.backend %}rust-clippy backend - always -
rust-msrv backend rust-clippy selected -
rust-tests backend rust-msrv selected -
rust-doctests backend rust-tests selected -
fuzz fuzz - selected -
images images - selected -
{% endif %}{% if context.web %}web-install web - always -
web-build web web-install{% if context.backend %},openapi{% endif %} selected web-e2e,playwright
web-lint web web-build always -
web-unit web web-lint selected -
web-service-worker web web-build selected -
{% if context.auth_enabled and not context.auth_oidc %}web-bundle web web-build selected -
{% endif %}playwright web web-build selected web-e2e,playwright
webkit-repeats web playwright selected -
{% endif %}{% if context.mobile %}mobile-install mobile - always -
mobile-typecheck mobile mobile-install{% if context.backend %},openapi{% endif %} always -
mobile-lint mobile mobile-typecheck always -
expo-doctor mobile mobile-lint selected -
mobile-unit mobile expo-doctor selected -
{% if not context.web %}mobile-service-worker mobile mobile-unit selected -
{% endif %}{% if context.auth_enabled and not context.auth_oidc %}mobile-bundle mobile mobile-unit selected -
{% endif %}ios-check mobile mobile-unit selected -
android-build mobile ios-check selected -
android-e2e mobile android-build selected -
{% endif %}STEPS
  if [ "$(manifest_value quality.full_stack_e2e)" = "true" ]; then
    echo "full-stack-e2e stack - selected -"
  fi
  if command -v local_steps >/dev/null 2>&1; then
    local_steps
  fi
}

step_scripts_tests() {
  python3 scripts/reconcile-env.test.py
  python3 scripts/osv-resolved-lock.test.py
  python3 scripts/check-markdown-links.test.py
  python3 scripts/gate.test.py
}

step_markdown_links() {
  python3 scripts/check-markdown-links.py README.md CLAUDE.md AGENTS.md docs
}

step_preflight() {
  sh scripts/preflight.sh
}

step_baukit_doctor() {
  use_baukit_root
  cargo run --quiet --locked --manifest-path "$baukit_root/cli/Cargo.toml" --bin baukit -- doctor
}

step_baukit_check() {
  use_baukit_root
  cargo run --quiet --locked --manifest-path "$baukit_root/cli/Cargo.toml" --bin baukit -- \
    check agent-docs file-size suppressions lints patterns observability template-files --root .
}

step_workflow_lint() {
  sh scripts/repo-lint.sh workflow-lint
}

step_secret_scan() {
  sh scripts/repo-lint.sh secret-scan
}

step_osv() {
  sh scripts/repo-lint.sh osv
}

step_cargo_deny() {
  sh scripts/repo-lint.sh deny
}

step_python_lint() {
  sh scripts/repo-lint.sh python-lint
}

step_shell_lint() {
  sh scripts/repo-lint.sh shell-lint
}

step_metric_names() {
  if [ ! -f deploy/observability/product-metrics.txt ]; then
    echo "quality gate: no deploy/observability/product-metrics.txt"
    return
  fi
  use_baukit_root
  python3 "$baukit_root/deploy/observability/lint/check-metric-names.py" \
    --observability-root deploy/observability \
    --allowlist deploy/observability/product-metrics.txt
}

step_full_stack_e2e() {
  test -f scripts/full-stack-e2e.sh || {
    echo "quality gate: full_stack_e2e requires scripts/full-stack-e2e.sh" >&2
    exit 2
  }
  sh scripts/full-stack-e2e.sh
}
{% if context.backend %}
step_rust_fmt() {
  cargo fmt --manifest-path backend/Cargo.toml --all --check
}

step_rust_clippy() {
  cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
}

step_rust_msrv() {
  rust_version=$(python3 - <<'PY'
import re

text = open("backend/Cargo.toml", encoding="utf-8").read()
match = re.search(r'^\s*rust-version\s*=\s*"([^"]+)"', text, re.MULTILINE)
if match is None:
    raise SystemExit("backend/Cargo.toml declares no rust-version")
print(match.group(1))
PY
)
  cargo "+$rust_version" check --manifest-path backend/Cargo.toml --workspace --all-targets --locked
}

step_rust_tests() {
  if [ "$gate_mode" = affected ]; then
    set --
    for package in ${BAUKIT_GATE_RUST_PACKAGES:-}; do
      set -- "$@" --package "$package"
    done
    if [ "$#" -eq 0 ]; then
      set -- --workspace
    fi
    cargo nextest run --manifest-path backend/Cargo.toml "$@" --all-targets \
      --run-ignored all --test-threads "$workers"{% if context.mcp %} \
      -E 'not binary_id({{ context.app_name }}-mcp::tool_drift)'{% endif %}
    return
  fi
  coverage_lines=$(manifest_value quality.backend_coverage_lines)
  cargo llvm-cov nextest --manifest-path backend/Cargo.toml --workspace --all-targets \
    --fail-under-lines "$coverage_lines" --ignore-filename-regex 'src/bin/' \
    --run-ignored all --test-threads "$workers"
  cargo llvm-cov report --manifest-path backend/Cargo.toml \
    --ignore-filename-regex 'src/bin/' --html
  mkdir -p backend/target/llvm-cov
  cargo llvm-cov report --manifest-path backend/Cargo.toml \
    --ignore-filename-regex 'src/bin/' --lcov \
    --output-path backend/target/llvm-cov/lcov.info
}

step_rust_doctests() {
  cargo test --manifest-path backend/Cargo.toml --workspace --doc
}

# Runs each cargo-fuzz target in backend/fuzz for BAUKIT_FUZZ_SECONDS and fails on any crash.
step_fuzz() {
  fuzz_seconds=${BAUKIT_FUZZ_SECONDS:-60}
  cargo fmt --manifest-path backend/fuzz/Cargo.toml --all --check
  (
    cd backend/fuzz
    # cargo-fuzz defaults --target to the triple it was built for; prebuilt binaries are musl.
    host=$(rustc +nightly -vV | sed -n 's/^host: //p')
    cargo +nightly fuzz build --target "$host"
    for target in $(cargo +nightly fuzz list); do
      mkdir -p "corpus/$target" "artifacts/$target"
      cargo +nightly fuzz run --target "$host" "$target" "corpus/$target" -- \
        -max_total_time="$fuzz_seconds" \
        -artifact_prefix="artifacts/$target/"
    done
  )
}
{% if context.mcp %}
step_mcp_drift() {
  cargo nextest run --manifest-path backend/Cargo.toml \
    --package {{ context.app_name }}-mcp --test tool_drift --test-threads "$workers"
}
{% endif %}
step_migrations() {
  sh scripts/check-migrations-immutable.test.sh
  if [ "$has_git_history" = false ]; then
    echo "quality gate: migration immutability has no history to compare"
    return
  fi
  use_base_revision
  sh scripts/check-migrations-immutable.sh "$base_revision" HEAD
}

step_openapi() {
  committed_schema=$(mktemp)
  temporary_paths="$temporary_paths $committed_schema"
  cp "$(manifest_value openapi.schema)" "$committed_schema"
  sh scripts/openapi.sh
  diff -u "$committed_schema" "$(manifest_value openapi.schema)"
  openapi_compatibility=$(manifest_value quality.openapi_compatibility off)
  case "$openapi_compatibility" in
    off) ;;
    report | enforce)
      if [ "$has_git_history" = true ]; then
        use_base_revision
        use_baukit_root
        set -- --base-revision "$base_revision" --current "$(manifest_value openapi.schema)"
        if [ -f docs/openapi-accepted-breaks.json ]; then
          set -- "$@" --accepted docs/openapi-accepted-breaks.json
        fi
        if [ "$openapi_compatibility" = enforce ]; then
          set -- "$@" --enforce
        fi
        python3 "$baukit_root/scripts/check-openapi-compatibility.py" "$@"
      else
        echo "quality gate: OpenAPI compatibility has no history to compare"
      fi
      ;;
    *)
      echo "quality gate: quality.openapi_compatibility must be off, report, or enforce" >&2
      exit 2
      ;;
  esac
  if [ "$has_git_history" = false ]; then
    consumer_snapshot=$(mktemp -d)
    temporary_paths="$temporary_paths $consumer_snapshot"
    manifest_value openapi.consumers | while IFS= read -r consumer; do
      test -f "$consumer" || {
        echo "quality gate: OpenAPI consumer is missing: $consumer" >&2
        exit 1
      }
      mkdir -p "$consumer_snapshot/$(dirname "$consumer")"
      cp "$consumer" "$consumer_snapshot/$consumer"
    done
  fi
  sh scripts/openapi-client.sh
  manifest_value openapi.consumers | while IFS= read -r consumer; do
    if [ "$has_git_history" = true ]; then
      git ls-files --error-unmatch "$consumer" >/dev/null 2>&1 || {
        echo "quality gate: OpenAPI consumer is not committed: $consumer" >&2
        exit 1
      }
      git diff --exit-code -- "$consumer"
    else
      cmp "$consumer_snapshot/$consumer" "$consumer"
    fi
  done
}

step_images() {
  if [ ! -f backend/Dockerfile ]; then
    echo "quality gate: no backend/Dockerfile"
    return
  fi
  if [ "$(manifest_value dependencies.baukit.source)" = "path" ]; then
    baukit_path=$(manifest_value dependencies.baukit.path)
    build_context=$(mktemp -d)
    temporary_paths="$temporary_paths $build_context"
    mkdir -p "$build_context/backend" "$build_context/baukit"
    tar -C backend --exclude target -cf - . | tar -C "$build_context/backend" -xf -
    tar -C "$baukit_path" --exclude target -cf - . | tar -C "$build_context/baukit" -xf -
    cp limits.json "$build_context/limits.json"
    sh scripts/docker.sh build -f backend/Dockerfile \
      --build-arg BACKEND_CONTEXT=backend \
      --build-arg BAUKIT_CONTEXT=baukit \
      --build-arg BAUKIT_DESTINATION="$baukit_path" \
      "$build_context"
  else
    sh scripts/docker.sh build -f backend/Dockerfile --build-arg BACKEND_CONTEXT=backend .
  fi
}
{% endif %}{% if context.web %}
step_spec_purposes() {
  python3 scripts/gate.py check-specs web/e2e/tests
}

step_web_install() {
  corepack "$pnpm_package" --dir web install --frozen-lockfile
}

step_web_build() {
  corepack "$pnpm_package" --dir web build
}

step_web_lint() {
  corepack "$pnpm_package" --dir web lint
}

step_web_unit() {
  if [ "$gate_mode" = affected ]; then
    corepack "$pnpm_package" --dir web exec vitest run --changed "$BAUKIT_GATE_BASE" --passWithNoTests
    return
  fi
  corepack "$pnpm_package" --dir web run test:coverage
}

step_web_service_worker() {
  if [ "$(manifest_value capabilities.pwa)" != "true" ]; then
    echo "quality gate: capabilities.pwa is false"
    return
  fi
  corepack "$pnpm_package" --dir web run build:sw:check
}

{% if context.web and context.auth_enabled and not context.auth_oidc %}step_web_bundle() {
  (cd web && node --test scripts/check-production-bundle.test.mjs)
  corepack "$pnpm_package" --dir web run check:bundle
}

{% endif %}critical_paths() {
  manifest_value quality.critical_paths | while IFS= read -r critical_path; do
    [ -n "$critical_path" ] || continue
    case "$critical_path" in
      e2e/tests/*.spec.ts) ;;
      *) echo "quality gate: critical path must be an e2e/tests/*.spec.ts file: $critical_path" >&2; exit 2 ;;
    esac
    test -f "web/$critical_path" || { echo "quality gate: missing critical path web/$critical_path" >&2; exit 2; }
    echo "$critical_path"
  done
}

step_playwright() {
  set -- --config e2e/playwright.config.ts \
    --project=desktop-chromium --project=mobile-chrome \
    --project=webkit-desktop --project=mobile-safari
  if [ "$gate_mode" = affected ] && ! has_token web; then
    grep=$(printf '%s\n' "${BAUKIT_GATE_TOKENS:-}" | tr -s ' ' '\n' | sed -n 's/^playwright:\(@.*\)$/\1/p' | paste -sd '|' -)
    if has_token web-e2e || [ -z "$grep" ]; then
      set -- "$@" --only-changed="$BAUKIT_GATE_BASE"
    else
      set -- "$@" --grep "$grep"
    fi
  elif [ "$(manifest_value quality.webkit_repeats)" -gt 0 ]; then
    E2E_WEBKIT_SKIP=$(critical_paths | paste -sd ',' -)
    export E2E_WEBKIT_SKIP
  fi
  corepack "$pnpm_package" --dir web exec playwright test "$@"
}

step_webkit_repeats() {
  critical=$(critical_paths)
  if [ -z "$critical" ]; then
    echo "quality gate: quality.critical_paths is empty"
    return
  fi
  set --
  for critical_path in $critical; do
    set -- "$@" "$critical_path"
  done
  E2E_ALL_PURPOSES=1 corepack "$pnpm_package" --dir web exec playwright test "$@" \
    --config e2e/playwright.config.ts --project=webkit-desktop \
    --project=mobile-safari --repeat-each "$(manifest_value quality.webkit_repeats)"
}
{% endif %}{% if context.mobile %}
step_mobile_install() {
  corepack "$pnpm_package" --dir mobile install --frozen-lockfile
}

step_mobile_typecheck() {
  corepack "$pnpm_package" --dir mobile typecheck
}

step_mobile_lint() {
  corepack "$pnpm_package" --dir mobile lint
}

step_expo_doctor() {
  (cd mobile && corepack "$pnpm_package" dlx expo-doctor)
}

step_mobile_unit() {
  if [ "$gate_mode" = affected ]; then
    corepack "$pnpm_package" --dir mobile exec jest --changedSince="$BAUKIT_GATE_BASE" \
      --maxWorkers="$workers" --passWithNoTests
    return
  fi
  corepack "$pnpm_package" --dir mobile run test:coverage --maxWorkers="$workers"
}
{% if not context.web %}
step_mobile_service_worker() {
  if [ "$(manifest_value capabilities.pwa)" != "true" ]; then
    echo "quality gate: capabilities.pwa is false"
    return
  fi
{% if context.pwa %}  corepack "$pnpm_package" --dir mobile exec expo export --platform web --max-workers "$workers"
  corepack "$pnpm_package" --dir mobile run build:sw
{% endif %}  corepack "$pnpm_package" --dir mobile run build:sw:check
}
{% endif %}
{% if context.mobile and context.auth_enabled and not context.auth_oidc %}step_mobile_bundle() {
  (cd mobile && node --test scripts/check-production-bundle.test.mjs)
  corepack "$pnpm_package" --dir mobile run check:bundle
}

{% endif %}step_ios_check() {
  CI=1 corepack "$pnpm_package" --dir mobile exec expo prebuild --clean --platform ios
  CI=1 corepack "$pnpm_package" --dir mobile exec expo export --platform ios --output-dir dist/ios-check --max-workers "$workers"
}

step_android_build() {
  bash mobile/scripts/qa/android-sdk.sh
  BAUKIT_QA_ASSEMBLE_DEBUG=1 BAUKIT_QA_GRADLE_WORKERS="$workers" bash mobile/scripts/qa/build-android.sh
}

step_android_e2e() {
  status=0
  BAUKIT_QA_SKIP_BUILD=1 BAUKIT_QA_HEADLESS=1 bash mobile/scripts/qa/android-env.sh start || status=$?
  if [ "$status" -eq 0 ]; then
    bash mobile/scripts/qa/run-maestro.sh android || status=$?
  fi
  bash mobile/scripts/qa/android-env.sh stop || status=$?
  return "$status"
}
{% endif %}
if [ -f scripts/quality-gate.local.sh ]; then
  # shellcheck source=/dev/null
  . ./scripts/quality-gate.local.sh
fi

case "${1:-}" in
  --list-steps)
    list_steps
    ;;
  --step)
    [ "$#" -eq 2 ] || usage
    "step_$(printf %s "$2" | tr - _)"
    ;;
  "" | --affected | --plan)
    exec python3 scripts/gate.py run --script "sh scripts/quality-gate.sh" --settings baukit.toml \
      --log-dir "${BAUKIT_GATE_LOG_DIR:-.quality-gate}"{% if context.backend %} --cargo-workspace backend/Cargo.toml{% endif %} "$@"
    ;;
  *)
    usage
    ;;
esac
