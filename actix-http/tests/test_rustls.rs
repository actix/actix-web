#![cfg(feature = "rustls")]
use actix_http::error::PayloadError;
use actix_http::http::header::{self, HeaderName, HeaderValue};
use actix_http::http::{Method, StatusCode, Version};
use actix_http::{body, error, Error, HttpService, Request, Response};
use actix_http_test::test_server;
use actix_service::{fn_factory_with_config, fn_service};

use bytes::{Bytes, BytesMut};
use futures_util::future::{self, err, ok};
use futures_util::stream::{once, Stream, StreamExt};
use rust_tls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    NoClientAuth, ServerConfig as RustlsServerConfig,
};

use std::fs::File;
use std::io::{self, BufReader};

async fn load_body<S>(mut stream: S) -> Result<BytesMut, PayloadError>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    let mut body = BytesMut::new();
    while let Some(item) = stream.next().await {
        body.extend_from_slice(&item?)
    }
    Ok(body)
}

fn ssl_acceptor() -> RustlsServerConfig {
    // load ssl keys
    let mut config = RustlsServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("../tests/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("../tests/key.pem").unwrap());
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();
    config
}

#[actix_rt::test]
async fn test_h1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h1(|_| future::ok::<_, Error>(Response::Ok().finish()))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn test_h2() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| future::ok::<_, Error>(Response::Ok().finish()))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn test_h1_1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h1(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), Version::HTTP_11);
                future::ok::<_, Error>(Response::Ok().finish())
            })
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn test_h2_1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .finish(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), Version::HTTP_2);
                future::ok::<_, Error>(Response::Ok().finish())
            })
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn test_h2_body1() -> io::Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024);
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|mut req: Request<_>| async move {
                let body = load_body(req.take_payload()).await?;
                Ok::<_, Error>(Response::Ok().body(body))
            })
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send_body(data.clone()).await.unwrap();
    assert!(response.status().is_success());

    let body = srv.load_body(response).await.unwrap();
    assert_eq!(&body, data.as_bytes());
    Ok(())
}

#[actix_rt::test]
async fn test_h2_content_length() {
    let srv = test_server(move || {
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
            .rustls(ssl_acceptor())
    })
    .await;

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");
    {
        for i in 0..4 {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = req.await.unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = srv
                .request(Method::HEAD, srv.surl(&format!("/{}", i)))
                .send();
            let response = req.await.unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = req.await.unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[actix_rt::test]
async fn test_h2_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test_server(move || {
        let data = data.clone();
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
        })
            .rustls(ssl_acceptor())
    }).await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
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

#[actix_rt::test]
async fn test_h2_body2() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_h2_head_empty() {
    let mut srv = test_server(move || {
        HttpService::build()
            .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
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
    let bytes = srv.load_body(response).await.unwrap();
    assert!(bytes.is_empty());
}

#[actix_rt::test]
async fn test_h2_head_binary() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert!(bytes.is_empty());
}

#[actix_rt::test]
async fn test_h2_head_binary2() {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }
}

#[actix_rt::test]
async fn test_h2_body_length() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| {
                let body = once(ok(Bytes::from_static(STR.as_ref())));
                ok::<_, ()>(
                    Response::Ok().body(body::SizedStream::new(STR.len() as u64, body)),
                )
            })
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_h2_body_chunked_explicit() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, ()>(
                    Response::Ok()
                        .header(header::TRANSFER_ENCODING, "chunked")
                        .streaming(body),
                )
            })
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    assert!(!response.headers().contains_key(header::TRANSFER_ENCODING));

    // read response
    let bytes = srv.load_body(response).await.unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_h2_response_http_error_handling() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(fn_factory_with_config(|_: ()| {
                ok::<_, ()>(fn_service(|_| {
                    let broken_header = Bytes::from_static(b"\0\0\0");
                    ok::<_, ()>(
                        Response::Ok()
                            .header(http::header::CONTENT_TYPE, broken_header)
                            .body(STR),
                    )
                }))
            }))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"failed to parse header value"));
}

#[actix_rt::test]
async fn test_h2_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| err::<Response, Error>(error::ErrorBadRequest("error")))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

#[actix_rt::test]
async fn test_h1_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h1(|_| err::<Response, Error>(error::ErrorBadRequest("error")))
            .rustls(ssl_acceptor())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}
