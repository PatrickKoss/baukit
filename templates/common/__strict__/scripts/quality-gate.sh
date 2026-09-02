#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

manifest_value() {
  python3 - "$1" <<'PY'
import sys
import tomllib

with open("baukit.toml", "rb") as source:
    value = tomllib.load(source)
for component in sys.argv[1].split("."):
    value = value[component]
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

sh scripts/preflight.sh
{% if context.web %}PLAYWRIGHT_BROWSERS_PATH="$repository_root/web/node_modules/.cache/playwright-browsers"
export PLAYWRIGHT_BROWSERS_PATH
{% endif %}

{% if context.backend %}cargo fmt --manifest-path backend/Cargo.toml --all --check
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
coverage_lines=$(manifest_value quality.backend_coverage_lines)
cargo llvm-cov nextest --manifest-path backend/Cargo.toml --workspace --all-targets \
  --fail-under-lines "$coverage_lines" --ignore-filename-regex 'src/bin/' \
  --run-ignored all --test-threads 4
cargo llvm-cov report --manifest-path backend/Cargo.toml \
  --ignore-filename-regex 'src/bin/' --html
cargo llvm-cov report --manifest-path backend/Cargo.toml \
  --ignore-filename-regex 'src/bin/' --lcov \
  --output-path backend/target/llvm-cov/lcov.info

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

sh scripts/check-migrations-immutable.test.sh
base_revision=${BAUKIT_BASE_REVISION:-}
if [ -z "$base_revision" ]; then
  base_revision=$(git merge-base HEAD origin/main 2>/dev/null || git rev-parse HEAD^ 2>/dev/null || git rev-list --max-parents=0 HEAD 2>/dev/null || true)
fi
if [ -z "$base_revision" ]; then
  echo "quality gate: set BAUKIT_BASE_REVISION to check migration immutability" >&2
  exit 2
fi
sh scripts/check-migrations-immutable.sh "$base_revision" HEAD

committed_schema=$(mktemp)
cp "$(manifest_value openapi.schema)" "$committed_schema"
sh scripts/openapi.sh
diff -u "$committed_schema" "$(manifest_value openapi.schema)"
rm "$committed_schema"
sh scripts/openapi-client.sh
manifest_value openapi.consumers | while IFS= read -r consumer; do
  git ls-files --error-unmatch "$consumer" >/dev/null 2>&1 || {
    echo "quality gate: OpenAPI consumer is not committed: $consumer" >&2
    exit 1
  }
  git diff --exit-code -- "$consumer"
done

if [ -f backend/Dockerfile ]; then
  if [ "$(manifest_value dependencies.baukit.source)" = "path" ]; then
    baukit_path=$(manifest_value dependencies.baukit.path)
    build_context=$(mktemp -d)
    trap 'rm -rf "$build_context"' EXIT
    mkdir -p "$build_context/backend" "$build_context/baukit"
    tar -C backend --exclude target -cf - . | tar -C "$build_context/backend" -xf -
    tar -C "$baukit_path" --exclude target -cf - . | tar -C "$build_context/baukit" -xf -
    cp limits.json "$build_context/limits.json"
    docker build -f backend/Dockerfile \
      --build-arg BACKEND_CONTEXT=backend \
      --build-arg BAUKIT_CONTEXT=baukit \
      --build-arg BAUKIT_DESTINATION="$baukit_path" \
      "$build_context"
    rm -rf "$build_context"
  else
    docker build -f backend/Dockerfile --build-arg BACKEND_CONTEXT=backend .
  fi
fi
{% endif %}
{% if context.web %}corepack pnpm@11.18.0 --dir web install --frozen-lockfile
corepack pnpm@11.18.0 --dir web build
corepack pnpm@11.18.0 --dir web lint
corepack pnpm@11.18.0 --dir web run test:coverage
if [ "$(manifest_value capabilities.pwa)" = "true" ]; then
  corepack pnpm@11.18.0 --dir web run build:sw:check
fi
corepack pnpm@11.18.0 --dir web exec playwright test \
  --config e2e/playwright.config.ts \
  --project=desktop-chromium --project=mobile-chrome \
  --project=webkit-desktop --project=mobile-safari

critical_paths=$(manifest_value quality.critical_paths)
if [ -n "$critical_paths" ]; then
  repeats=$(manifest_value quality.webkit_repeats)
  set --
  while IFS= read -r critical_path; do
    case "$critical_path" in
      e2e/tests/*.spec.ts) ;;
      *) echo "quality gate: critical path must be an e2e/tests/*.spec.ts file: $critical_path" >&2; exit 2 ;;
    esac
    test -f "web/$critical_path" || { echo "quality gate: missing critical path web/$critical_path" >&2; exit 2; }
    set -- "$@" "$critical_path"
  done <<EOF
$critical_paths
EOF
  corepack pnpm@11.18.0 --dir web exec playwright test "$@" \
    --config e2e/playwright.config.ts --project=webkit-desktop \
    --project=mobile-safari --repeat-each "$repeats"
fi
{% endif %}
{% if context.mobile %}corepack pnpm@11.18.0 --dir mobile install --frozen-lockfile
(cd mobile && corepack pnpm@11.18.0 dlx expo-doctor)
corepack pnpm@11.18.0 --dir mobile typecheck
corepack pnpm@11.18.0 --dir mobile lint
corepack pnpm@11.18.0 --dir mobile run test:coverage
CI=1 corepack pnpm@11.18.0 --dir mobile exec expo prebuild --clean --platform ios
CI=1 corepack pnpm@11.18.0 --dir mobile exec expo export --platform ios --output-dir dist/ios-check
CI=1 corepack pnpm@11.18.0 --dir mobile exec expo prebuild --clean --platform android
mobile/android/gradlew -p mobile/android --no-daemon --stacktrace assembleDebug
{% endif %}

if [ -f scripts/observability-lint.py ]; then
  contract_checkout=$(mktemp -d)
  trap 'rm -rf "$contract_checkout"' EXIT
  git clone --branch v{{ context.template_version }} --depth 1 \
    https://github.com/PatrickKoss/baukit.git "$contract_checkout"
  python3 scripts/observability-lint.py \
    "$contract_checkout/deploy/observability/lint/check-metric-names.py"
fi

if [ "$(manifest_value quality.full_stack_e2e)" = "true" ]; then
  test -f scripts/full-stack-e2e.sh || {
    echo "quality gate: full_stack_e2e requires scripts/full-stack-e2e.sh" >&2
    exit 2
  }
  sh scripts/full-stack-e2e.sh
fi

echo "quality gate: passed"
