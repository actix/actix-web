//! Signal startup to an external test harness through stdout.
//!
//! Run with: `cargo run -p actix-web --example startup-ready`
//! Wait for `READY <url>` before sending requests to that URL.
//!
//! This example relies on actix-server completing startup during the first poll.
//! This is current implementation behavior, not an explicit readiness API contract.

use std::time::Duration;

use actix_web::{http::StatusCode, web, App, HttpServer};
use futures_util::FutureExt as _;
use tokio::time::timeout;

#[tokio::main(flavor = "local")]
async fn main() -> std::io::Result<()> {
    let server =
        HttpServer::new(|| App::new().route("/", web::get().to(|| async { "Hello world!" })))
            .workers(1)
            .bind(("127.0.0.1", 0))?;

    let addr = server.addrs()[0];
    let mut server = server.run();

    let client = awc::Client::new();
    let url = format!("http://{addr}/");

    // The socket is bound, but the server cannot respond before its first poll.
    println!("Sending a request before startup; expecting a timeout...");
    let err = timeout(Duration::from_secs(2), client.get(&url).send())
        .await
        .unwrap_err();
    println!("Request timed out as expected: {err}");

    // Poll in this task so that startup runs before we signal readiness.
    // Return startup errors without printing READY. Keep the future for later polls.
    if let Some(result) = (&mut server).now_or_never() {
        return result;
    }

    println!("READY http://{addr}");

    println!("Sending a request after startup; expecting 200 OK and the complete body...");
    let mut response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().await.unwrap(), "Hello world!");

    // Continue awaiting the server as normal to process commands and shutdown signals.
    println!("Continuing to await the server as normal. Press Ctrl-C to stop.");
    server.await
}
