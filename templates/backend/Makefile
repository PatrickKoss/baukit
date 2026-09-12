.PHONY: setup preflight fmt lint test test-scripts check{% if context.web %} check-web{% endif %}{% if context.mobile %} check-mobile qa-setup qa-android-setup qa-ios-setup qa-android-build qa-ios-build qa-android qa-ios e2e-mobile e2e-mobile-live e2e-android e2e-android-live e2e-ios e2e-ios-live qa-android-down qa-ios-down qa-down{% endif %} lockfiles run{% if context.worker %} run-worker{% endif %} migrate openapi openapi-client db-up db-down{% if context.auth_oidc %} dev keycloak-policy keycloak-reconcile keycloak-test keycloak-theme-test{% if context.web %} keycloak-theme-browser-test{% endif %}{% endif %}

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

qa-setup: qa-android-setup

qa-android-setup:
	bash mobile/scripts/qa/android-sdk.sh

qa-ios-setup:
	bash mobile/scripts/qa/ios-simulator.sh prepare

qa-android-build: qa-android-setup
	bash mobile/scripts/qa/build-android.sh

qa-ios-build: qa-ios-setup
	bash mobile/scripts/qa/build-ios.sh

qa-android: qa-android-setup
	bash mobile/scripts/qa/android-env.sh start

qa-ios: qa-ios-setup
	bash mobile/scripts/qa/ios-env.sh start

e2e-android-live:
	bash mobile/scripts/qa/run-maestro.sh android

e2e-ios-live:
	bash mobile/scripts/qa/run-maestro.sh ios

e2e-mobile-live: e2e-android-live

e2e-android:
	@set -eu; \
		cleanup() { $(MAKE) --no-print-directory qa-android-down; }; \
		trap cleanup EXIT INT TERM; \
		$(MAKE) --no-print-directory qa-android; \
		$(MAKE) --no-print-directory e2e-android-live

e2e-ios:
	@set -eu; \
		cleanup() { $(MAKE) --no-print-directory qa-ios-down; }; \
		trap cleanup EXIT INT TERM; \
		$(MAKE) --no-print-directory qa-ios; \
		$(MAKE) --no-print-directory e2e-ios-live

e2e-mobile: e2e-android

qa-android-down:
	bash mobile/scripts/qa/android-env.sh stop

qa-ios-down:
	bash mobile/scripts/qa/ios-env.sh stop

qa-down:
	@bash mobile/scripts/qa/android-env.sh stop
	@bash mobile/scripts/qa/ios-env.sh stop

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
