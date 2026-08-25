.PHONY: preflight fmt lint test check{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %} lockfiles run{% if context.worker %} run-worker{% endif %} migrate openapi openapi-client db-up db-down

BACKEND_MANIFEST := backend/Cargo.toml
PLAYWRIGHT_BROWSERS_PATH := $(abspath web/node_modules/.cache/playwright-browsers)
export PLAYWRIGHT_BROWSERS_PATH

preflight:
	sh scripts/preflight.sh

fmt: preflight
	cargo fmt --manifest-path $(BACKEND_MANIFEST) --all --check

lint: preflight
	cargo clippy --manifest-path $(BACKEND_MANIFEST) --all-targets -- -D warnings

test: preflight
	cargo test --manifest-path $(BACKEND_MANIFEST) -- --include-ignored

check: preflight fmt lint test{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %}

{% if context.web %}check-web: preflight
	cd web && corepack pnpm@11.18.0 install --frozen-lockfile
	cd web && corepack pnpm@11.18.0 build
	cd web && corepack pnpm@11.18.0 lint
	cd web && corepack pnpm@11.18.0 test

{% endif %}{% if context.mobile %}check-mobile: preflight
	cd mobile && corepack pnpm@11.18.0 install --frozen-lockfile
	cd mobile && corepack pnpm@11.18.0 exec tsc --noEmit
	cd mobile && corepack pnpm@11.18.0 lint
	cd mobile && corepack pnpm@11.18.0 test

{% endif %}run: preflight
	{% if context.port_offset > 0 %}{{ context.app_env }}__HTTP__PORT={{ context.api_host_port }} {{ context.app_env }}__OPS__PORT={{ context.ops_host_port }} {% endif %}cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin api

{% if context.worker %}run-worker: preflight
	{% if context.port_offset > 0 %}{{ context.app_env }}__OPS__PORT={{ context.ops_host_port }} {% endif %}cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin worker

{% endif %}
migrate: preflight
	cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin migrate

openapi: preflight
	sh scripts/openapi.sh

openapi-client:
	sh scripts/openapi-client.sh

db-up:
	docker compose up -d postgres

db-down:
	docker compose down

lockfiles: preflight
	sh scripts/lockfiles.sh
