# Signals

This example shows how to handle Unix signals and properly stop http server. This example does not work with Windows.

## Usage

```bash
cd actix-web/examples/signal
cargo run (or ``cargo watch -x run``)
# Started http server: 127.0.0.1:8080
# CTRL+C
# INFO:actix_web::server: SIGINT received, exiting
# INFO:actix_web::worker: Shutting down http worker, 0 connections
# INFO:actix_web::worker: Shutting down http worker, 0 connections
# INFO:actix_web::worker: Shutting down http worker, 0 connections
# INFO:actix_web::worker: Shutting down http worker, 0 connections
```