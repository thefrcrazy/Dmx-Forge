.PHONY: dev check test build fmt lint migrate docker-build

dev:
	cargo run -p dmxforge

check:
	cargo check --workspace

test:
	cargo test --workspace

build:
	cargo build --release -p dmxforge

migrate:
	cargo run -p dmxforge -- --migrate-only

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets -- -D warnings

docker-build:
	docker build -t dmxforge:local .
