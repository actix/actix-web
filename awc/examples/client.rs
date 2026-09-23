//! Demonstrates construction and usage of a TLS-capable HTTP client.

extern crate tls_rustls_0_23 as rustls;

use std::{error::Error as StdError, sync::Arc};

use rustls::ClientConfig;
use rustls_platform_verifier::ConfigVerifierExt as _;

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn StdError>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let mut config = ClientConfig::with_platform_verifier();

    let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.alpn_protocols = protos;

    // construct request builder with TLS support
    let client = awc::Client::builder()
        .connector(awc::Connector::new().rustls_0_23(Arc::new(config)))
        .finish();

    // configure request
    let request = client
        .get("https://www.rust-lang.org/")
        .append_header(("User-Agent", "awc/3.0"));

    println!("Request: {request:?}");

    let mut response = request.send().await?;

    // server response head
    println!("Response: {response:?}");

    // read response body
    let body = response.body().await?;
    println!("Downloaded: {:?} bytes", body.len());

    Ok(())
}
