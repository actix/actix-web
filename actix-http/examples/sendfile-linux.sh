#!/usr/bin/env bash
# Invoked inside the Linux container by sendfile-docker.sh.
set -euo pipefail

out=${RESULTS_DIR:-/results}
mkdir -p "$out"
{
    uname -a
    rustc --version
    oha --version
} >"$out/environment.txt"

cargo check --locked -p actix-http --no-default-features
cargo test --locked -p actix-http --lib --test test_server

# Trace the actual socket tests. DirectOnly rejects the buffered fallback, and strace independently
# records kernel calls. No tracing runs during the throughput benchmark.
test_binary=$(cargo test --locked -p actix-http --test test_sendfile --no-run --message-format=json \
    | jq -r 'select(.reason == "compiler-artifact" and .target.name == "test_sendfile") | .executable // empty')
strace -f -e trace=sendfile -o "$out/sendfile.strace" "$test_binary" --test-threads=1
rg -q 'sendfile\(.* = [1-9][0-9]*$|<\.\.\. sendfile resumed>.* = [1-9][0-9]*$' "$out/sendfile.strace"

cargo build --locked --release -p actix-http --example sendfile
export BINARY="$CARGO_TARGET_DIR/release/examples/sendfile"

# Generate files on the container filesystem, not a macOS bind mount.
fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT
for size in 1 8; do
    dd if=/dev/urandom of="$fixture_dir/file.bin" bs=1048576 count="$size" status=none
    bash actix-http/examples/sendfile-bench.sh "$fixture_dir/file.bin" "$out/${size}m"
done
