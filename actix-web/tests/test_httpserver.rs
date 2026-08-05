#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;

use std::{
    convert::Infallible,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use actix_web::{
    rt::{
        net::TcpStream,
        time::{sleep, timeout},
    },
    web, App, HttpRequest, HttpResponse, HttpServer,
};
use bytes::Bytes;
use futures_util::stream;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    sync::{oneshot, Notify},
};

// Read the response head while retaining direct ownership of the connection.
async fn read_http1_response_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut response = Vec::new();

    loop {
        response.push(stream.read_u8().await?);

        if response.ends_with(b"\r\n\r\n") {
            return Ok(response);
        }
    }
}

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

    // Attempt to start a second server using the same address.
    let result = HttpServer::new(|| {
        App::new().service(
            web::resource("/").route(web::to(|| async { HttpResponse::Ok().body("test") })),
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
    .bind(format!("{}", addr));

    // This should fail: the address is in use.
    assert!(result.is_err());

    srv.stop(false).await;
}

#[actix_rt::test]
async fn test_app_data_dropped_after_graceful_shutdown_with_slow_request() {
    struct State {
        _data: Arc<String>,
    }

    async fn echo(_body: web::Json<String>) -> HttpResponse {
        HttpResponse::Ok().finish()
    }

    let (weak_data, app_data) = {
        let data = Arc::new("data".to_owned());
        (Arc::downgrade(&data), web::Data::new(State { _data: data }))
    };

    let server = HttpServer::new(move || {
        App::new()
            .app_data(app_data.clone())
            .service(web::resource("/echo").route(web::post().to(echo)))
    })
    .workers(1)
    .shutdown_timeout(1)
    .bind(("127.0.0.1", 0))
    .unwrap();

    let addr = server.addrs()[0];
    let server = server.run();
    let server_handle = server.handle();

    let send_request = async move {
        sleep(Duration::from_millis(100)).await;

        let slow_body = stream::unfold(0, |idx| async move {
            if idx < 8 {
                sleep(Duration::from_millis(200)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b" ")), idx + 1))
            } else {
                None
            }
        });

        let client = awc::Client::default();
        let _ = client
            .post(format!("http://{addr}/echo"))
            .insert_header(("content-type", "application/json"))
            .send_stream(slow_body)
            .await;
    };

    let graceful_stop = async move {
        sleep(Duration::from_millis(300)).await;
        server_handle.stop(true).await;
    };

    let (server_res, (), ()) = tokio::join!(server, send_request, graceful_stop);
    server_res.unwrap();

    for _ in 0..20 {
        sleep(Duration::from_millis(100)).await;

        if weak_data.upgrade().is_none() {
            return;
        }
    }

    panic!("app data still referenced after graceful shutdown");
}

#[actix_rt::test]
async fn graceful_shutdown_closes_idle_http1_connection_after_in_flight_request() {
    let request_started = Arc::new(Notify::new());
    let finish_request = Arc::new(Notify::new());
    let queued_request_started = Arc::new(AtomicBool::new(false));

    let server_request_started = Arc::clone(&request_started);
    let server_finish_request = Arc::clone(&finish_request);
    let server_queued_request_started = Arc::clone(&queued_request_started);

    let server = HttpServer::new(move || {
        let request_started = Arc::clone(&server_request_started);
        let finish_request = Arc::clone(&server_finish_request);
        let queued_request_started = Arc::clone(&server_queued_request_started);

        App::new()
            .route(
                "/idle",
                web::get().to(|| async { HttpResponse::Ok().finish() }),
            )
            .route(
                "/in-flight",
                web::get().to(move || {
                    let request_started = Arc::clone(&request_started);
                    let finish_request = Arc::clone(&finish_request);

                    async move {
                        request_started.notify_one();
                        finish_request.notified().await;
                        HttpResponse::Ok().finish()
                    }
                }),
            )
            .route(
                "/queued",
                web::get().to(move || {
                    queued_request_started.store(true, Ordering::SeqCst);
                    async { HttpResponse::Ok().finish() }
                }),
            )
    })
    .workers(1)
    // Keep these timeouts above the test assertions so they cannot cause EOF.
    .keep_alive(Duration::from_secs(30))
    .shutdown_timeout(5)
    .disable_signals()
    .bind(("127.0.0.1", 0))
    .unwrap();

    let addr = server.addrs()[0];
    let server = server.run();
    let server_handle = server.handle();
    let server_task = actix_web::rt::spawn(server);

    // Complete one request, leaving this connection idle and eligible for HTTP/1 keep-alive.
    let mut idle_connection = TcpStream::connect(addr).await.unwrap();
    idle_connection
        .write_all(b"GET /idle HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .await
        .unwrap();
    let response = read_http1_response_head(&mut idle_connection)
        .await
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));

    // Establish keep-alive on a second connection before giving it active and queued work.
    let mut in_flight_connection = TcpStream::connect(addr).await.unwrap();
    in_flight_connection
        .write_all(b"GET /idle HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .await
        .unwrap();
    let response = read_http1_response_head(&mut in_flight_connection)
        .await
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));

    // Pipeline one request that blocks in its handler and one request that must remain queued.
    in_flight_connection
        .write_all(
            b"GET /in-flight HTTP/1.1\r\nhost: localhost\r\n\r\n\
              GET /queued HTTP/1.1\r\nhost: localhost\r\n\r\n",
        )
        .await
        .unwrap();

    // Do not start shutdown until the first pipelined request is in flight.
    request_started.notified().await;

    let shutdown_task = actix_web::rt::spawn(async move {
        server_handle.stop(true).await;
    });

    // The idle connection must close while the active request remains blocked.
    let mut buf = [0];
    let idle_connection_closed =
        timeout(Duration::from_millis(500), idle_connection.read(&mut buf)).await;

    // Allow the current request, but not its queued successor, to finish during shutdown.
    finish_request.notify_one();

    let response = timeout(
        Duration::from_millis(500),
        read_http1_response_head(&mut in_flight_connection),
    )
    .await
    .expect("in-flight request did not finish during graceful shutdown")
    .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));

    // The active connection must close instead of returning to keep-alive after its response.
    let in_flight_connection_closed = timeout(
        Duration::from_millis(500),
        in_flight_connection.read(&mut buf),
    )
    .await;

    // Do not leave the server waiting on client sockets when an EOF assertion fails.
    drop((idle_connection, in_flight_connection));

    timeout(Duration::from_secs(2), shutdown_task)
        .await
        .expect("server did not stop after its connections closed")
        .unwrap();
    server_task.await.unwrap().unwrap();

    assert!(
        matches!(idle_connection_closed, Ok(Ok(0))),
        "idle keep-alive connection did not close immediately"
    );
    assert!(
        matches!(in_flight_connection_closed, Ok(Ok(0))),
        "keep-alive connection did not close after its in-flight request"
    );
    assert!(
        !queued_request_started.load(Ordering::SeqCst),
        "queued request started during graceful shutdown"
    );
}

