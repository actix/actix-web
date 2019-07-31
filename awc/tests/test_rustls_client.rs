#![cfg(feature = "rust-tls")]
use rustls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    ClientConfig, NoClientAuth,
};

use std::fs::File;
use std::io::{BufReader, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_server::ssl::RustlsAcceptor;
use actix_service::{service_fn, NewService};
use actix_web::http::Version;
use actix_web::{web, App, HttpResponse};

fn ssl_acceptor<T: AsyncRead + AsyncWrite>() -> Result<RustlsAcceptor<T, ()>> {
    use rustls::ServerConfig;
    // load ssl keys
    let mut config = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("../tests/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("../tests/key.pem").unwrap());
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();
    let protos = vec![b"h2".to_vec()];
    config.set_protocols(&protos);
    Ok(RustlsAcceptor::new(config))
}

mod danger {
    pub struct NoCertificateVerification {}

    impl rustls::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _roots: &rustls::RootCertStore,
            _presented_certs: &[rustls::Certificate],
            _dns_name: webpki::DNSNameRef<'_>,
            _ocsp: &[u8],
        ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
            Ok(rustls::ServerCertVerified::assertion())
        }
    }
}

#[test]
fn test_connection_reuse_h2() {
    let rustls = ssl_acceptor().unwrap();
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(rustls.clone().map_err(|e| println!("Rustls error: {}", e)))
        .and_then(
            HttpService::build()
                .h2(App::new()
                    .service(web::resource("/").route(web::to(|| HttpResponse::Ok()))))
                .map_err(|_| ()),
        )
    });

    // disable ssl verification
    let mut config = ClientConfig::new();
    let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.set_protocols(&protos);
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification {}));

    let client = awc::Client::build()
        .connector(awc::Connector::new().rustls(Arc::new(config)).finish())
        .finish();

    // req 1
    let request = client.get(srv.surl("/")).send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.surl("/"));
    let response = srv.block_on_fn(move || req.send()).unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);

    // one connection
    assert_eq!(num.load(Ordering::Relaxed), 1);
}
