extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;
extern crate futures;
extern crate h2;
extern crate http;
extern crate bytes;
extern crate flate2;
extern crate brotli2;

use std::{net, thread, time};
use std::io::Write;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicUsize, Ordering};
use flate2::Compression;
use flate2::write::{GzEncoder, DeflateEncoder, DeflateDecoder};
use brotli2::write::{BrotliEncoder, BrotliDecoder};
use futures::{Future, Stream};
use futures::stream::once;
use h2::client;
use bytes::{Bytes, BytesMut, BufMut};
use http::Request;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use reqwest::header::{Encoding, ContentEncoding};

use actix_web::*;
use actix::System;

const STR: &str =
    "Hello World Hello World Hello World Hello World Hello World \
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
fn test_start() {
    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = System::new("test");
        let srv = HttpServer::new(
            || vec![Application::new()
                    .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk))]);

        let srv = srv.bind("127.0.0.1:0").unwrap();
        let addr = srv.addrs()[0];
        let srv_addr = srv.start();
        let _ = tx.send((addr, srv_addr));
        sys.run();
    });
    let (addr, srv_addr) = rx.recv().unwrap();
    assert!(reqwest::get(&format!("http://{}/", addr)).unwrap().status().is_success());

    // pause
    let _ = srv_addr.call_fut(server::PauseServer).wait();
    thread::sleep(time::Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_err());

    // resume
    let _ = srv_addr.call_fut(server::ResumeServer).wait();
    assert!(reqwest::get(&format!("http://{}/", addr)).unwrap().status().is_success());
}

#[test]
fn test_simple() {
    let srv = test::TestServer::new(|app| app.handler(httpcodes::HTTPOk));
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
}

#[test]
fn test_body() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| httpcodes::HTTPOk.build().body(STR)));
    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_gzip() {
    let srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(STR)));
    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_streaming_implicit() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_br_streaming() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Br)
                .body(Body::Streaming(Box::new(body)))}));

    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();

    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_head_empty() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_length(STR.len() as u64).finish()}));

    let client = reqwest::Client::new();
    let mut res = client.head(&srv.url("/")).send().unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let len = res.headers()
        .get::<reqwest::header::ContentLength>().map(|ct_len| **ct_len).unwrap();
    assert_eq!(len, STR.len() as u64);
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Identity)
                .content_length(100).body(STR)}));

    let client = reqwest::Client::new();
    let mut res = client.head(&srv.url("/")).send().unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let len = res.headers()
        .get::<reqwest::header::ContentLength>().map(|ct_len| **ct_len).unwrap();
    assert_eq!(len, STR.len() as u64);
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary2() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Identity)
                .body(STR)
        }));

    let client = reqwest::Client::new();
    let mut res = client.head(&srv.url("/")).send().unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let len = res.headers()
        .get::<reqwest::header::ContentLength>().map(|ct_len| **ct_len).unwrap();
    assert_eq!(len, STR.len() as u64);
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert!(bytes.is_empty());
}

#[test]
fn test_body_length() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_length(STR.len() as u64)
                .body(Body::Streaming(Box::new(body)))}));

    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_streaming_explicit() {
    let srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .chunked()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_deflate() {
    let srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk
                .build()
                .content_encoding(headers::ContentEncoding::Deflate)
                .body(STR)));
    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();

    let mut e = DeflateDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_brotli() {
    let srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk
                .build()
                .content_encoding(headers::ContentEncoding::Br)
                .body(STR)));
    let mut res = reqwest::get(&srv.url("/")).unwrap();
    assert!(res.status().is_success());
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();

    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_gzip_encoding() {
    let srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let client = reqwest::Client::new();
    let mut res = client.post(&srv.url("/"))
        .header(ContentEncoding(vec![Encoding::Gzip]))
        .body(enc.clone()).send().unwrap();
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_deflate_encoding() {
    let srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let client = reqwest::Client::new();
    let mut res = client.post(&srv.url("/"))
        .header(ContentEncoding(vec![Encoding::Deflate]))
        .body(enc.clone()).send().unwrap();
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_brotli_encoding() {
    let srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let client = reqwest::Client::new();
    let mut res = client.post(&srv.url("/"))
        .header(ContentEncoding(vec![Encoding::Brotli]))
        .body(enc.clone()).send().unwrap();
    let mut bytes = BytesMut::with_capacity(2048).writer();
    let _ = res.copy_to(&mut bytes);
    let bytes = bytes.into_inner();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2() {
    let srv = test::TestServer::new(|app| app.handler(|_|{
        httpcodes::HTTPOk.build().body(STR)
    }));
    let addr = srv.addr();

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let tcp = TcpStream::connect(&addr, &handle);

    let tcp = tcp.then(|res| {
        client::handshake(res.unwrap())
    }).then(move |res| {
        let (mut client, h2) = res.unwrap();

        let request = Request::builder()
            .uri(format!("https://{}/", addr).as_str())
            .body(())
            .unwrap();
        let (response, _) = client.send_request(request, false).unwrap();

        // Spawn a task to run the conn...
        handle.spawn(h2.map_err(|e| println!("GOT ERR={:?}", e)));

        response.and_then(|response| {
            assert_eq!(response.status(), StatusCode::OK);

            let (_, body) = response.into_parts();

            body.fold(BytesMut::new(), |mut b, c| -> Result<_, h2::Error> {
                b.extend(c);
                Ok(b)
            })
        })
    });
    let _res = core.run(tcp);
    // assert_eq!(_res.unwrap(), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_application() {
    let srv = test::TestServer::with_factory(
        || Application::new().resource("/", |r| r.h(httpcodes::HTTPOk)));
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
}

struct MiddlewareTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl<S> middleware::Middleware<S> for MiddlewareTest {
    fn start(&self, _: &mut HttpRequest<S>) -> Result<middleware::Started> {
        self.start.store(self.start.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Ok(middleware::Started::Done)
    }

    fn response(&self, _: &mut HttpRequest<S>, resp: HttpResponse) -> Result<middleware::Response> {
        self.response.store(self.response.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Ok(middleware::Response::Done(resp))
    }

    fn finish(&self, _: &mut HttpRequest<S>, _: &HttpResponse) -> middleware::Finished {
        self.finish.store(self.finish.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middleware::Finished::Done
    }
}

#[test]
fn test_middlewares() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let srv = test::TestServer::new(
        move |app| app.middleware(MiddlewareTest{start: Arc::clone(&act_num1),
                                                 response: Arc::clone(&act_num2),
                                                 finish: Arc::clone(&act_num3)})
            .handler(httpcodes::HTTPOk)
    );
    
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}


#[test]
fn test_resource_middlewares() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let srv = test::TestServer::new(
        move |app| app.handler2(
            httpcodes::HTTPOk,
            MiddlewareTest{start: Arc::clone(&act_num1),
                           response: Arc::clone(&act_num2),
                           finish: Arc::clone(&act_num3)})
    );

    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    // assert_eq!(num3.load(Ordering::Relaxed), 1);
}
