#![cfg(feature = "openssl")]
use actix_http::HttpService;
use actix_http_test::test_server;
use actix_service::{map_config, ServiceFactory};
use actix_web::http::Version;
use actix_web::{dev::AppConfig, web, App, HttpResponse};
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
async fn test_connection_window_size() {
    let srv = test_server(move || {
        HttpService::build()
            .h2(map_config(
                App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                |_| AppConfig::default(),
            ))
            .openssl(ssl_acceptor())
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
        .connector(awc::Connector::new().ssl(builder.build()).finish())
        .initial_window_size(100)
        .initial_connection_window_size(100)
        .finish();

    let request = client.get(srv.surl("/")).send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);
}
