extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate tokio_core;

use std::net;
use std::str::FromStr;
use std::io::prelude::*;
use actix::*;
use actix_web::*;
use futures::Future;
use tokio_core::net::{TcpStream, TcpListener};


fn create_server<T, A>() -> HttpServer<T, A, Application<()>> {
    HttpServer::new(
        vec![Application::default("/")
             .resource("/", |r|
                       r.handler(Method::GET, |_, _, _| {
                           httpcodes::HTTPOk
                       }))
             .finish()])
}

#[test]
fn test_serve() {
    let sys = System::new("test");

    let srv = create_server();
    srv.serve::<_, ()>("127.0.0.1:58902").unwrap();
    let addr = net::SocketAddr::from_str("127.0.0.1:58902").unwrap();

    Arbiter::handle().spawn(
        TcpStream::connect(&addr, Arbiter::handle()).and_then(|mut stream| {
            let _ = stream.write("GET /\r\n\r\n ".as_ref());
            Arbiter::system().send(msgs::SystemExit(0));
            futures::future::ok(())
        }).map_err(|_| panic!("should not happen"))
    );

    sys.run();
}

#[test]
fn test_serve_incoming() {
    let sys = System::new("test");

    let srv = create_server();
    let addr = net::SocketAddr::from_str("127.0.0.1:58906").unwrap();
    let tcp = TcpListener::bind(&addr, Arbiter::handle()).unwrap();
    srv.serve_incoming::<_, ()>(tcp.incoming()).unwrap();
    let addr = net::SocketAddr::from_str("127.0.0.1:58906").unwrap();

    // connect
    Arbiter::handle().spawn(
        TcpStream::connect(&addr, Arbiter::handle()).and_then(|mut stream| {
            let _ = stream.write("GET /\r\n\r\n ".as_ref());
            Arbiter::system().send(msgs::SystemExit(0));
            futures::future::ok(())
        }).map_err(|_| panic!("should not happen"))
    );

    sys.run();
}
