.PHONY: check install

check:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test

install:
	cargo install --path . --locked
