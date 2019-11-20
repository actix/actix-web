#![cfg(feature = "rustls")]
use rust_tls::ClientConfig;

use std::io::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::HttpService;
use actix_http_test::{block_on, TestServer};
use actix_server::ssl::OpensslAcceptor;
use actix_service::{pipeline_factory, ServiceFactory};
use actix_web::http::Version;
use actix_web::{web, App, HttpResponse};
use futures::future::ok;
use open_ssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};

fn ssl_acceptor<T: AsyncRead + AsyncWrite>() -> Result<OpensslAcceptor<T, ()>> {
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_verify_callback(SslVerifyMode::NONE, |_, _| true);
    builder
        .set_private_key_file("../tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("../tests/cert.pem")
        .unwrap();
    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(open_ssl::ssl::AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x02h2")?;
    Ok(actix_server::ssl::OpensslAcceptor::new(builder.build()))
}

mod danger {
    pub struct NoCertificateVerification {}

    impl rust_tls::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _roots: &rust_tls::RootCertStore,
            _presented_certs: &[rust_tls::Certificate],
            _dns_name: webpki::DNSNameRef<'_>,
            _ocsp: &[u8],
        ) -> Result<rust_tls::ServerCertVerified, rust_tls::TLSError> {
            Ok(rust_tls::ServerCertVerified::assertion())
        }
    }
}

// #[test]
fn _test_connection_reuse_h2() {
    block_on(async {
        let openssl = ssl_acceptor().unwrap();
        let num = Arc::new(AtomicUsize::new(0));
        let num2 = num.clone();

        let srv = TestServer::start(move || {
            let num2 = num2.clone();
            pipeline_factory(move |io| {
                num2.fetch_add(1, Ordering::Relaxed);
                ok(io)
            })
            .and_then(
                openssl
                    .clone()
                    .map_err(|e| println!("Openssl error: {}", e)),
            )
            .and_then(
                HttpService::build()
                    .h2(App::new().service(
                        web::resource("/").route(web::to(|| HttpResponse::Ok())),
                    ))
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
        let response = request.await.unwrap();
        assert!(response.status().is_success());

        // req 2
        let req = client.post(srv.surl("/"));
        let response = req.send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.version(), Version::HTTP_2);

        // one connection
        assert_eq!(num.load(Ordering::Relaxed), 1);
    })
}