#[actix_rt::test]
async fn shutdown_signal_closes_idle_http1_connection() {
    // Control the exact point at which the builder-configured shutdown signal resolves.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server = HttpServer::new(|| {
        App::new().route("/", web::get().to(|| async { HttpResponse::Ok().finish() }))
    })
    .workers(1)
    .keep_alive(Duration::from_secs(30))
    .shutdown_timeout(5)
    .shutdown_signal(async move {
        let _ = shutdown_rx.await;
    })
    .bind(("127.0.0.1", 0))
    .unwrap();

    let addr = server.addrs()[0];
    let server_task = actix_web::rt::spawn(server.run());

    // Complete a request and leave the connection idle in HTTP/1 keep-alive.
    let mut connection = TcpStream::connect(addr).await.unwrap();
    connection
        .write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .await
        .unwrap();
    let response = read_http1_response_head(&mut connection).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));

    // Resolve HttpServer::shutdown_signal and expect it to reach the HTTP/1 dispatcher.
    shutdown_tx.send(()).unwrap();

    let mut buf = [0];
    let connection_closed = timeout(Duration::from_millis(500), connection.read(&mut buf)).await;

    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server did not stop after the shutdown signal")
        .unwrap()
        .unwrap();

    assert!(
        matches!(connection_closed, Ok(Ok(0))),
        "idle keep-alive connection did not close after the shutdown signal"
    );
}

#[cfg(feature = "openssl")]
fn ssl_acceptor() -> openssl::ssl::SslAcceptorBuilder {
    use openssl::{
        pkey::PKey,
        ssl::{SslAcceptor, SslMethod},
        x509::X509,
    };

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = signing_key.serialize_pem();

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

async fn assert_tcp_nodelay_config(nodelay: bool) {
    let addr = actix_test::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        actix_rt::System::new()
            .block_on(async move {
                let srv = HttpServer::new(move || {
                    let expected = nodelay;

                    App::new().service(web::resource("/").route(web::to(
                        move |req: HttpRequest| {
                            let expected = expected;

                            async move {
                                let actual = req.conn_data::<bool>().copied().unwrap_or(!expected);
                                if actual == expected {
                                    HttpResponse::Ok().finish()
                                } else {
                                    HttpResponse::InternalServerError().finish()
                                }
                            }
                        },
                    )))
                })
                .workers(1)
                .tcp_nodelay(nodelay)
                .on_connect(move |io, ext| {
                    if let Some(io) = io.downcast_ref::<actix_web::rt::net::TcpStream>() {
                        ext.insert(io.nodelay().unwrap());
                    }
                })
                .bind(format!("{}", addr))
                .unwrap()
                .run();

                tx.send(srv.handle()).unwrap();
                srv.await
            })
            .unwrap()
    });

    let srv = rx.recv().unwrap();

    let client = awc::Client::builder()
        .connector(awc::Connector::new().timeout(Duration::from_millis(100)))
        .finish();

    let response = client.get(format!("http://{}", addr)).send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop(false).await;
}

#[actix_rt::test]
async fn test_tcp_nodelay_enabled() {
    assert_tcp_nodelay_config(true).await;
}

#[actix_rt::test]
async fn test_tcp_nodelay_disabled() {
    assert_tcp_nodelay_config(false).await;
}

#[actix_rt::test]
#[cfg(windows)]
async fn test_dual_stack_ipv6_on_windows() {
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
                .disable_signals()
                .bind("[::]:0")
                .unwrap();

                let port = srv.addrs()[0].port();
                let srv = srv.run();

                tx.send((srv.handle(), port)).unwrap();
                srv.await
            })
            .unwrap();
    });

    let (srv, port) = rx.recv().unwrap();

    let client = awc::Client::builder()
        .connector(awc::Connector::new().timeout(Duration::from_secs(1)))
        .finish();

    let response = client
        .get(format!("http://127.0.0.1:{port}"))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    srv.stop(false).await;
}
