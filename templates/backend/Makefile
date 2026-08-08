.PHONY: fmt lint test check run migrate openapi openapi-client db-up db-down

BACKEND_MANIFEST := backend/Cargo.toml

fmt:
	cargo fmt --manifest-path $(BACKEND_MANIFEST) --all --check

lint:
	cargo clippy --manifest-path $(BACKEND_MANIFEST) --all-targets -- -D warnings

test:
	cargo test --manifest-path $(BACKEND_MANIFEST)

check: fmt lint test

run:
	cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin api

migrate:
	cargo run --manifest-path $(BACKEND_MANIFEST) -p {{ context.app_name }}-bin --bin migrate

openapi:
	sh scripts/openapi.sh

openapi-client:
	baukit generate openapi-client

db-up:
	docker compose up -d postgres

db-down:
	docker compose down
