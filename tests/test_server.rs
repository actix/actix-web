extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate futures;
extern crate h2;
extern crate http;
extern crate bytes;
extern crate flate2;
extern crate brotli2;

use std::{net, thread, time};
use std::io::{Read, Write};
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicUsize, Ordering};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, DeflateEncoder, DeflateDecoder};
use brotli2::write::{BrotliEncoder, BrotliDecoder};
use futures::{Future, Stream};
use futures::stream::once;
use h2::client as h2client;
use bytes::{Bytes, BytesMut};
use http::{header, Request};
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;

use actix::System;
use actix_web::*;


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

    let mut sys = System::new("test-server");

    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str()).finish().unwrap();
        let response = sys.run_until_complete(req.send()).unwrap();
        assert!(response.status().is_success());
    }

    // pause
    let _ = srv_addr.send(server::PauseServer).wait();
    thread::sleep(time::Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_err());

    // resume
    let _ = srv_addr.send(server::ResumeServer).wait();

    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str()).finish().unwrap();
        let response = sys.run_until_complete(req.send()).unwrap();
        assert!(response.status().is_success());
    }
}

#[test]
#[cfg(unix)]
fn test_shutdown() {
    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = System::new("test");
        let srv = HttpServer::new(
            || vec![Application::new()
                    .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk))]);

        let srv = srv.bind("127.0.0.1:0").unwrap();
        let addr = srv.addrs()[0];
        let srv_addr = srv.shutdown_timeout(1).start();
        let _ = tx.send((addr, srv_addr));
        sys.run();
    });
    let (addr, srv_addr) = rx.recv().unwrap();

    let mut sys = System::new("test-server");

    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str()).finish().unwrap();
        let response = sys.run_until_complete(req.send()).unwrap();
        srv_addr.do_send(server::StopServer{graceful: true});
        assert!(response.status().is_success());
    }

    thread::sleep(time::Duration::from_millis(1000));
    assert!(net::TcpStream::connect(addr).is_err());
}

#[test]
fn test_simple() {
    let mut srv = test::TestServer::new(|app| app.handler(httpcodes::HTTPOk));
    let req = srv.get().finish().unwrap();
    let response = srv.execute(req.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_headers() {
    let data = STR.to_owned() + STR + STR + STR + STR + STR + STR + STR + STR + STR;
    let srv_data = Arc::new(data.clone());
    let mut srv = test::TestServer::new(
        move |app| {
            let data = srv_data.clone();
            app.handler(move |_| {
                let mut builder = httpcodes::HTTPOk.build();
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
                         TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST ");
                }
                builder.body(data.as_ref())})
        });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_body() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| httpcodes::HTTPOk.build().body(STR)));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_gzip() {
    let mut srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(STR)));

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_gzip_large() {
    let data = STR.to_owned() + STR + STR + STR + STR + STR + STR + STR + STR + STR;
    let srv_data = Arc::new(data.clone());

    let mut srv = test::TestServer::new(
        move |app| {
            let data = srv_data.clone();
            app.handler(
                move |_| httpcodes::HTTPOk.build()
                    .content_encoding(headers::ContentEncoding::Gzip)
                    .body(data.as_ref()))});

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from(data));
}

#[test]
fn test_body_chunked_implicit() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_br_streaming() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Br)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode br
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_head_empty() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_length(STR.len() as u64).finish()}));

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    //let bytes = srv.execute(response.body()).unwrap();
    //assert!(bytes.is_empty());
}

#[test]
fn test_head_binary() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Identity)
                .content_length(100).body(STR)}));

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    //let bytes = srv.execute(response.body()).unwrap();
    //assert!(bytes.is_empty());
}

#[test]
fn test_head_binary2() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Identity)
                .body(STR)
        }));

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }
}

#[test]
fn test_body_length() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_length(STR.len() as u64)
                .content_encoding(headers::ContentEncoding::Identity)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_explicit() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .chunked()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_deflate() {
    let mut srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk
                .build()
                .content_encoding(headers::ContentEncoding::Deflate)
                .body(STR)));

    // client request
    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode deflate
    let mut e = DeflateDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_brotli() {
    let mut srv = test::TestServer::new(
        |app| app.handler(
            |_| httpcodes::HTTPOk
                .build()
                .content_encoding(headers::ContentEncoding::Br)
                .body(STR)));

    // client request
    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode brotli
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_gzip_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv.post()
        .header(header::CONTENT_ENCODING, "gzip")
        .body(enc.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_gzip_encoding_large() {
    let data = STR.to_owned() + STR + STR + STR + STR + STR + STR + STR + STR + STR;
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv.post()
        .header(header::CONTENT_ENCODING, "gzip")
        .body(enc.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_deflate_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
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

    // client request
    let request = srv.post()
        .header(header::CONTENT_ENCODING, "deflate")
        .body(enc).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_deflate_encoding_large() {
    let data = STR.to_owned() + STR + STR + STR + STR + STR + STR + STR + STR + STR + STR;
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv.post()
        .header(header::CONTENT_ENCODING, "deflate")
        .body(enc).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_brotli_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
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

    // client request
    let request = srv.post()
        .header(header::CONTENT_ENCODING, "br")
        .body(enc).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_brotli_encoding_large() {
    let data = STR.to_owned() + STR + STR + STR + STR + STR + STR + STR + STR + STR + STR;
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Identity)
                   .body(bytes))
            }).responder()}
    ));

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv.post()
        .header(header::CONTENT_ENCODING, "br")
        .body(enc).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
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
        h2client::handshake(res.unwrap())
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
    let mut srv = test::TestServer::with_factory(
        || Application::new().resource("/", |r| r.h(httpcodes::HTTPOk)));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
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

    let mut srv = test::TestServer::new(
        move |app| app.middleware(MiddlewareTest{start: Arc::clone(&act_num1),
                                                 response: Arc::clone(&act_num2),
                                                 finish: Arc::clone(&act_num3)})
            .handler(httpcodes::HTTPOk)
    );

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

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

    let mut srv = test::TestServer::new(
        move |app| app.handler2(
            httpcodes::HTTPOk,
            MiddlewareTest{start: Arc::clone(&act_num1),
                           response: Arc::clone(&act_num2),
                           finish: Arc::clone(&act_num3)})
    );

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    // assert_eq!(num3.load(Ordering::Relaxed), 1);
}
