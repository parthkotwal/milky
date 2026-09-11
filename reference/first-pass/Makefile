# Milky dev commands. Rust builds first: the Swift package links its staticlib.

RUST_DIR   := rust
SWIFT_DIR  := apps/macos
PROFILE    ?= debug
CARGO_FLAG := $(if $(filter release,$(PROFILE)),--release,)

.PHONY: all rust swift build test test-rust selftest probe cli bench fmt lint clean

all: build

## Build the Rust workspace (engine, C ABI staticlib, dev CLI).
rust:
	cd $(RUST_DIR) && cargo build $(CARGO_FLAG)

## Build the Swift package. Depends on the staticlib existing.
swift: rust
	cd $(SWIFT_DIR) && MILKY_RUST_PROFILE=$(PROFILE) swift build

build: swift

test: test-rust selftest

test-rust:
	cd $(RUST_DIR) && cargo test $(CARGO_FLAG)

## Assertions across the FFI boundary. An executable, not `swift test`, because
## XCTest ships with Xcode and this machine has CLT only (.agents/ISSUES.md).
selftest: rust
	cd $(SWIFT_DIR) && MILKY_RUST_PROFILE=$(PROFILE) swift run milky-selftest

## End-to-end check: Swift -> C ABI -> Rust engine -> JSON -> back.
probe: rust
	cd $(SWIFT_DIR) && MILKY_RUST_PROFILE=$(PROFILE) swift run milky-probe "$(if $(Q),$(Q),terminal)"

## Drive the engine headlessly, no app build. Example: make cli ARGS='search terminal'
cli:
	cd $(RUST_DIR) && cargo run $(CARGO_FLAG) --bin milky -- $(ARGS)

## Engine-side latency. Always measure in release. Quote Q: make splits on spaces.
bench:
	cd $(RUST_DIR) && cargo run --release --bin milky -- bench "$(if $(Q),$(Q),terminal)"

fmt:
	cd $(RUST_DIR) && cargo fmt
	cd $(SWIFT_DIR) && swift format --in-place --recursive Sources Tests 2>/dev/null || true

lint:
	cd $(RUST_DIR) && cargo clippy --all-targets -- -D warnings

clean:
	cd $(RUST_DIR) && cargo clean
	rm -rf $(SWIFT_DIR)/.build
