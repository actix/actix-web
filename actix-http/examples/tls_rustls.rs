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

extern crate tls_rustls_021 as rustls;

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
                .rustls_021(rustls_config())
        })?
        .run()
        .await
}

fn rustls_config() -> rustls::ServerConfig {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_file = cert.serialize_pem().unwrap();
    let key_file = cert.serialize_private_key_pem();

    let cert_file = &mut io::BufReader::new(cert_file.as_bytes());
    let key_file = &mut io::BufReader::new(key_file.as_bytes());

    let cert_chain = rustls_pemfile::certs(cert_file)
        .unwrap()
        .into_iter()
        .map(rustls::Certificate)
        .collect();
    let mut keys = rustls_pemfile::pkcs8_private_keys(key_file).unwrap();

    let mut config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, rustls::PrivateKey(keys.remove(0)))
        .unwrap();

    const H1_ALPN: &[u8] = b"http/1.1";
    const H2_ALPN: &[u8] = b"h2";

    config.alpn_protocols.push(H2_ALPN.to_vec());
    config.alpn_protocols.push(H1_ALPN.to_vec());

    config
}
