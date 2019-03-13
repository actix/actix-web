use net2::TcpBuilder;
use std::sync::mpsc;
use std::{net, thread, time::Duration};

use actix_http::{client, Response};

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

    let mut connector = test::run_on(|| {
        Ok::<_, ()>(
            client::Connector::default()
                .timeout(Duration::from_millis(100))
                .service(),
        )
    })
    .unwrap();
    let host = format!("http://{}", addr);

    let response = test::block_on(
        client::ClientRequest::get(host.clone())
            .finish()
            .unwrap()
            .send(&mut connector),
    )
    .unwrap();
    assert!(response.status().is_success());

    // stop
    let _ = srv.stop(false);

    thread::sleep(Duration::from_millis(100));
    let _ = sys.stop();
}
