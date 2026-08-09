.PHONY: fmt lint test check ci platform-validate platform-up platform-down platform-nuke platform-recreate platform-status ts-install ts-build ts-fmt ts-lint ts-test ts-check cli-fmt cli-lint cli-test cli-check cli-ci install-skills

RUST_MANIFEST := rust/Cargo.toml
TS_DIR := typescript
CLI_MANIFEST := cli/Cargo.toml

fmt: ts-fmt
	cargo fmt --manifest-path $(RUST_MANIFEST) --all --check

lint: ts-lint
	cargo clippy --manifest-path $(RUST_MANIFEST) --all-targets -- -D warnings

test: ts-test
	cargo test --manifest-path $(RUST_MANIFEST)

check: ts-check
	cargo check --manifest-path $(RUST_MANIFEST) --workspace --all-targets

ci: fmt lint test check ts-check cli-ci platform-validate

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

install-skills:
	@test -n "$(TARGET)" || (echo "TARGET is required: make install-skills TARGET=<product-dir>" >&2; exit 2)
	./agent-skills/install.sh --target "$(TARGET)"

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

ts-check: ts-install
	corepack pnpm --dir $(TS_DIR) run check
