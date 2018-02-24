extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate flate2;

use std::io::Read;

use bytes::Bytes;
use futures::Future;
use futures::stream::once;
use flate2::read::GzDecoder;

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
fn test_simple() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| httpcodes::HTTPOk.build().body(STR)));

    let request = srv.get().header("x-test", "111").finish().unwrap();
    let repr = format!("{:?}", request);
    assert!(repr.contains("ClientRequest"));
    assert!(repr.contains("x-test"));

    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let request = srv.post().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_no_decompress() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| httpcodes::HTTPOk.build().body(STR)));

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));

    // POST
    let request = srv.post().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();

    let bytes = srv.execute(response.body()).unwrap();
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_gzip_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Deflate)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(headers::ContentEncoding::Gzip)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_brotli_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Deflate)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.client(Method::POST, "/")
        .content_encoding(headers::ContentEncoding::Br)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_deflate_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(headers::ContentEncoding::Br)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(headers::ContentEncoding::Deflate)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_streaming_explicit() {
    let mut srv = test::TestServer::new(
        |app| app.handler(
            |req: HttpRequest| req.body()
                .map_err(Error::from)
                .and_then(|body| {
                    Ok(httpcodes::HTTPOk.build()
                       .chunked()
                       .content_encoding(headers::ContentEncoding::Identity)
                       .body(body)?)})
                .responder()));

    let body = once(Ok(Bytes::from_static(STR.as_ref())));

    let request = srv.get().body(Body::Streaming(Box::new(body))).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_streaming_implicit() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            httpcodes::HTTPOk.build()
                .content_encoding(headers::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}
