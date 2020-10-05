use std::sync::mpsc;
use std::{thread, time::Duration};

#[cfg(feature = "openssl")]
use open_ssl::ssl::SslAcceptorBuilder;

use actix_web::{test, web, App, HttpResponse, HttpServer};

#[cfg(unix)]
#[actix_rt::test]
async fn test_start() {
    let addr = test::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = actix_rt::System::new("test");

        let srv = HttpServer::new(|| {
            App::new().service(
                web::resource("/").route(web::to(|| HttpResponse::Ok().body("test"))),
            )
        })
        .workers(1)
        .backlog(1)
        .max_connections(10)
        .max_connection_rate(10)
        .keep_alive(10)
        .client_timeout(5000)
        .client_shutdown(0)
        .server_hostname("localhost")
        .system_exit()
        .disable_signals()
        .bind(format!("{}", addr))
        .unwrap()
        .run();

        let _ = tx.send((srv, actix_rt::System::current()));
        let _ = sys.run();
    });
    let (srv, sys) = rx.recv().unwrap();

    #[cfg(feature = "client")]
    {
        use actix_http::client;

        let client = awc::Client::builder()
            .connector(
                client::Connector::new()
                    .timeout(Duration::from_millis(100))
                    .finish(),
            )
            .finish();

        let host = format!("http://{}", addr);
        let response = client.get(host.clone()).send().await.unwrap();
        assert!(response.status().is_success());
    }

    // stop
    let _ = srv.stop(false);

    thread::sleep(Duration::from_millis(100));
    let _ = sys.stop();
}

#[cfg(feature = "openssl")]
fn ssl_acceptor() -> std::io::Result<SslAcceptorBuilder> {
    use open_ssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("tests/cert.pem")
        .unwrap();
    Ok(builder)
}

#[actix_rt::test]
#[cfg(feature = "openssl")]
async fn test_start_ssl() {
    use actix_web::HttpRequest;

    let addr = test::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = actix_rt::System::new("test");
        let builder = ssl_acceptor().unwrap();

        let srv = HttpServer::new(|| {
            App::new().service(web::resource("/").route(web::to(|req: HttpRequest| {
                assert!(req.app_config().secure());
                HttpResponse::Ok().body("test")
            })))
        })
        .workers(1)
        .shutdown_timeout(1)
        .system_exit()
        .disable_signals()
        .bind_openssl(format!("{}", addr), builder)
        .unwrap()
        .run();

        let _ = tx.send((srv, actix_rt::System::current()));
        let _ = sys.run();
    });
    let (srv, sys) = rx.recv().unwrap();

    use open_ssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let _ = builder
        .set_alpn_protos(b"\x02h2\x08http/1.1")
        .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));

    let client = awc::Client::builder()
        .connector(
            awc::Connector::new()
                .ssl(builder.build())
                .timeout(Duration::from_millis(100))
                .finish(),
        )
        .finish();

    let host = format!("https://{}", addr);
    let response = client.get(host.clone()).send().await.unwrap();
    assert!(response.status().is_success());

    // stop
    let _ = srv.stop(false);

    thread::sleep(Duration::from_millis(100));
    let _ = sys.stop();
}
