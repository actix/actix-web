#![cfg(feature = "rustls-0_23-webpki-roots")]

extern crate tls_rustls_0_23 as rustls;

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
use actix_tls::connect::rustls_0_23::webpki_roots_cert_store;
use actix_utils::future::ok;
use actix_web::{dev::AppConfig, http::Version, web, App, HttpResponse};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, ServerName},
    ClientConfig, ServerConfig,
};
use rustls_pemfile::{certs, pkcs8_private_keys};

fn tls_config() -> ServerConfig {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

    let cert_file = &mut BufReader::new(cert_file.as_bytes());
    let key_file = &mut BufReader::new(key_file.as_bytes());

    let cert_chain = certs(cert_file).collect::<Result<Vec<_>, _>>().unwrap();
    let mut keys = pkcs8_private_keys(key_file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, PrivateKeyDer::Pkcs8(keys.remove(0)))
        .unwrap()
}

mod danger {
    use rustls::{
        client::danger::{ServerCertVerified, ServerCertVerifier},
        pki_types::UnixTime,
    };

    use super::*;

    #[derive(Debug)]
    pub struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
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
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification));

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
