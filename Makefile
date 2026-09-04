.PHONY: toolchain fmt lint test check ci platform-validate platform-up platform-down platform-nuke platform-recreate platform-status ts-install ts-build ts-fmt ts-lint ts-test ts-browser-test ts-check cli-fmt cli-lint cli-test cli-check cli-ci mcp-fixture-gate install-skills android-sdk-setup native-android-gate expo-sqlite-conformance

RUST_MANIFEST := rust/Cargo.toml
TS_DIR := typescript
CLI_MANIFEST := cli/Cargo.toml

toolchain:
	@command -v mise >/dev/null || (echo "missing: mise (https://mise.jdx.dev/getting-started.html)" && exit 1)
	mise install
	mise exec -- corepack enable

fmt: ts-fmt
	cargo fmt --manifest-path $(RUST_MANIFEST) --all --check

lint: ts-lint
	cargo clippy --manifest-path $(RUST_MANIFEST) --all-targets -- -D warnings

test: ts-test
	cargo test --manifest-path $(RUST_MANIFEST)

check: ts-check
	cargo check --manifest-path $(RUST_MANIFEST) --workspace --all-targets

ci: fmt lint test check ts-check ts-browser-test cli-ci platform-validate

platform-validate:
	./deploy/platform/validate.sh

platform-up platform-down platform-nuke platform-recreate platform-status:
	./deploy/platform/platform-lifecycle.sh $(if $(PLATFORM_CONFIG),--config "$(PLATFORM_CONFIG)") $(patsubst platform-%,%,$@)

cli-fmt:
	cargo fmt --manifest-path $(CLI_MANIFEST) --all --check

cli-lint:
	cargo clippy --manifest-path $(CLI_MANIFEST) --all-targets -- -D warnings

cli-test:
	cargo test --manifest-path $(CLI_MANIFEST)

cli-check:
	cargo check --manifest-path $(CLI_MANIFEST) --all-targets

cli-ci: cli-fmt cli-lint cli-test cli-check

mcp-fixture-gate:
	@set -eu; \
	fixture_parent="$$(mktemp -d)"; \
	trap 'rm -rf "$$fixture_parent"' EXIT; \
	cargo build --manifest-path $(CLI_MANIFEST) --bin baukit; \
	cli/target/debug/baukit new fixture --backend --mcp --dir "$$fixture_parent" --baukit-path rust; \
	cargo fmt --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" --all --check; \
	cargo clippy --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" --all-targets -- -D warnings; \
	cargo test --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml"; \
	cargo test --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" -p fixture-bin --test openapi_drift; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" install --frozen-lockfile; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" build; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" typecheck; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" lint; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" test; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" openapi:check; \
	corepack pnpm@11.18.0 --dir "$$fixture_parent/fixture/mcp" docs:check

install-skills:
	@test -n "$(TARGET)" || (echo "TARGET is required: make install-skills TARGET=<product-dir>" >&2; exit 2)
	./agent-skills/install.sh --target "$(TARGET)"

android-sdk-setup:
	./scripts/android-sdk-setup.sh

native-android-gate: android-sdk-setup
	@fixture_parent="$$(mktemp -d)"; \
	trap 'rm -rf "$$fixture_parent"' EXIT; \
	corepack pnpm --dir $(TS_DIR) install --frozen-lockfile; \
	corepack pnpm --dir $(TS_DIR) --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/api-runtime --filter @baukit/data-contracts --filter @baukit/data-contracts-expo-sqlite --filter @baukit/localization-core --filter @baukit/ui-tokens run build; \
	cargo build --manifest-path $(CLI_MANIFEST) --bin baukit; \
	cli/target/debug/baukit new fixture --mobile --dir "$$fixture_parent" --baukit-path rust; \
	corepack pnpm --dir "$$fixture_parent/fixture/mobile" install --frozen-lockfile; \
	CI=1 corepack pnpm --dir "$$fixture_parent/fixture/mobile" exec expo prebuild --clean --platform android; \
	ANDROID_HOME="$${ANDROID_HOME:-$$HOME/Android/Sdk}" ANDROID_SDK_ROOT="$${ANDROID_SDK_ROOT:-$${ANDROID_HOME:-$$HOME/Android/Sdk}}" \
		"$$fixture_parent/fixture/mobile/android/gradlew" -p "$$fixture_parent/fixture/mobile/android" --no-daemon --stacktrace assembleDebug

expo-sqlite-conformance:
	./examples/expo-sqlite-conformance/scripts/run-android.sh

ts-install:
	corepack pnpm --dir $(TS_DIR) install --frozen-lockfile

ts-build: ts-install
	corepack pnpm --dir $(TS_DIR) run build

ts-fmt: ts-install
	corepack pnpm --dir $(TS_DIR) run format:check

ts-lint: ts-install
	corepack pnpm --dir $(TS_DIR) run lint

ts-test: ts-install
	corepack pnpm --dir $(TS_DIR) run test

ts-browser-test: ts-install
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/data-contracts-dexie exec playwright install --with-deps chromium webkit
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/data-contracts-dexie run test:browser

ts-check: ts-install
	corepack pnpm --dir $(TS_DIR) run check
