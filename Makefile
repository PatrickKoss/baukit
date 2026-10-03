.PHONY: toolchain fmt lint test check ci platform-validate platform-up platform-down platform-nuke platform-recreate platform-status ts-install ts-build ts-fmt ts-lint ts-test ts-browser-deps ts-browser-test ts-check cli-fmt cli-lint cli-test cli-check cli-ci scripts-test mcp-fixture-gate install-skills android-sdk-setup native-android-gate expo-sqlite-conformance expo-notifications-conformance media-grants-test media-grants-njs-test

RUST_MANIFEST := rust/Cargo.toml
TS_DIR := typescript
CLI_MANIFEST := cli/Cargo.toml
MEDIA_GRANTS_NJS := deploy/media-grants/njs
NJS_IMAGE := nginx:1.31-alpine@sha256:df221db836e1754089190208cee7eeda94f233197056426eda74a43ab1abeac2

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

ci: fmt lint test check ts-check ts-browser-test cli-ci scripts-test platform-validate media-grants-test media-grants-njs-test

media-grants-test:
	node --test $(MEDIA_GRANTS_NJS)/media-grant.test.mjs

media-grants-njs-test:
	for engine in njs QuickJS; do \
		docker run --rm -v "$(CURDIR):/repo:ro" $(NJS_IMAGE) njs -n $$engine -m -p /repo/$(MEDIA_GRANTS_NJS) /repo/$(MEDIA_GRANTS_NJS)/run-njs-vectors.js /repo/fixtures/media-grants/vectors-v1.json || exit 1; \
	done

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

scripts-test:
	python3 -m unittest discover -s scripts -p 'test_*.py'

mcp-fixture-gate:
	@set -eu; \
	fixture_parent="$$(mktemp -d)"; \
	trap 'rm -rf "$$fixture_parent"' EXIT; \
	cargo build --manifest-path $(CLI_MANIFEST) --bin baukit; \
	cli/target/debug/baukit new fixture --backend --mcp --dir "$$fixture_parent" --baukit-path rust; \
	corepack pnpm@12.7.0 --dir typescript install --frozen-lockfile; \
	corepack pnpm@12.7.0 --dir typescript --filter @baukit/auth-node run build; \
	cargo fmt --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" --all --check; \
	cargo clippy --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" --all-targets -- -D warnings; \
	cargo test --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml"; \
	cargo test --manifest-path "$$fixture_parent/fixture/backend/Cargo.toml" -p fixture-bin --test openapi_drift; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" install --frozen-lockfile; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" build; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" typecheck; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" lint; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" test; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" openapi:check; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mcp" docs:check

install-skills:
	@test -n "$(TARGET)" || (echo "TARGET is required: make install-skills TARGET=<product-dir>" >&2; exit 2)
	./agent-skills/install.sh --target "$(TARGET)"

android-sdk-setup:
	./scripts/android-sdk-setup.sh

native-android-gate: android-sdk-setup
	@fixture_parent="$$(mktemp -d)"; \
	trap 'rm -rf "$$fixture_parent"' EXIT; \
	corepack pnpm@12.7.0 --dir $(TS_DIR) install --frozen-lockfile --ignore-scripts; \
	corepack pnpm@12.7.0 --dir $(TS_DIR) --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/analytics-posthog-native --filter @baukit/api-runtime --filter @baukit/data-contracts --filter @baukit/data-contracts-expo-sqlite --filter @baukit/localization-core --filter @baukit/ui-tokens --filter @baukit/navigation run build; \
	cargo build --manifest-path $(CLI_MANIFEST) --bin baukit; \
	cli/target/debug/baukit new fixture --mobile --dir "$$fixture_parent" --baukit-path rust; \
	corepack pnpm@12.7.0 --dir "$$fixture_parent/fixture/mobile" install --frozen-lockfile; \
	(cd "$$fixture_parent/fixture/mobile" && BAUKIT_QA_BUILD=1 CI=1 ./node_modules/.bin/expo prebuild --clean --platform android --no-install); \
	ANDROID_HOME="$${ANDROID_HOME:-$$HOME/Android/Sdk}" ANDROID_SDK_ROOT="$${ANDROID_SDK_ROOT:-$${ANDROID_HOME:-$$HOME/Android/Sdk}}" \
		"$$fixture_parent/fixture/mobile/android/gradlew" -p "$$fixture_parent/fixture/mobile/android" --no-daemon --stacktrace assembleDebug

expo-sqlite-conformance:
	./examples/expo-sqlite-conformance/scripts/run-android.sh

expo-notifications-conformance:
	./examples/expo-notifications-conformance/scripts/run-android.sh

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

ts-browser-deps: ts-install
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/data-contracts-dexie exec playwright install --with-deps chromium webkit

ts-browser-test: ts-install
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/data-contracts-dexie exec playwright install chromium webkit
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/data-contracts-dexie run test:browser
	PLAYWRIGHT_BROWSERS_PATH="$(CURDIR)/$(TS_DIR)/.playwright-browsers" corepack pnpm --dir $(TS_DIR) --filter @baukit/navigation run test:browser

ts-check: ts-install
	corepack pnpm --dir $(TS_DIR) run check
