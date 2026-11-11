#!/usr/bin/env bash
set -euo pipefail

# Match the generated-fixture matrix in .github/workflows/ci.yml.
export CI=true CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6} VITEST_MAX_WORKERS=${VITEST_MAX_WORKERS:-4}
repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"
mkdir -p .generated-fixture
fixture_parent=$(mktemp -d "$repo_root/.generated-fixture/gate-XXXXXX")
trap 'rm -rf "$fixture_parent"' EXIT
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$fixture_parent/target}"
export PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH:-$repo_root/typescript/.playwright-browsers}"

cargo build --manifest-path cli/Cargo.toml --bin baukit --target-dir cli/target
if [[ "${BAUKIT_TYPESCRIPT_BUILT:-0}" != 1 ]]; then
    corepack pnpm --dir typescript install --frozen-lockfile
    corepack pnpm --dir typescript --filter @baukit/type-guards --filter @baukit/csp --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/analytics-posthog-native --filter @baukit/api-runtime --filter @baukit/auth-native --filter @baukit/auth-node --filter @baukit/auth-web --filter @baukit/data-contracts --filter @baukit/ui-tokens --filter @baukit/navigation --filter @baukit/purchases-core --filter @baukit/purchases-revenuecat --filter @baukit/pwa-web run build
fi

flavors=(backend web mobile mobile-pwa mobile-pwa-auth combined mcp auth clerk workos platform-standalone platform)
if (($#)); then flavors=("$@"); fi
report_flavor() {
    if [[ -n "${previous:-}" ]]; then echo "Fixture flavor $previous took $((SECONDS - flavor_started)) s"; fi
}
previous=""
flavor_started=$SECONDS
for flavor in "${flavors[@]}"; do
    report_flavor
    previous=$flavor
    flavor_started=$SECONDS
    backend=false mobile=false web=false mcp=false
    name=fixture
    case "$flavor" in
        backend) flags=(--backend); backend=true ;;
        web) flags=(--web); web=true ;;
        mobile) flags=(--mobile); name="mobile-fixture"; mobile=true ;;
        mobile-pwa) flags=(--mobile --pwa); mobile=true ;;
        mobile-pwa-auth) flags=(--backend --mobile --pwa --auth oidc); name="mobile-pwa-auth-fixture"; backend=true mobile=true ;;
        combined) flags=(--backend --mobile --web); backend=true mobile=true web=true ;;
        mcp) flags=(--backend --mcp --auth oidc); backend=true mcp=true ;;
        auth) flags=(--backend --mobile --web --auth oidc --mcp); name=long-product-name-fixture; backend=true mobile=true web=true mcp=true ;;
        clerk|workos) flags=(--backend --mobile --web --auth "$flavor" --mcp); name="$flavor-provider-fixture"; backend=true mobile=true web=true mcp=true ;;
        platform-standalone) flags=(--backend --platform --web); name=platform-standalone-fixture; backend=true web=true ;;
        platform) flags=(--backend --platform --worker --web --auth oidc); name=platform-fixture; backend=true web=true ;;
        *) echo "unknown fixture flavor: $flavor" >&2; exit 2 ;;
    esac
    echo "Checking generated fixture: $flavor"
    cli/target/debug/baukit new "$name" "${flags[@]}" --dir "$fixture_parent/$flavor" --baukit-path rust
    fixture="$fixture_parent/$flavor/$name"
    mise trust --quiet "$fixture/mise.toml"
    for step in workflow-lint secret-scan osv python-lint shell-lint; do
        (cd "$fixture" && mise exec -- sh scripts/repo-lint.sh "$step")
    done
    if "$backend"; then
        cargo fmt --manifest-path "$fixture/backend/Cargo.toml" --all --check
        cargo clippy --manifest-path "$fixture/backend/Cargo.toml" --all-targets -- -D warnings
        cargo test --manifest-path "$fixture/backend/Cargo.toml" -- --include-ignored
        cargo test --manifest-path "$fixture/backend/Cargo.toml" -p "$name-bin" --test openapi_drift
    fi
    if [[ "$flavor" == platform* ]]; then
        cp "$fixture/generated/openapi.d.ts" "$fixture_parent/openapi-before.d.ts"
        (cd "$fixture" && sh scripts/openapi-client.sh)
        diff -u "$fixture_parent/openapi-before.d.ts" "$fixture/generated/openapi.d.ts"
        helm template "$name" deploy/chart/baukit-app -f "$fixture/deploy/values.yaml" > "$fixture_parent/platform-rendered.yaml"
        if grep -q 'PLATFORM__RECEIVER_SECRET\|PLATFORM__PREVIOUS_RECEIVER_SECRET' "$fixture_parent/platform-rendered.yaml"; then
            echo "standalone platform values unexpectedly inject receiver secrets" >&2
            exit 1
        fi
    fi
    if "$web"; then
        corepack pnpm --dir "$fixture/web" install --frozen-lockfile
        corepack pnpm --dir "$fixture/web" build
        corepack pnpm --dir "$fixture/web" lint
        corepack pnpm --dir "$fixture/web" test
        corepack pnpm --dir "$fixture/web" run test:coverage
        scripts/with-loopback-network.sh sh scripts/check-web-headers.sh "$fixture/web"
        corepack pnpm --dir "$fixture/web" exec playwright install chromium
        scripts/with-loopback-network.sh corepack pnpm --dir "$fixture/web" run test:e2e --project=desktop-chromium
    fi
    if "$mobile"; then
        corepack pnpm --dir "$fixture/mobile" install --frozen-lockfile
        corepack pnpm --dir "$fixture/mobile" exec expo install --check
        corepack pnpm --dir "$fixture/mobile" exec tsc --noEmit
        corepack pnpm --dir "$fixture/mobile" lint
        corepack pnpm --dir "$fixture/mobile" run test:coverage
    fi
    if [[ "$flavor" == clerk || "$flavor" == workos ]]; then
        corepack pnpm --dir "$fixture/web" run check:bundle
        (cd "$fixture/mobile" && node --test scripts/check-production-bundle.test.mjs)
        corepack pnpm --dir "$fixture/mobile" run check:bundle
    fi
    if [[ "$flavor" == mobile-pwa || "$flavor" == mobile-pwa-auth ]]; then
        corepack pnpm --dir "$fixture/mobile" exec expo export --platform web --max-workers 4
        corepack pnpm --dir "$fixture/mobile" run build:sw
        corepack pnpm --dir "$fixture/mobile" run build:sw:check
        corepack pnpm --dir "$fixture/mobile" run csp
    fi
    if [[ "$flavor" == auth || "$flavor" == clerk || "$flavor" == workos ]]; then
        cargo build --manifest-path "$fixture/backend/Cargo.toml" -p "$name-bin" --bins
        python3 scripts/web-session-stack.py "$fixture"
    fi
    if "$mcp"; then
        cargo test --manifest-path "$fixture/backend/Cargo.toml" -p "$name-mcp" --test tool_drift
    fi
done
report_flavor
