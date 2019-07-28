#![cfg(feature = "ssl")]
use openssl::ssl::{SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode};

use std::io::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_server::ssl::OpensslAcceptor;
use actix_service::{service_fn, NewService};
use actix_web::http::Version;
use actix_web::{web, App, HttpResponse};

fn ssl_acceptor<T: AsyncRead + AsyncWrite>() -> Result<OpensslAcceptor<T, ()>> {
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
            Err(openssl::ssl::AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x02h2")?;
    Ok(actix_server::ssl::OpensslAcceptor::new(builder.build()))
}

#[test]
fn test_connection_reuse_h2() {
    let openssl = ssl_acceptor().unwrap();
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(
            openssl
                .clone()
                .map_err(|e| println!("Openssl error: {}", e)),
        )
        .and_then(
            HttpService::build()
                .h2(App::new()
                    .service(web::resource("/").route(web::to(|| HttpResponse::Ok()))))
                .map_err(|_| ()),
        )
    });

    // disable ssl verification
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let _ = builder
        .set_alpn_protos(b"\x02h2\x08http/1.1")
        .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));

    let client = awc::Client::build()
        .connector(awc::Connector::new().ssl(builder.build()).finish())
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
