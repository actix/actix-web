.PHONY: default build test doc clean

CARGO_FLAGS := --features "$(FEATURES)"

default: test

build:
	cargo build $(CARGO_FLAGS)

test: build clippy
	cargo test $(CARGO_FLAGS)

skeptic:
	USE_SKEPTIC=1 cargo test $(CARGO_FLAGS)

# cd examples/word-count && python setup.py install && pytest -v tests

clippy:
	if $$CLIPPY; then cargo clippy $(CARGO_FLAGS); fi

doc: build
	cargo doc --no-deps $(CARGO_FLAGS)
