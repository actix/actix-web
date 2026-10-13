use std::{error::Error as _, net::TcpListener, time::Duration};

use actix_web::{
    error::PayloadError,
    rt::{net::TcpStream, time::timeout},
    web, App, HttpResponse, HttpServer,
};
use bytes::Bytes;
use futures_util::StreamExt as _;

#[test]
fn lockfile_excludes_legacy_h2() {
    let lockfile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    let versions: Vec<_> = lockfile
        .split("[[package]]")
        .filter(|package| package.lines().any(|line| line == "name = \"h2\""))
        .map(|package| {
            package
                .lines()
                .find(|line| line.starts_with("version = "))
                .unwrap()
        })
        .collect();

    assert_eq!(versions.len(), 1, "expected one h2 version: {versions:?}");
    assert!(versions[0].starts_with("version = \"0.4."));
}

#[actix_rt::test]
async fn native_http2_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = HttpServer::new(|| {
        App::new().route(
            "/echo",
            web::post().to(|body: Bytes| async move { HttpResponse::Created().body(body) }),
        )
    })
    .workers(1)
    .disable_signals()
    .listen_auto_h2c(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let server_task = actix_web::rt::spawn(server);

    let (mut client, connection) = h2::client::handshake(TcpStream::connect(addr).await.unwrap())
        .await
        .unwrap();
    let connection = actix_web::rt::spawn(connection);
    let request = http::Request::builder()
        .method("POST")
        .uri(format!("http://{addr}/echo"))
        .body(())
        .unwrap();
    let (response, mut stream) = client.send_request(request, false).unwrap();
    stream
        .send_data(Bytes::from_static(b"native"), true)
        .unwrap();
    let response = timeout(Duration::from_secs(5), response)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::CREATED);
    assert_eq!(
        response.into_body().data().await.unwrap().unwrap(),
        "native"
    );

    connection.abort();
    handle.stop(false).await;
    server_task.await.unwrap().unwrap();
}

#[actix_rt::test]
async fn native_reset_uses_opaque_actix_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let (error_tx, mut error_rx) = tokio::sync::mpsc::unbounded_channel();
    let server = HttpServer::new(move || {
        let started_tx = started_tx.clone();
        let error_tx = error_tx.clone();

        App::new().route(
            "/reset",
            web::post().to(move |mut payload: web::Payload| {
                let started_tx = started_tx.clone();
                let error_tx = error_tx.clone();

                async move {
                    started_tx.send(()).unwrap();
                    error_tx
                        .send(payload.next().await.unwrap().unwrap_err())
                        .unwrap();
                    HttpResponse::Ok().finish()
                }
            }),
        )
    })
    .workers(1)
    .disable_signals()
    .listen_auto_h2c(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let server_task = actix_web::rt::spawn(server);

    let (mut client, connection) = h2::client::handshake(TcpStream::connect(addr).await.unwrap())
        .await
        .unwrap();
    let connection = actix_web::rt::spawn(connection);
    let request = http::Request::builder()
        .method("POST")
        .uri(format!("http://{addr}/reset"))
        .body(())
        .unwrap();
    let (_response, mut stream) = client.send_request(request, false).unwrap();
    timeout(Duration::from_secs(5), started_rx.recv())
        .await
        .unwrap()
        .unwrap();
    stream.send_reset(h2::Reason::CANCEL);
    let err = timeout(Duration::from_secs(5), error_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let PayloadError::Http2(err) = err else {
        panic!("expected the native HTTP/2 error variant");
    };

    let err: actix_web::error::Http2Error = err;
    assert_eq!(err.reason_code(), Some(8));
    assert!(err.is_reset());
    assert!(err.is_remote());
    assert!(!err.is_library());
    assert!(!err.is_go_away());
    assert!(err.as_io_error().is_none());
    assert!(err.source().is_none());
    assert!(!(&err as &dyn std::error::Error).is::<h2::Error>());

    connection.abort();
    handle.stop(false).await;
    server_task.await.unwrap().unwrap();
}
