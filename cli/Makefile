.PHONY: check install

check:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test

install:
	cargo install --git https://github.com/PatrickKoss/baukit --tag v$(shell cat ../templates/VERSION) --locked baukit-cli
