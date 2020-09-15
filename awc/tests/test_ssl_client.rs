#![cfg(feature = "openssl")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use actix_http::HttpService;
use actix_http_test::test_server;
use actix_service::{map_config, pipeline_factory, ServiceFactory};
use actix_web::http::Version;
use actix_web::{dev::AppConfig, web, App, HttpResponse};
use futures_util::future::ok;
use open_ssl::ssl::{SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode};

fn ssl_acceptor() -> SslAcceptor {
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
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
    builder.set_alpn_protos(b"\x02h2").unwrap();
    builder.build()
}

#[actix_rt::test]
async fn test_connection_reuse_h2() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        pipeline_factory(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::build()
                .h2(map_config(
                    App::new()
                        .service(web::resource("/").route(web::to(HttpResponse::Ok))),
                    |_| AppConfig::default(),
                ))
                .openssl(ssl_acceptor())
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
        .connector(awc::Connector::new().ssl(builder.build()).finish())
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
