use std::io;

use actix_http::{error::Error, HttpService, Response};
use actix_server::Server;

#[actix_rt::test]
async fn h2_ping_pong() -> io::Result<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    let lst = std::net::TcpListener::bind("127.0.0.1:0")?;

    let addr = lst.local_addr().unwrap();

    let join = std::thread::spawn(move || {
        actix_rt::System::new().block_on(async move {
            let srv = Server::build()
                .disable_signals()
                .workers(1)
                .listen("h2_ping_pong", lst, || {
                    HttpService::build()
                        .keep_alive(3)
                        .h2(|_| async { Ok::<_, Error>(Response::ok()) })
                        .tcp()
                })?
                .run();

            tx.send(srv.handle()).unwrap();

            srv.await
        })
    });

    let handle = rx.recv().unwrap();

    let (sync_tx, rx) = std::sync::mpsc::sync_channel(1);

    // use a separate thread for h2 client so it can be blocked.
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let stream = tokio::net::TcpStream::connect(addr).await.unwrap();

                let (mut tx, conn) = h2::client::handshake(stream).await.unwrap();

                tokio::spawn(async move { conn.await.unwrap() });

                let (res, _) = tx.send_request(::http::Request::new(()), true).unwrap();
                let res = res.await.unwrap();

                assert_eq!(res.status().as_u16(), 200);

                sync_tx.send(()).unwrap();

                // intentionally block the client thread so it can not answer ping pong.
                std::thread::sleep(std::time::Duration::from_secs(1000));
            })
    });

    rx.recv().unwrap();

    let now = std::time::Instant::now();

    // stop server gracefully. this step would take up to 30 seconds.
    handle.stop(true).await;

    // join server thread. only when connection are all gone this step would finish.
    join.join().unwrap()?;

    // check the time used for join server thread so it's known that the server shutdown
    // is from keep alive and not server graceful shutdown timeout.
    assert!(now.elapsed() < std::time::Duration::from_secs(30));

    Ok(())
}
