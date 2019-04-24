use net2::TcpBuilder;
use std::sync::mpsc;
use std::{net, thread, time::Duration};

#[cfg(feature = "ssl")]
use openssl::ssl::SslAcceptorBuilder;

use actix_http::Response;
use actix_web::{test, web, App, HttpServer};

fn unused_addr() -> net::SocketAddr {
    let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = TcpBuilder::new_v4().unwrap();
    socket.bind(&addr).unwrap();
    socket.reuse_address(true).unwrap();
    let tcp = socket.to_tcp_listener().unwrap();
    tcp.local_addr().unwrap()
}

#[test]
#[cfg(unix)]
fn test_start() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = actix_rt::System::new("test");

        let srv = HttpServer::new(|| {
            App::new().service(
                web::resource("/").route(web::to(|| Response::Ok().body("test"))),
            )
        })
        .workers(1)
        .backlog(1)
        .maxconn(10)
        .maxconnrate(10)
        .keep_alive(10)
        .client_timeout(5000)
        .client_shutdown(0)
        .server_hostname("localhost")
        .system_exit()
        .disable_signals()
        .bind(format!("{}", addr))
        .unwrap()
        .start();

        let _ = tx.send((srv, actix_rt::System::current()));
        let _ = sys.run();
    });
    let (srv, sys) = rx.recv().unwrap();

    #[cfg(feature = "client")]
    {
        use actix_http::client;
        use actix_web::test;

        let client = test::run_on(|| {
            Ok::<_, ()>(
                awc::Client::build()
                    .connector(
                        client::Connector::new()
                            .timeout(Duration::from_millis(100))
                            .finish(),
                    )
                    .finish(),
            )
        })
        .unwrap();
        let host = format!("http://{}", addr);

        let response = test::block_on(client.get(host.clone()).send()).unwrap();
        assert!(response.status().is_success());
    }

    // stop
    let _ = srv.stop(false);

    thread::sleep(Duration::from_millis(100));
    let _ = sys.stop();
}

#[cfg(feature = "ssl")]
fn ssl_acceptor() -> std::io::Result<SslAcceptorBuilder> {
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
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

#[test]
#[cfg(feature = "ssl")]
fn test_start_ssl() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = actix_rt::System::new("test");
        let builder = ssl_acceptor().unwrap();

        let srv = HttpServer::new(|| {
            App::new().service(
                web::resource("/").route(web::to(|| Response::Ok().body("test"))),
            )
        })
        .workers(1)
        .shutdown_timeout(1)
        .system_exit()
        .disable_signals()
        .bind_ssl(format!("{}", addr), builder)
        .unwrap()
        .start();

        let _ = tx.send((srv, actix_rt::System::current()));
        let _ = sys.run();
    });
    let (srv, sys) = rx.recv().unwrap();

    let client = test::run_on(|| {
        use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::NONE);
        let _ = builder
            .set_alpn_protos(b"\x02h2\x08http/1.1")
            .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));

        Ok::<_, ()>(
            awc::Client::build()
                .connector(
                    awc::Connector::new()
                        .ssl(builder.build())
                        .timeout(Duration::from_millis(100))
                        .finish(),
                )
                .finish(),
        )
    })
    .unwrap();
    let host = format!("https://{}", addr);

    let response = test::block_on(client.get(host.clone()).send()).unwrap();
    assert!(response.status().is_success());

    // stop
    let _ = srv.stop(false);

    thread::sleep(Duration::from_millis(100));
    let _ = sys.stop();
}
