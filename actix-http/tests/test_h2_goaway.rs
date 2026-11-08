#![cfg(feature = "http2")]

use std::{
    io, net,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use actix_web::{dev::ServerHandle, web, App, HttpResponse, HttpServer};

const SLOW: Duration = Duration::from_millis(500);

type Started = (
    net::SocketAddr,
    ServerHandle,
    thread::JoinHandle<io::Result<()>>,
);

fn start(started: Arc<tokio::sync::Notify>) -> io::Result<Started> {
    let lst = net::TcpListener::bind("127.0.0.1:0")?;
    let addr = lst.local_addr()?;
    let (tx, rx) = mpsc::sync_channel(1);
    let join = thread::spawn(move || {
        actix_rt::System::new().block_on(async move {
            let server = HttpServer::new(move || {
                App::new().app_data(web::Data::from(started.clone())).route(
                    "/slow",
                    web::get().to(|started: web::Data<tokio::sync::Notify>| async move {
                        started.notify_one();
                        actix_rt::time::sleep(SLOW).await;
                        HttpResponse::Ok().finish()
                    }),
                )
            })
            .workers(1)
            .disable_signals()
            .shutdown_timeout(30);
            let server = server.listen_auto_h2c(lst)?.run();
            tx.send(server.handle()).unwrap();
            server.await
        })
    });
    Ok((addr, rx.recv().unwrap(), join))
}

fn get(path: &str) -> http::Request<()> {
    http::Request::get(format!("http://localhost{path}"))
        .body(())
        .unwrap()
}

fn client_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn h2_graceful_shutdown_sends_goaway() -> io::Result<()> {
    let started = Arc::new(tokio::sync::Notify::new());
    let (addr, handle, join) = start(started.clone())?;

    let stopped_after = client_rt().block_on(async move {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (client, conn) = h2::client::handshake(stream).await.unwrap();
        let conn = tokio::spawn(conn);

        let mut client = client.ready().await.unwrap();
        let (slow, _) = client.send_request(get("/slow"), true).unwrap();
        started.notified().await;

        let stopping = Instant::now();
        let stop = tokio::spawn(async move { handle.stop(true).await });

        assert_eq!(
            slow.await.unwrap().status(),
            200,
            "in-flight stream failed on shutdown"
        );
        conn.await.unwrap().unwrap();
        stop.await.unwrap();
        assert!(
            client.ready().await.is_err(),
            "closed connection accepted a new stream"
        );
        stopping.elapsed()
    });

    join.join().unwrap()?;
    // Without GOAWAY the client keeps the connection open and actix waits the full 30s.
    assert!(
        stopped_after < Duration::from_secs(5),
        "shutdown waited for the timeout: {stopped_after:?}"
    );
    Ok(())
}
