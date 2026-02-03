#![cfg(feature = "openssl")]

extern crate tls_openssl as openssl;

use actix_http::HttpService;
use actix_http_test::test_server;
use actix_service::{map_config, ServiceFactoryExt};
use actix_web::{dev::AppConfig, http::Version, web, App, HttpResponse};
use openssl::{
    pkey::PKey,
    ssl::{SslAcceptor, SslConnector, SslMethod, SslVerifyMode},
    x509::X509,
};

fn tls_config() -> SslAcceptor {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

    let cert = X509::from_pem(cert_file.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key_file.as_bytes()).unwrap();

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&key).unwrap();

    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(openssl::ssl::AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x02h2").unwrap();

    builder.build()
}

#[actix_rt::test]
async fn test_connection_window_size() {
    let srv = test_server(|| {
        HttpService::build()
            .h2(map_config(
                App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                |_| AppConfig::default(),
            ))
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    // disable ssl verification
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let _ = builder
        .set_alpn_protos(b"\x02h2\x08http/1.1")
        .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));

    let client = awc::Client::builder()
        .connector(awc::Connector::new().openssl(builder.build()))
        .initial_window_size(100)
        .initial_connection_window_size(100)
        .finish();

    let request = client.get(srv.surl("/")).send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);
}
