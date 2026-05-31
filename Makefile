.PHONY: all build build-release release install install-serve clean test test-unit test-integration check fmt lint run serve demo

ARGS ?=
LEVEL ?= minor

# Default target
all: check build test

# Build debug version
build:
	cargo build

# Build release version
build-release:
	cargo build --release

# Publish a new release (usage: make release or make release LEVEL=patch)
release:
	cargo release $(LEVEL) --execute --no-confirm

# Install to ~/.cargo/bin
install:
	CARGO_INCREMENTAL=0 cargo install --path . --locked --bins

# Install with network streaming (ascii-cam serve)
install-serve:
	CARGO_INCREMENTAL=0 cargo install --path . --locked --bins --features serve

# Stream over HTTP (usage: make serve ARGS="--token mytoken")
serve:
	cargo run --features serve -- serve $(ARGS)

# Clean build artifacts
clean:
	cargo clean

# Run all tests
test:
	cargo test
	cargo test --features serve

# Run library and binary tests only
test-unit:
	cargo test --lib --bins

# Run integration tests only
test-integration:
	cargo test --tests

# Run check, tests, and clippy
check:
	cargo check
	cargo check --features serve
	cargo test
	cargo test --features serve
	cargo clippy --all-targets --all-features -- -D warnings

# Format code
fmt:
	cargo fmt --all

# Lint (check formatting and clippy)
lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings

# Run with arguments (usage: make run ARGS="--resolution low --no-color")
run:
	cargo run -- $(ARGS)

# Quick demo
demo: build
	@echo "=== ascii-cam demo ==="
	cargo run -- --help
