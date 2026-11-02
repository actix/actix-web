#!/usr/bin/env bash
# Run from the workspace root. Build the release example first.
set -euo pipefail

file=${1:?usage: bash actix-http/examples/sendfile-bench.sh FILE [OUTPUT_DIR]}
out=${2:-target/sendfile-benchmark/results}
binary=${BINARY:-target/release/examples/sendfile}
port=${PORT:-18080}
runs=${RUNS:-5}
duration=${DURATION:-5s}
connections=${CONNECTIONS:-16}
client_threads=${CLIENT_THREADS:-2}
mkdir -p "$out"
server_pid=
cleanup() {
    if [[ -n "$server_pid" ]]; then
        kill -INT "$server_pid" 2>/dev/null || true
        wait "$server_pid" || true
        server_pid=
    fi
}
trap cleanup EXIT INT TERM

for ((run = 1; run <= runs; run++)); do
    # Alternate order to reduce systematic warmup and thermal bias.
    modes="buffered sendfile"
    if ((run % 2 == 0)); then modes="sendfile buffered"; fi
    for mode in $modes; do
        RUST_LOG=off "$binary" "$file" "$mode" "$port" >"$out/$mode-$run.server.log" 2>&1 &
        server_pid=$!
        ready=false
        for ((attempt = 0; attempt < 100; attempt++)); do
            if ! kill -0 "$server_pid" 2>/dev/null; then
                cat "$out/$mode-$run.server.log" >&2
                exit 1
            fi
            if curl --fail --silent "http://127.0.0.1:$port/" -o "$out/received.bin"; then
                ready=true
                break
            fi
            sleep 0.1
        done
        if [[ "$ready" != true ]]; then exit 1; fi
        cmp "$file" "$out/received.bin"
        oha --no-tui --worker-threads "$client_threads" --output-format quiet -z 1s -w -c "$connections" "http://127.0.0.1:$port/"
        oha --no-tui --worker-threads "$client_threads" --output-format json -z "$duration" -w -c "$connections" \
            "http://127.0.0.1:$port/" >"$out/$mode-$run.json"
        cleanup
        jq -e '.summary.successRate == 1 and (.statusCodeDistribution | keys == ["200"])' "$out/$mode-$run.json" >/dev/null
        jq -r --arg mode "$mode" --arg run "$run" \
            '[$mode, $run, .summary.requestsPerSec, .summary.sizePerSec] | @tsv' "$out/$mode-$run.json"
    done
done
rm "$out/received.bin"

for mode in buffered sendfile; do
    paths=()
    for ((run = 1; run <= runs; run++)); do
        paths+=("$out/$mode-$run.json")
    done
    jq -s --arg mode "$mode" '
        def median: sort | if length % 2 == 1 then .[length / 2 | floor]
            else (.[length / 2 - 1] + .[length / 2]) / 2 end;
        {mode: $mode, trials: length,
         median_requests_per_second: (map(.summary.requestsPerSec) | median),
         median_mib_per_second: (map(.summary.sizePerSec / 1048576) | median),
         min_mib_per_second: (map(.summary.sizePerSec / 1048576) | min),
         max_mib_per_second: (map(.summary.sizePerSec / 1048576) | max)}
    ' "${paths[@]}"
done | jq -s '.' >"$out/summary.json"
cat "$out/summary.json"
