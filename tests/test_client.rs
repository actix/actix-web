extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate bytes;
extern crate futures;

use std::{thread, time};

use actix::System;
use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use bytes::Bytes;
use futures::future::{self, lazy, ok};

use actix_http::HttpMessage;
use actix_http::{client, h1, test, Request, Response};

const STR: &str = "Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World";

#[test]
fn test_h1_v2() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::build()
                    .finish(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
                    .map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let mut connector = sys
        .block_on(lazy(|| Ok::<_, ()>(client::Connector::default().service())))
        .unwrap();

    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();

    let response = sys.block_on(req.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    let request = client::ClientRequest::get(format!("http://{}/", addr))
        .header("x-test", "111")
        .finish()
        .unwrap();
    let repr = format!("{:?}", request);
    assert!(repr.contains("ClientRequest"));
    assert!(repr.contains("x-test"));

    let response = sys.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let request = client::ClientRequest::post(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_connection_close() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::build()
                    .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
                    .map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let mut connector = sys
        .block_on(lazy(|| Ok::<_, ()>(client::Connector::default().service())))
        .unwrap();

    let request = client::ClientRequest::get(format!("http://{}/", addr))
        .header("Connection", "close")
        .finish()
        .unwrap();
    let response = sys.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_with_query_parameter() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::build()
                    .finish(|req: Request| {
                        if req.uri().query().unwrap().contains("qp=") {
                            ok::<_, ()>(Response::Ok().finish())
                        } else {
                            ok::<_, ()>(Response::BadRequest().finish())
                        }
                    }).map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let mut connector = sys
        .block_on(lazy(|| Ok::<_, ()>(client::Connector::default().service())))
        .unwrap();

    let request = client::ClientRequest::get(format!("http://{}/?qp=5", addr))
        .finish()
        .unwrap();

    let response = sys.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}
