extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate flate2;
extern crate rand;

use std::io::Read;

use bytes::Bytes;
use futures::Future;
use futures::stream::once;
use flate2::read::GzDecoder;
use rand::Rng;

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
fn test_with_query_parameter() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|req: HttpRequest| match req.query().get("qp") {
            Some(_) => httpcodes::HTTPOk.build().finish(),
            None => httpcodes::HTTPBadRequest.build().finish(),
        }));

    let request = srv.get().uri(srv.url("/?qp=5").as_str()).finish().unwrap();

    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
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
                   .content_encoding(header::ContentEncoding::Deflate)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(header::ContentEncoding::Gzip)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_gzip_encoding_large() {
    let data = STR.repeat(10);

    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Deflate)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(header::ContentEncoding::Gzip)
        .body(data.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_client_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .gen_ascii_chars()
        .take(100_000)
        .collect::<String>();

    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Deflate)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(header::ContentEncoding::Gzip)
        .body(data.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(feature="brotli")]
#[test]
fn test_client_brotli_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Gzip)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.client(Method::POST, "/")
        .content_encoding(header::ContentEncoding::Br)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[cfg(feature="brotli")]
#[test]
fn test_client_brotli_encoding_large_random() {
    let data = rand::thread_rng()
        .gen_ascii_chars()
        .take(70_000)
        .collect::<String>();

    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(move |bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Gzip)
                   .body(bytes))
            }).responder()}
        ));

    // client request
    let request = srv.client(Method::POST, "/")
        .content_encoding(header::ContentEncoding::Br)
        .body(data.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(feature="brotli")]
#[test]
fn test_client_deflate_encoding() {
    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Br)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(header::ContentEncoding::Deflate)
        .body(STR).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[cfg(feature="brotli")]
#[test]
fn test_client_deflate_encoding_large_random() {
    let data = rand::thread_rng()
        .gen_ascii_chars()
        .take(70_000)
        .collect::<String>();

    let mut srv = test::TestServer::new(|app| app.handler(|req: HttpRequest| {
        req.body()
            .and_then(|bytes: Bytes| {
                Ok(httpcodes::HTTPOk
                   .build()
                   .content_encoding(header::ContentEncoding::Br)
                   .body(bytes))
            }).responder()}
    ));

    // client request
    let request = srv.post()
        .content_encoding(header::ContentEncoding::Deflate)
        .body(data.clone()).unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
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
                       .content_encoding(header::ContentEncoding::Identity)
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
                .content_encoding(header::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))}));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_cookie_handling() {
    use actix_web::header::Cookie;
    fn err() -> Error {
        use std::io::{ErrorKind, Error as IoError};
        // stub some generic error
        Error::from(IoError::from(ErrorKind::NotFound))
    }
    let cookie1 = Cookie::build("cookie1", "value1").finish();
    let cookie2 = Cookie::build("cookie2", "value2")
        .domain("www.example.org")
        .path("/")
        .secure(true)
        .http_only(true)
        .finish();
    // Q: are all these clones really necessary? A: Yes, possibly
    let cookie1b = cookie1.clone();
    let cookie2b = cookie2.clone();
    let mut srv = test::TestServer::new(
        move |app| {
            let cookie1 = cookie1b.clone();
            let cookie2 = cookie2b.clone();
            app.handler(move |req: HttpRequest| {
                // Check cookies were sent correctly
                req.cookie("cookie1").ok_or_else(err)
                    .and_then(|c1| if c1.value() == "value1" {
                        Ok(())
                    } else {
                        Err(err())
                    })
                    .and_then(|()| req.cookie("cookie2").ok_or_else(err))
                    .and_then(|c2| if c2.value() == "value2" {
                        Ok(())
                    } else {
                        Err(err())
                    })
                    // Send some cookies back
                    .map(|_|
                         httpcodes::HTTPOk.build()
                         .cookie(cookie1.clone())
                         .cookie(cookie2.clone())
                         .finish()
                    )
            })
        });

    let request = srv.get()
        .cookie(cookie1.clone())
        .cookie(cookie2.clone())
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
    let c1 = response.cookie("cookie1").expect("Missing cookie1");
    assert_eq!(c1, &cookie1);
    let c2 = response.cookie("cookie2").expect("Missing cookie2");
    assert_eq!(c2, &cookie2);
}
