.PHONY: default build test doc book clean

CARGO_FLAGS := --features "$(FEATURES) alpn"

default: test

build:
	cargo build $(CARGO_FLAGS)

test: build clippy
	cargo test $(CARGO_FLAGS)

doc: build
	cargo doc --no-deps $(CARGO_FLAGS)
