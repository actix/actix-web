FROM ghcr.io/hatoo/oha:1.16.0 AS load-generator
FROM rust:1.98-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates cmake curl jq libssl-dev pkg-config ripgrep strace \
    && rm -rf /var/lib/apt/lists/*

COPY --from=load-generator /usr/bin/oha /usr/local/bin/oha
RUN oha --version

ENV CARGO_TARGET_DIR=/target
WORKDIR /workspace
