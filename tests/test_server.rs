extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate bytes;
extern crate futures;

use std::io::{Read, Write};
use std::time::Duration;
use std::{net, thread};

use actix_net::service::NewServiceExt;
use bytes::Bytes;
use futures::future::{self, ok};
use futures::stream::once;

use actix_http::{
    body, client, h1, http, test, Body, Error, HttpMessage as HttpMessage2, KeepAlive,
    Request, Response,
};

#[test]
fn test_h1_v2() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .server_hostname("localhost")
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let req = client::ClientRequest::get(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_slow_request() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .client_timeout(100)
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[test]
fn test_malformed_request() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| future::ok::<_, ()>(Response::Ok().finish())).map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));
}

#[test]
fn test_keepalive() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
}

#[test]
fn test_keepalive_timeout() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .keep_alive(1)
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
    thread::sleep(Duration::from_millis(1100));

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_keepalive_close() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ =
        stream.write_all(b"GET /test/tests/test HTTP/1.1\r\nconnection: close\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_keepalive_http10_default_close() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_keepalive_http10() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream
        .write_all(b"GET /test/tests/test HTTP/1.0\r\nconnection: keep-alive\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_keepalive_disabled() {
    let srv = test::TestServer::with_factory(|| {
        h1::H1Service::build()
            .keep_alive(KeepAlive::Disabled)
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
            .map(|_| ())
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let mut srv = test::TestServer::with_factory(|| {
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
    });

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = client::ClientRequest::get(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = client::ClientRequest::head(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = client::ClientRequest::get(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test::TestServer::with_factory(move || {
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
    });

    let mut connector = srv.new_connector();

    let req = srv.get().finish().unwrap();

    let response = srv.block_on(req.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
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
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| future::ok::<_, ()>(Response::Ok().body(STR))).map(|_| ())
    });

    let req = srv.get().finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_head_empty() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| ok::<_, ()>(Response::Ok().body(STR))).map(|_| ())
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| {
            ok::<_, ()>(Response::Ok().content_length(STR.len() as u64).body(STR))
        }).map(|_| ())
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary2() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| ok::<_, ()>(Response::Ok().body(STR))).map(|_| ())
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
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
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(
                Response::Ok()
                    .body(Body::from_message(body::SizedStream::new(STR.len(), body))),
            )
        }).map(|_| ())
    });

    let req = srv.get().finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_explicit() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(Response::Ok().streaming(body))
        }).map(|_| ())
    });

    let req = srv.get().finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_implicit() {
    let mut srv = test::TestServer::with_factory(|| {
        h1::H1Service::new(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(Response::Ok().streaming(body))
        }).map(|_| ())
    });

    let req = srv.get().finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}
