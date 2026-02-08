//! Demonstrates TLS configuration (via Rustls) for HTTP/1.1 and HTTP/2 connections.
//!
//! Test using cURL:
//!
//! ```console
//! $ curl --insecure https://127.0.0.1:8443
//! Hello World!
//! Protocol: HTTP/2.0
//!
//! $ curl --insecure --http1.1 https://127.0.0.1:8443
//! Hello World!
//! Protocol: HTTP/1.1
//! ```

extern crate tls_rustls_023 as rustls;

use std::io;

use actix_http::{Error, HttpService, Request, Response};
use actix_utils::future::ok;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    tracing::info!("starting HTTP server at https://127.0.0.1:8443");

    actix_server::Server::build()
        .bind("echo", ("127.0.0.1", 8443), || {
            HttpService::build()
                .finish(|req: Request| {
                    let body = format!(
                        "Hello World!\n\
                        Protocol: {:?}",
                        req.head().version
                    );
                    ok::<_, Error>(Response::ok().set_body(body))
                })
                .rustls_0_23(rustls_config())
        })?
        .run()
        .await
}

fn rustls_config() -> rustls::ServerConfig {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_chain = vec![cert.der().clone()];
    let key_der = rustls_pki_types::PrivateKeyDer::Pkcs8(
        rustls_pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
    );

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)
        .unwrap();

    const H1_ALPN: &[u8] = b"http/1.1";
    const H2_ALPN: &[u8] = b"h2";

    config.alpn_protocols.push(H2_ALPN.to_vec());
    config.alpn_protocols.push(H1_ALPN.to_vec());

    config
}
