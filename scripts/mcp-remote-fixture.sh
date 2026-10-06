#!/usr/bin/env bash
set -euo pipefail

export CI=true CARGO_BUILD_JOBS=6 VITEST_MAX_WORKERS=4
fixture_name=long-product-name-fixture
fixture_parent=$(mktemp -d)
trap 'python3 -c "import shutil,sys; shutil.rmtree(sys.argv[1])" "$fixture_parent"' EXIT
fixture="$fixture_parent/$fixture_name"

cargo build --manifest-path cli/Cargo.toml --bin baukit
corepack pnpm --dir typescript install --frozen-lockfile
corepack pnpm --dir typescript --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/analytics-posthog-native --filter @baukit/api-runtime --filter @baukit/auth-native --filter @baukit/auth-node --filter @baukit/data-contracts --filter @baukit/ui-tokens --filter @baukit/navigation run build
cli/target/debug/baukit new "$fixture_name" --backend --web --mobile --mcp --auth oidc --dir "$fixture_parent" --baukit-path rust
cargo fmt --manifest-path "$fixture/backend/Cargo.toml" --all --check
cargo clippy --manifest-path "$fixture/backend/Cargo.toml" --all-targets -- -D warnings
cargo test --manifest-path "$fixture/backend/Cargo.toml" -- --include-ignored
cargo test --manifest-path "$fixture/backend/Cargo.toml" -p "$fixture_name-bin" --test openapi_drift
cargo test --manifest-path "$fixture/backend/Cargo.toml" -p "$fixture_name-mcp" --test tool_drift
cargo build --manifest-path "$fixture/backend/Cargo.toml" -p "$fixture_name-bin" --bins
corepack pnpm --dir "$fixture/web" build
corepack pnpm --dir "$fixture/web" lint
corepack pnpm --dir "$fixture/web" test
corepack pnpm --dir "$fixture/web" run test:coverage
corepack pnpm --dir "$fixture/mobile" exec expo install --check
corepack pnpm --dir "$fixture/mobile" exec tsc --noEmit
corepack pnpm --dir "$fixture/mobile" lint
corepack pnpm --dir "$fixture/mobile" run test:coverage
python3 scripts/mcp-remote-smoke.py "$fixture"
