#![cfg(feature = "rustls-0_23-webpki-roots")]

extern crate tls_rustls_0_23 as rustls;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use actix_http::HttpService;
use actix_http_test::test_server;
use actix_service::{fn_service, map_config, ServiceFactoryExt};
use actix_tls::connect::rustls_0_23::webpki_roots_cert_store;
use actix_utils::future::ok;
use actix_web::{dev::AppConfig, http::Version, web, App, HttpResponse};
use rustls::{pki_types::ServerName, ClientConfig, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

fn tls_config() -> ServerConfig {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_chain = vec![cert.der().clone()];
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)
        .unwrap()
}

mod danger {
    use std::collections::HashSet;

    use rustls::{
        client::danger::{ServerCertVerified, ServerCertVerifier},
        pki_types::UnixTime,
    };

    use super::*;

    #[derive(Debug, Default)]
    pub struct NoCertificateVerification {
        pub trusted_hosts: HashSet<String>,
    }

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            if self.trusted_hosts.is_empty() {
                return Ok(ServerCertVerified::assertion());
            }

            if self.trusted_hosts.contains(_server_name.to_str().as_ref()) {
                return Ok(ServerCertVerified::assertion());
            }

            Err(rustls::Error::General("untrusted host".into()))
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::aws_lc_rs::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
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
                .rustls_0_23(tls_config())
                .map_err(|_| ()),
        )
    })
    .await;

    let mut config = ClientConfig::builder()
        .with_root_certificates(webpki_roots_cert_store())
        .with_no_client_auth();

    let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.alpn_protocols = protos;

    // disable TLS verification
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification::default()));

    let client = awc::Client::builder()
        .connector(awc::Connector::new().rustls_0_23(Arc::new(config)))
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

#[actix_rt::test]
async fn test_connection_with_sni() {
    let srv = test_server(move || {
        HttpService::build()
            .h2(map_config(
                App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                |_| AppConfig::default(),
            ))
            .rustls_0_23(tls_config())
            .map_err(|_| ())
    })
    .await;

    let mut config = ClientConfig::builder()
        .with_root_certificates(webpki_roots_cert_store())
        .with_no_client_auth();

    let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.alpn_protocols = protos;

    // disable TLS verification
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification {
            trusted_hosts: ["localhost".to_owned()].iter().cloned().collect(),
        }));

    let client = awc::Client::builder()
        .connector(awc::Connector::new().rustls_0_23(Arc::new(config)))
        .finish();

    // req : standard request
    let request = client.get(srv.surl("/")).send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req : test specific host with address, return trusted host
    let request = client.get(srv.surl("/")).sni_host("localhost").send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req : test bad host, return untrusted host
    let request = client.get(srv.surl("/")).sni_host("bad.host").send();
    let response = request.await;

    assert!(response.is_err());
    assert_eq!(
        response.unwrap_err().to_string(),
        "Failed to connect to host: unexpected error: untrusted host"
    );

    // req : test specific host with address, return untrusted host
    let addr = srv.addr();
    let request = client.get("https://example.com:443/").address(addr).send();
    let response = request.await;

    assert!(response.is_err());
    assert_eq!(
        response.unwrap_err().to_string(),
        "Failed to connect to host: unexpected error: untrusted host"
    );

    // req : test specify sni_host with address and other host (authority)
    let request = client
        .get("https://example.com:443/")
        .address(addr)
        .sni_host("localhost")
        .send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req : test ip address with sni host
    let request = client
        .get("https://127.0.0.1:443/")
        .address(addr)
        .sni_host("localhost")
        .send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());
}
