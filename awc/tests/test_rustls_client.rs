#![cfg(feature = "rustls")]

extern crate tls_rustls as rustls;

use std::{
    io::BufReader,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use actix_http::HttpService;
use actix_http_test::test_server;
use actix_service::{fn_service, map_config, ServiceFactoryExt};
use actix_utils::future::ok;
use actix_web::{dev::AppConfig, http::Version, web, App, HttpResponse};
use rustls::internal::pemfile::{certs, pkcs8_private_keys};
use rustls::{ClientConfig, NoClientAuth, ServerConfig};

fn tls_config() -> ServerConfig {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_file = cert.serialize_pem().unwrap();
    let key_file = cert.serialize_private_key_pem();

    let mut config = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(cert_file.as_bytes());
    let key_file = &mut BufReader::new(key_file.as_bytes());

    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    config
}

mod danger {
    pub struct NoCertificateVerification;

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

#[actix_rt::test]
async fn test_connection_reuse_h2() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::build()
                .h2(map_config(
                    App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                    |_| AppConfig::default(),
                ))
                .rustls(tls_config())
                .map_err(|_| ()),
        )
    })
    .await;

    // disable TLS verification
    let mut config = ClientConfig::new();
    let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.set_protocols(&protos);
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification));

    let client = awc::Client::builder()
        .connector(awc::Connector::new().rustls(Arc::new(config)))
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
}
