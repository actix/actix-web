extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;

use std::{net, thread};
use std::str::FromStr;
use actix::*;
use actix_web::*;
use tokio_core::net::TcpListener;


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
    thread::spawn(|| {
        let sys = System::new("test");

        let srv = create_server();
        srv.serve::<_, ()>("127.0.0.1:58902").unwrap();
        sys.run();
    });
    assert!(reqwest::get("http://localhost:58906/").unwrap().status().is_success());
}

#[test]
fn test_serve_incoming() {
    thread::spawn(|| {
        let sys = System::new("test");

        let srv = create_server();
        let addr = net::SocketAddr::from_str("127.0.0.1:58906").unwrap();
        let tcp = TcpListener::bind(&addr, Arbiter::handle()).unwrap();
        srv.serve_incoming::<_, ()>(tcp.incoming()).unwrap();
        sys.run();

    });

    assert!(reqwest::get("http://localhost:58906/").unwrap().status().is_success());
}
