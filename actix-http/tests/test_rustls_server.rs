#![cfg(feature = "rust-tls")]
use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::error::PayloadError;
use actix_http::http::header::{self, HeaderName, HeaderValue};
use actix_http::http::{Method, StatusCode, Version};
use actix_http::{body, error, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use actix_server::ssl::RustlsAcceptor;
use actix_server_config::ServerConfig;
use actix_service::{new_service_cfg, NewService};

use bytes::{Bytes, BytesMut};
use futures::future::{self, ok, Future};
use futures::stream::{once, Stream};
use rustls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    NoClientAuth, ServerConfig as RustlsServerConfig,
};

use std::fs::File;
use std::io::{BufReader, Result};

fn load_body<S>(stream: S) -> impl Future<Item = BytesMut, Error = PayloadError>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    stream.fold(BytesMut::new(), move |mut body, chunk| {
        body.extend_from_slice(&chunk);
        Ok::<_, PayloadError>(body)
    })
}

fn ssl_acceptor<T: AsyncRead + AsyncWrite>() -> Result<RustlsAcceptor<T, ()>> {
    // load ssl keys
    let mut config = RustlsServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("../tests/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("../tests/key.pem").unwrap());
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    let protos = vec![b"h2".to_vec()];
    config.set_protocols(&protos);
    Ok(RustlsAcceptor::new(config))
}

#[test]
fn test_h2() -> Result<()> {
    let rustls = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| future::ok::<_, Error>(Response::Ok().finish()))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[test]
fn test_h2_1() -> Result<()> {
    let rustls = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .finish(|req: Request| {
                        assert!(req.peer_addr().is_some());
                        assert_eq!(req.version(), Version::HTTP_2);
                        future::ok::<_, Error>(Response::Ok().finish())
                    })
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[test]
fn test_h2_body() -> Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024);
    let rustls = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|mut req: Request<_>| {
                        load_body(req.take_payload())
                            .and_then(|body| Ok(Response::Ok().body(body)))
                    })
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send_body(data.clone())).unwrap();
    assert!(response.status().is_success());

    let body = srv.load_body(response).unwrap();
    assert_eq!(&body, data.as_bytes());
    Ok(())
}

#[test]
fn test_h2_content_length() {
    let rustls = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|req: Request| {
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
                    })
                    .map_err(|_| ()),
            )
    });

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = srv
                .request(Method::HEAD, srv.surl(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_h2_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();
    let rustls = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        let data = data.clone();
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
        HttpService::build().h2(move |_| {
            let mut config = Response::Ok();
            for idx in 0..90 {
                config.header(
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
            future::ok::<_, ()>(config.body(data.clone()))
        }).map_err(|_| ()))
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
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
fn test_h2_body2() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_head_empty() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.shead("/").send()).unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_head_binary() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        ok::<_, ()>(
                            Response::Ok().content_length(STR.len() as u64).body(STR),
                        )
                    })
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.shead("/").send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_head_binary2() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.shead("/").send()).unwrap();
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
fn test_h2_body_length() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        let body = once(Ok(Bytes::from_static(STR.as_ref())));
                        ok::<_, ()>(
                            Response::Ok()
                                .body(body::SizedStream::new(STR.len() as u64, body)),
                        )
                    })
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_body_chunked_explicit() {
    let rustls = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        let body =
                            once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
                        ok::<_, ()>(
                            Response::Ok()
                                .header(header::TRANSFER_ENCODING, "chunked")
                                .streaming(body),
                        )
                    })
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert!(response.status().is_success());
    assert!(!response.headers().contains_key(header::TRANSFER_ENCODING));

    // read response
    let bytes = srv.load_body(response).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_response_http_error_handling() {
    let rustls = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(new_service_cfg(|_: &ServerConfig| {
                        Ok::<_, ()>(|_| {
                            let broken_header = Bytes::from_static(b"\0\0\0");
                            ok::<_, ()>(
                                Response::Ok()
                                    .header(http::header::CONTENT_TYPE, broken_header)
                                    .body(STR),
                            )
                        })
                    }))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"failed to parse header value"));
}

#[test]
fn test_h2_service_error() {
    let rustls = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        rustls
            .clone()
            .map_err(|e| println!("Rustls error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| Err::<Response, Error>(error::ErrorBadRequest("error")))
                    .map_err(|_| ()),
            )
    });

    let response = srv.block_on(srv.sget("/").send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert!(bytes.is_empty());
}
