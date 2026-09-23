.PHONY: all help build release test fmt fmt-check clippy audit md-lint expected expected-check hooks check clean

all: check ## Default: run the local check suite

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-14s %s\n", $$1, $$2}'

build: ## Debug build
	cargo build

release: ## Release build (what the herdr manifest runs)
	cargo build --release

test: ## Unit tests and the fixture suite
	cargo test

fmt: ## Format sources
	cargo fmt --all

fmt-check: ## Fail if sources are not formatted
	cargo fmt --all --check

clippy: ## Lint, warnings are errors
	cargo clippy --all-targets -- -D warnings

audit: ## Check dependencies against the RustSec advisory database (needs cargo-audit)
	cargo audit

md-lint: ## Lint markdown
	markdownlint-cli2

expected: ## Regenerate tests/expected/ from the fixtures
	python3 scripts/expected.py

expected-check: ## Fail if tests/expected/ does not match the fixtures
	python3 scripts/expected.py --check

hooks: ## Install pre-commit hooks
	pre-commit install

check: fmt-check clippy test audit md-lint expected-check ## Local suite, same as CI

clean: ## Remove build output
	cargo clean
