.PHONY: build test check clippy fmt clean wasm wasm-build uniffi-kotlin uniffi-swift

# Default target
all: build test

# Build the library (no features)
build:
	cargo build

# Build with CLI binary
build-cli:
	cargo build --features cli

# Run all tests (FakeRpcClient — no CKB node needed)
test:
	cargo test

# Run tests with output shown
test-verbose:
	cargo test -- --nocapture

# Run a single test (usage: make test-one TEST=test_get_tip_block)
test-one:
	cargo test $(TEST) -- --nocapture

# Type-check all feature combinations
check:
	cargo check
	cargo check --features cli
	cargo check --features wasm
	cargo check --features uniffi

# Lint
clippy:
	cargo clippy --no-default-features
	cargo clippy --all-features

# Format check
fmt:
	cargo fmt --check

# Format fix
fmt-fix:
	cargo fmt

# WASM build (requires wasm-pack: cargo install wasm-pack)
wasm-build:
	wasm-pack build --features wasm --target web

# WASM build for Node.js
wasm-build-node:
	wasm-pack build --features wasm --target nodejs

# Check WASM compilation only
wasm-check:
	cargo check --target wasm32-unknown-unknown --features wasm

# Uniffi: generate Kotlin bindings for Android
uniffi-kotlin:
	uniffi-bindgen generate uniffi/opticrum_sdk.udl --language kotlin --out-dir uniffi/generated/kotlin

# Uniffi: generate Swift bindings for iOS
uniffi-swift:
	uniffi-bindgen generate uniffi/opticrum_sdk.udl --language swift --out-dir uniffi/generated/swift

# Clean build artifacts
clean:
	cargo clean
	rm -rf build/
	rm -rf uniffi/generated/
