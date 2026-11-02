#!/usr/bin/env bash
# Build the Linux tools image, then test and benchmark the current checkout.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-"$root/target/sendfile-linux-results"}
mkdir -p "$out"
out=$(cd "$out" && pwd)
image=actix-http-sendfile-linux:local

# The Dockerfile only installs tools. Sending it on stdin avoids uploading the workspace or target.
docker build -t "$image" - <"$root/actix-http/examples/sendfile.Dockerfile"
docker run --rm --init \
    --mount "type=bind,source=$root,target=/workspace,readonly" \
    --mount "type=bind,source=$out,target=/results" \
    --mount type=volume,source=actix-http-sendfile-linux-target,target=/target \
    --mount type=volume,source=actix-http-sendfile-linux-registry,target=/usr/local/cargo/registry \
    -e RUNS="${RUNS:-3}" -e DURATION="${DURATION:-5s}" \
    -e CONNECTIONS="${CONNECTIONS:-16}" -e CLIENT_THREADS="${CLIENT_THREADS:-2}" \
    "$image" bash actix-http/examples/sendfile-linux.sh
