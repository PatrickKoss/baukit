.PHONY: setup preflight fmt lint test test-scripts check{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %} lockfiles run{% if context.worker %} run-worker{% endif %} migrate openapi openapi-client db-up db-down{% if context.auth_oidc %} dev keycloak-policy keycloak-reconcile keycloak-test keycloak-theme-test{% if context.web %} keycloak-theme-browser-test{% endif %}{% endif %}

BACKEND_MANIFEST := backend/Cargo.toml
PLAYWRIGHT_BROWSERS_PATH := $(abspath web/node_modules/.cache/playwright-browsers)
export PLAYWRIGHT_BROWSERS_PATH

setup:
	sh scripts/setup.sh

preflight:
	sh scripts/preflight.sh

fmt: preflight
	cargo fmt --manifest-path $(BACKEND_MANIFEST) --all --check

lint: preflight
	cargo clippy --manifest-path $(BACKEND_MANIFEST) --all-targets -- -D warnings

test: preflight
	cargo test --manifest-path $(BACKEND_MANIFEST) -- --include-ignored

test-scripts:
	python3 scripts/reconcile-env.test.py
{% if context.auth_oidc %}	python3 -m unittest discover -s scripts/tests
	node --test scripts/tests/keycloak_accessibility.test.mjs
{% endif %}
{% if context.quality_strict %}	python3 scripts/check-markdown-links.test.py

{% endif %}check: preflight fmt lint test test-scripts{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile{% endif %}

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

{% if context.auth_oidc %}keycloak-policy:
	python3 scripts/keycloak_policy.py --environment-class development

keycloak-test:
	python3 -m unittest discover -s scripts/tests

keycloak-theme-test:
	node --test scripts/tests/keycloak_accessibility.test.mjs

{% if context.web %}keycloak-theme-browser-test: keycloak-theme-test
	cd web && corepack pnpm@11.18.0 install --frozen-lockfile
	cd web && corepack pnpm@11.18.0 exec playwright install chromium
	sh scripts/test-keycloak-theme-patches.sh

{% endif %}

keycloak-reconcile: keycloak-policy
	docker compose up -d --wait keycloak
	python3 scripts/reconcile_keycloak.py

dev: keycloak-reconcile

{% endif %}
db-down:
	docker compose down

lockfiles: preflight
	sh scripts/lockfiles.sh
