.PHONY: fmt lint test check{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %} lockfiles run migrate openapi openapi-client db-up db-down

BACKEND_MANIFEST := backend/Cargo.toml

fmt:
	cargo fmt --manifest-path $(BACKEND_MANIFEST) --all --check

lint:
	cargo clippy --manifest-path $(BACKEND_MANIFEST) --all-targets -- -D warnings

test:
	cargo test --manifest-path $(BACKEND_MANIFEST) -- --include-ignored

check: fmt lint test{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %}

{% if context.web %}check-web:
	cd web && corepack pnpm@11.18.0 install --frozen-lockfile
	cd web && corepack pnpm@11.18.0 build
	cd web && corepack pnpm@11.18.0 lint
	cd web && corepack pnpm@11.18.0 test

{% endif %}{% if context.mobile %}check-mobile:
	cd mobile && corepack pnpm@11.18.0 install --frozen-lockfile
	cd mobile && corepack pnpm@11.18.0 exec tsc --noEmit
	cd mobile && corepack pnpm@11.18.0 lint
	cd mobile && corepack pnpm@11.18.0 test

{% endif %}run:
	cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin api

migrate:
	cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin migrate

openapi:
	sh scripts/openapi.sh

openapi-client:
	sh scripts/openapi-client.sh

db-up:
	docker compose up -d postgres

db-down:
	docker compose down

lockfiles:
	sh scripts/lockfiles.sh
