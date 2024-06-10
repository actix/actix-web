#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;

#[cfg(any(unix, feature = "openssl"))]
use {
    actix_web::{web, App, HttpResponse, HttpServer},
    std::{sync::mpsc, thread, time::Duration},
};

#[cfg(unix)]
#[actix_rt::test]
async fn test_start() {
    let addr = actix_test::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        actix_rt::System::new()
            .block_on(async {
                let srv = HttpServer::new(|| {
                    App::new().service(
                        web::resource("/")
                            .route(web::to(|| async { HttpResponse::Ok().body("test") })),
                    )
                })
                .workers(1)
                .backlog(1)
                .max_connections(10)
                .max_connection_rate(10)
                .keep_alive(Duration::from_secs(10))
                .client_request_timeout(Duration::from_secs(5))
                .client_disconnect_timeout(Duration::ZERO)
                .server_hostname("localhost")
                .system_exit()
                .disable_signals()
                .bind(format!("{}", addr))
                .unwrap()
                .run();

                tx.send(srv.handle()).unwrap();

                srv.await
            })
            .unwrap();
    });

    let srv = rx.recv().unwrap();

    let client = awc::Client::builder()
        .connector(awc::Connector::new().timeout(Duration::from_millis(100)))
        .finish();

    let host = format!("http://{}", addr);
    let response = client.get(host.clone()).send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop(false).await;
}

#[cfg(feature = "openssl")]
fn ssl_acceptor() -> openssl::ssl::SslAcceptorBuilder {
    use openssl::{
        pkey::PKey,
        ssl::{SslAcceptor, SslMethod},
        x509::X509,
    };

    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

    let cert = X509::from_pem(cert_file.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key_file.as_bytes()).unwrap();

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&key).unwrap();

    builder
}

#[actix_rt::test]
#[cfg(feature = "openssl")]
async fn test_start_ssl() {
    use actix_web::HttpRequest;
    use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

    let addr = actix_test::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        actix_rt::System::new()
            .block_on(async {
                let builder = ssl_acceptor();

                let srv = HttpServer::new(|| {
                    App::new().service(web::resource("/").route(web::to(|req: HttpRequest| {
                        assert!(req.app_config().secure());
                        async { HttpResponse::Ok().body("test") }
                    })))
                })
                .workers(1)
                .shutdown_timeout(1)
                .system_exit()
                .disable_signals()
                .bind_openssl(format!("{}", addr), builder)
                .unwrap();

                let srv = srv.run();
                tx.send(srv.handle()).unwrap();

                srv.await
            })
            .unwrap()
    });
    let srv = rx.recv().unwrap();

    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let _ = builder
        .set_alpn_protos(b"\x02h2\x08http/1.1")
        .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));

    let client = awc::Client::builder()
        .connector(
            awc::Connector::new()
                .openssl(builder.build())
                .timeout(Duration::from_millis(100)),
        )
        .finish();

    let host = format!("https://{}", addr);
    let response = client.get(host.clone()).send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop(false).await;
}
