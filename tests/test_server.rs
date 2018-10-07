extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate actix_web;
extern crate futures;

use std::{io::Read, io::Write, net, thread, time};

use actix::System;
use actix_net::server::Server;
use actix_web::{client, test};
use futures::future;

use actix_http::{h1, Error, KeepAlive, Response, ServiceConfig};

#[test]
fn test_h1_v2() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let settings = ServiceConfig::build()
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_disconnect(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish();

                h1::H1Service::new(settings, |_| {
                    future::ok::<_, Error>(Response::Ok().finish())
                })
            }).unwrap()
            .run();
    });

    let mut sys = System::new("test");
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = sys.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }
}

#[test]
fn test_slow_request() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let settings = ServiceConfig::build().client_timeout(100).finish();

                h1::H1Service::new(settings, |_| {
                    future::ok::<_, Error>(Response::Ok().finish())
                })
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[test]
fn test_malformed_request() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let settings = ServiceConfig::build().client_timeout(100).finish();
                h1::H1Service::new(settings, |_| {
                    future::ok::<_, Error>(Response::Ok().finish())
                })
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));
}
