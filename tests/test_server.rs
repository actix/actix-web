extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate actix_web;
extern crate bytes;
extern crate futures;

use std::{io::Read, io::Write, net, thread, time};

use actix::System;
use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use actix_web::{client, test, HttpMessage};
use bytes::Bytes;
use futures::future::{self, ok};
use futures::stream::once;

use actix_http::{body, h1, http, Body, Error, KeepAlive, Request, Response};

#[test]
fn test_h1_v2() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::build()
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_disconnect(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
                    .map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr))
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
                h1::H1Service::build()
                    .client_timeout(100)
                    .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
                    .map(|_| ())
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
                h1::H1Service::new(|_| future::ok::<_, ()>(Response::Ok().finish()))
                    .map(|_| ())
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
                h1::H1Service::new(|req: Request| {
                    let indx: usize = req.uri().path()[1..].parse().unwrap();
                    let statuses = [
                        StatusCode::NO_CONTENT,
                        StatusCode::CONTINUE,
                        StatusCode::SWITCHING_PROTOCOLS,
                        StatusCode::PROCESSING,
                        StatusCode::OK,
                        StatusCode::NOT_FOUND,
                    ];
                    future::ok::<_, ()>(Response::new(statuses[indx]))
                }).map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    let mut sys = System::new("test");
    {
        for i in 0..4 {
            let req = client::ClientRequest::get(format!("http://{}/{}", addr, i))
                .finish()
                .unwrap();
            let response = sys.block_on(req.send()).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = client::ClientRequest::head(format!("http://{}/{}", addr, i))
                .finish()
                .unwrap();
            let response = sys.block_on(req.send()).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = client::ClientRequest::get(format!("http://{}/{}", addr, i))
                .finish()
                .unwrap();
            let response = sys.block_on(req.send()).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let data = data.clone();
                h1::H1Service::new(move |_| {
                    let mut builder = Response::Ok();
                    for idx in 0..90 {
                        builder.header(
                            format!("X-TEST-{}", idx).as_str(),
                            "TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                             TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST ",
                        );
                    }
                    future::ok::<_, ()>(builder.body(data.clone()))
                }).map(|_| ())
            })
            .unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(400));

    let mut sys = System::new("test");
    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();

    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data2));
}

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
fn test_body() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
                    .map(|_| ())
            }).unwrap()
            .run();
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_head_empty() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| {
                    ok::<_, ()>(Response::Ok().content_length(STR.len() as u64).finish())
                }).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::head(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| {
                    ok::<_, ()>(
                        Response::Ok().content_length(STR.len() as u64).body(STR),
                    )
                }).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::head(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary2() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| ok::<_, ()>(Response::Ok().body(STR))).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::head(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }
}

#[test]
fn test_body_length() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| {
                    let body = once(Ok(Bytes::from_static(STR.as_ref())));
                    ok::<_, ()>(Response::Ok().body(Body::from_message(
                        body::SizedStream::new(STR.len(), body),
                    )))
                }).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_explicit() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| {
                    let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
                    ok::<_, ()>(Response::Ok().streaming(body))
                }).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_implicit() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                h1::H1Service::new(|_| {
                    let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
                    ok::<_, ()>(Response::Ok().streaming(body))
                }).map(|_| ())
            }).unwrap()
            .run()
    });
    thread::sleep(time::Duration::from_millis(100));

    let mut sys = System::new("test");
    let req = client::ClientRequest::get(format!("http://{}/", addr))
        .finish()
        .unwrap();
    let response = sys.block_on(req.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = sys.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}
