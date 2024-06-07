_list:
    @just --list

# Format workspace.
fmt:
    cargo +nightly fmt
    npx -y prettier --write $(fd --type=file --hidden --extension=md --extension=yml)

# Downgrade dev-dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:
    cargo update -p=clap --precise=4.4.18

msrv := ```
    cargo metadata --format-version=1 \
    | jq -r 'first(.packages[] | select(.source == null and .rust_version)) | .rust_version' \
    | sed -E 's/^1\.([0-9]{2})$/1\.\1\.0/'
```
msrv_rustup := "+" + msrv

non_linux_all_features_list := ```
    cargo metadata --format-version=1 \
    | jq '.packages[] | select(.source == null) | .features | keys' \
    | jq -r --slurp \
        --arg exclusions "tokio-uring,io-uring,experimental-io-uring" \
        'add | unique | . - ($exclusions | split(",")) | join(",")'
```

all_crate_features := if os() == "linux" {
    "--all-features"
} else {
    "--features='" + non_linux_all_features_list + "'"
}

# Run Clippy over workspace.
clippy toolchain="":
    cargo {{ toolchain }} clippy --workspace --all-targets {{ all_crate_features }}

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv (test msrv_rustup)

# Test workspace code.
test toolchain="":
    cargo {{ toolchain }} test --lib --tests -p=actix-web-codegen --all-features
    cargo {{ toolchain }} test --lib --tests -p=actix-multipart-derive --all-features
    cargo {{ toolchain }} nextest run -p=actix-router --no-default-features
    cargo {{ toolchain }} nextest run --workspace --exclude=actix-web-codegen --exclude=actix-multipart-derive {{ all_crate_features }} --filter-expr="not test(test_reading_deflate_encoding_large_random_rustls)"

# Test workspace docs.
test-docs toolchain="": && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all toolchain="": (test toolchain) (test-docs toolchain)

# Document crates in workspace.
doc *args:
    RUSTDOCFLAGS="--cfg=docsrs -Dwarnings" cargo +nightly doc --no-deps --workspace {{ all_crate_features }} {{ args }}

# Document crates in workspace and watch for changes.
doc-watch:
    @just doc --open
    cargo watch -- just doc

# Update READMEs from crate root documentation.
update-readmes: && fmt
    cd ./actix-files && cargo rdme --force
    cd ./actix-router && cargo rdme --force
