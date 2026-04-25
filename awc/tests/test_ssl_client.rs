#![cfg(feature = "openssl")]

extern crate tls_openssl as openssl;

use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

use actix_http::{HttpService, Request, Response};
use actix_http_test::test_server;
use actix_service::{fn_service, map_config, ServiceFactoryExt};
use actix_utils::future::ok;
use actix_web::{
    dev::AppConfig,
    http::{header, Version},
    web, App, HttpResponse,
};
use futures_util::stream;
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
                .openssl(tls_config())
                .map_err(|_| ()),
        )
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

// Regression test for https://github.com/actix/actix-web/issues/2305.
#[actix_rt::test]
async fn h2_streaming_body_does_not_send_transfer_encoding() {
    let has_transfer_encoding = Arc::new(AtomicBool::new(false));
    let has_transfer_encoding2 = Arc::clone(&has_transfer_encoding);

    let srv = test_server(move || {
        let has_transfer_encoding = Arc::clone(&has_transfer_encoding2);

        HttpService::build()
            .h2(move |req: Request| {
                let has_transfer_encoding = Arc::clone(&has_transfer_encoding);

                async move {
                    has_transfer_encoding.store(
                        req.head().headers.contains_key(header::TRANSFER_ENCODING),
                        Ordering::Relaxed,
                    );

                    Ok::<_, Infallible>(Response::ok())
                }
            })
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
        .finish();

    let response = client
        .post(srv.surl("/"))
        .version(Version::HTTP_2)
        .send_stream(stream::once(async {
            Ok::<_, Infallible>(bytes::Bytes::from_static(b"hello"))
        }))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);
    assert!(!has_transfer_encoding.load(Ordering::Relaxed));
}
