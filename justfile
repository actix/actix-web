_list:
    @just --list

# Document crates in workspace.
doc:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

# Document crates in workspace and watch for changes.
doc-watch:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl --open
    cargo watch -- RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

check-external-types-all:
    #!/usr/bin/env bash
    set -euxo pipefail
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml); do
        cargo +nightly check-external-types --manifest-path "$f"
    done
