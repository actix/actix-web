//! Minimal file server: `cargo run --release -p actix-http --example sendfile -- FILE [sendfile|buffered] [PORT]`.
//!
//! One worker, HTTP/1, plain TCP. Both modes open the same file per request. Keep the file unchanged
//! while serving it. Use `RUST_LOG=actix_http::h1::sendfile=trace` to see actual syscall results;
//! leave tracing disabled during benchmarks.
//!
//! From the workspace root, build and compare both modes:
//!
//! ```sh
//! cargo build --release -p actix-http --example sendfile
//! dd if=/dev/urandom of=/tmp/actix-sendfile.bin bs=1048576 count=8
//! bash actix-http/examples/sendfile-bench.sh /tmp/actix-sendfile.bin
//! ```
//!
//! The script requires `oha`, `curl`, and `jq`. It checks response contents, warms the cache, and
//! alternates mode order. Defaults: five five-second trials per mode, 16 connections, two client
//! threads. Set `RUNS`, `DURATION`, `CONNECTIONS`, `CLIENT_THREADS`, or `PORT` to change these.
//! Raw results and a median summary go in `target/sendfile-benchmark/results` by default.
//! This compares the buffered fallback with direct transfer, not the full `actix-files` service.
//!
//! To test and benchmark Linux from macOS, run:
//!
//! ```sh
//! bash actix-http/examples/sendfile-docker.sh
//! ```
//!
//! This builds a tools image and runs the current checkout in Docker. Source files are mounted
//! read-only. Linux build and registry caches use separate Docker volumes. The same socket tests
//! run under `strace`, followed by benchmarks for 1 MiB and 8 MiB files generated inside Linux.
//! Results and syscall traces are written to `target/sendfile-linux-results`. An optional first
//! argument selects another result directory. The benchmark environment variables also apply.

use std::{fs::File, io, sync::Arc};

use actix_http::{
    body::{BoxBody, FileBody},
    HttpService, Request, Response, StatusCode,
};
use actix_server::Server;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init();

    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or_else(|| io::Error::other("usage: sendfile FILE [sendfile|buffered] [PORT]"))?;
    let mode = args.next().unwrap_or_else(|| "sendfile".into());
    let buffered = match mode.as_str() {
        "sendfile" => false,
        "buffered" => true,
        _ => return Err(io::Error::other("mode must be sendfile or buffered")),
    };
    if !buffered && !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err(io::Error::other(
            "this sendfile implementation requires macOS or Linux",
        ));
    }
    let port = args
        .next()
        .unwrap_or_else(|| "8080".into())
        .parse::<u16>()
        .map_err(io::Error::other)?;
    FileBody::new(File::open(&path)?)?;
    let path = Arc::new(path);

    eprintln!(
        "Serving {} on http://127.0.0.1:{port}/ ({mode}, one worker)",
        path
    );

    Server::build()
        .workers(1)
        .bind("sendfile", ("127.0.0.1", port), move || {
            let path = Arc::clone(&path);

            HttpService::build()
                .h1(move |_: Request| {
                    let path = Arc::clone(&path);

                    async move {
                        let body =
                            File::open(path.as_str())
                                .and_then(FileBody::new)
                                .map_err(|err| {
                                    Response::new(StatusCode::INTERNAL_SERVER_ERROR)
                                        .set_body(BoxBody::new(err.to_string()))
                                })?;
                        let body = if buffered { body.buffered() } else { body };

                        Ok::<_, Response<BoxBody>>(Response::ok().set_body(body))
                    }
                })
                .tcp()
        })?
        .run()
        .await
}
