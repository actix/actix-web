_list:
    @just --list

# Format workspace.
fmt:
    cargo +nightly fmt
    npx -y prettier --write $(fd --type=file --hidden --extension=md --extension=yml)

# Document crates in workspace.
doc:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

# Document crates in workspace and watch for changes.
doc-watch:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl --open
    cargo watch -- RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl
