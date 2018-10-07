extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate actix_web;
extern crate futures;

use std::{io::Read, io::Write, net, thread, time};

use actix::System;
use actix_net::server::Server;
use actix_web::{client, test, HttpMessage};
use futures::future;

use actix_http::{h1, Error, KeepAlive, Request, Response, ServiceConfig};

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

#[test]
fn test_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let settings = ServiceConfig::build().client_timeout(100).finish();
                h1::H1Service::new(settings, |req: Request| {
                    let indx: usize = req.uri().path()[1..].parse().unwrap();
                    let statuses = [
                        StatusCode::NO_CONTENT,
                        StatusCode::CONTINUE,
                        StatusCode::SWITCHING_PROTOCOLS,
                        StatusCode::PROCESSING,
                        StatusCode::OK,
                        StatusCode::NOT_FOUND,
                    ];
                    future::ok::<_, Error>(Response::new(statuses[indx]))
                })
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    let mut sys = System::new("test");
    {
        for i in 0..4 {
            let req =
                client::ClientRequest::get(format!("http://{}/{}", addr, i).as_str())
                    .finish()
                    .unwrap();
            let response = sys.block_on(req.send()).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req =
                client::ClientRequest::get(format!("http://{}/{}", addr, i).as_str())
                    .finish()
                    .unwrap();
            let response = sys.block_on(req.send()).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}
