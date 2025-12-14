_list:
    @just --list

toolchain := ""

# Format workspace.
fmt:
    just --unstable --fmt
    cargo +nightly fmt
    fd --hidden --type=file --extension=md --extension=yml --exec-batch npx -y prettier --write

# Downgrade dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:
    # no downgrades currently needed

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
        --arg exclusions "__tls,__compress,tokio-uring,io-uring,experimental-io-uring" \
        'add | unique | . - ($exclusions | split(",")) | join(",")'
```
all_crate_features := if os() == "linux" { "--all-features" } else { "--features='" + non_linux_all_features_list + "'" }

[private]
check-min:
    cargo hack --workspace check --no-default-features

[private]
check-default:
    cargo hack --workspace check

# Check workspace.
check: && clippy
    fd --hidden --type=file --extension=md --extension=yml --exec-batch npx -y prettier --check

# Run Clippy over workspace.
clippy:
    cargo {{ toolchain }} clippy --workspace --all-targets {{ all_crate_features }}

# Run Clippy over workspace using MSRV.
clippy-msrv: downgrade-for-msrv
    @just toolchain={{ msrv_rustup }} clippy

# Test workspace code.
test:
    cargo {{ toolchain }} test --lib --tests -p=actix-web-codegen --all-features
    cargo {{ toolchain }} test --lib --tests -p=actix-multipart-derive --all-features
    cargo {{ toolchain }} nextest run --no-tests=warn -p=actix-router --no-default-features
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-web-codegen --exclude=actix-multipart-derive {{ all_crate_features }} --filter-expr="not test(test_reading_deflate_encoding_large_random_rustls)"

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv
    @just toolchain={{ msrv_rustup }} test

# Test workspace docs.
test-docs: && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all: test test-docs

# Test workspace and collect coverage info.
[private]
test-coverage:
    cargo {{ toolchain }} llvm-cov nextest --no-tests=warn --no-report {{ all_crate_features }}
    cargo {{ toolchain }} llvm-cov --doc --no-report {{ all_crate_features }}

# Test workspace and generate Codecov report.
test-coverage-codecov: test-coverage
    cargo {{ toolchain }} llvm-cov report --doctests --codecov --output-path=codecov.json

# Test workspace and generate LCOV report.
test-coverage-lcov: test-coverage
    cargo {{ toolchain }} llvm-cov report --doctests --lcov --output-path=lcov.info

# Document crates in workspace.
# FIXME: Re-add `RUSTDOCFLAGS="--cfg=docsrs -Dwarnings"` once crypto-related crates are updated.
doc *args: && doc-set-workspace-crates
    rm -f "$(cargo metadata --format-version=1 | jq -r '.target_directory')/doc/crates.js"
    cargo +nightly doc --workspace {{ all_crate_features }} {{ args }}

[private]
doc-set-workspace-crates:
    #!/usr/bin/env bash
    (
        echo "window.ALL_CRATES ="
        cargo metadata --format-version=1 \
        | jq '[.packages[] | select(.source == null) | .targets | map(select(.doc) | .name)] | flatten'
        echo ";"
    ) > "$(cargo metadata --format-version=1 | jq -r '.target_directory')/doc/crates.js"

# Document crates in workspace and watch for changes.
doc-watch:
    @just doc --open
    cargo watch -- just doc

# Update READMEs from crate root documentation.
update-readmes: && fmt
    cd ./actix-files && cargo rdme --force
    cd ./actix-http-test && cargo rdme --force
    cd ./actix-router && cargo rdme --force
    cd ./actix-multipart && cargo rdme --force
    cd ./actix-test && cargo rdme --force

feature_combo_skip_list := if os() == "linux" { "__tls,__compress" } else { "__tls,__compress,experimental-io-uring" }

# Checks compatibility of feature combinations.
check-feature-combinations:
    cargo hack --workspace \
        --feature-powerset --depth=4 \
        --skip={{ feature_combo_skip_list }} \
        check

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all toolchain="+nightly":
    #!/usr/bin/env bash
    set -euo pipefail
    exit=0
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        if ! just check-external-types-manifest "$f" {{ toolchain }}; then exit=1; fi
        echo
        echo
    done
    exit $exit

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all-table toolchain="+nightly":
    #!/usr/bin/env bash
    set -euo pipefail
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        echo
        echo "Checking for $f"
        just check-external-types-manifest "$f" {{ toolchain }} --output-format=markdown-table
    done

# Check for unintentional external type exposure on a crate.
check-external-types-manifest manifest_path toolchain="+nightly" *extra_args="":
    cargo {{ toolchain }} check-external-types --manifest-path "{{ manifest_path }}" {{ extra_args }}
