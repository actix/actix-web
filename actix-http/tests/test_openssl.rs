#![cfg(feature = "openssl")]

extern crate tls_openssl as openssl;

use std::io;

use actix_http::{
    body::{Body, SizedStream},
    error::{ErrorBadRequest, PayloadError},
    http::{
        header::{self, HeaderName, HeaderValue},
        Method, StatusCode, Version,
    },
    Error, HttpMessage, HttpService, Request, Response,
};
use actix_http_test::test_server;
use actix_service::{fn_service, ServiceFactoryExt};
use actix_utils::future::{err, ok, ready};
use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_util::stream::{once, StreamExt as _};
use openssl::{
    pkey::PKey,
    ssl::{SslAcceptor, SslMethod},
    x509::X509,
};

async fn load_body<S>(stream: S) -> Result<BytesMut, PayloadError>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    let body = stream
        .map(|res| match res {
            Ok(chunk) => chunk,
            Err(_) => panic!(),
        })
        .fold(BytesMut::new(), move |mut body, chunk| {
            body.extend_from_slice(&chunk);
            ready(body)
        })
        .await;

    Ok(body)
}

fn tls_config() -> SslAcceptor {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_file = cert.serialize_pem().unwrap();
    let key_file = cert.serialize_private_key_pem();
    let cert = X509::from_pem(cert_file.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key_file.as_bytes()).unwrap();

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&key).unwrap();

    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(openssl::ssl::AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x02h2").unwrap();

    builder.build()
}

#[actix_rt::test]
async fn test_h2() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Error>(Response::Ok().finish()))
            .openssl(tls_config())
            .map_err(|_| ())
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
                ok::<_, Error>(Response::Ok().finish())
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn test_h2_body() -> io::Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024);
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|mut req: Request<_>| async move {
                let body = load_body(req.take_payload()).await?;
                Ok::<_, Error>(Response::Ok().body(body))
            })
            .openssl(tls_config())
            .map_err(|_| ())
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
                let idx: usize = req.uri().path()[1..].parse().unwrap();
                let statuses = [
                    StatusCode::CONTINUE,
                    StatusCode::NO_CONTENT,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                ];
                ok::<_, ()>(Response::new(statuses[idx]))
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for &i in &[0] {
            let req = srv
                .request(Method::HEAD, srv.surl(&format!("/{}", i)))
                .send();
            let _response = req.await.expect_err("should timeout on recv 1xx frame");
            // assert_eq!(response.headers().get(&header), None);

            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let _response = req.await.expect_err("should timeout on recv 1xx frame");
            // assert_eq!(response.headers().get(&header), None);
        }

        for &i in &[1] {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = req.await.unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for &i in &[2, 3] {
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
            let mut builder = Response::Ok();
            for idx in 0..90 {
                builder.insert_header(
                    (format!("X-TEST-{}", idx).as_str(),
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
                ));
            }
            ok::<_, ()>(builder.body(data.clone()))
        })
            .openssl(tls_config())
                    .map_err(|_| ())
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
            .h2(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .openssl(tls_config())
            .map_err(|_| ())
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
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), Version::HTTP_2);

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
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
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
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
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.shead("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(header::CONTENT_LENGTH).unwrap();
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
                    Response::Ok().body(SizedStream::new(STR.len() as u64, body)),
                )
            })
            .openssl(tls_config())
            .map_err(|_| ())
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
                        .insert_header((header::TRANSFER_ENCODING, "chunked"))
                        .streaming(body),
                )
            })
            .openssl(tls_config())
            .map_err(|_| ())
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
            .h2(fn_service(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, ()>(
                    Response::Ok()
                        .insert_header((header::CONTENT_TYPE, broken_header))
                        .body(STR),
                )
            }))
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"failed to parse header value"));
}

#[actix_rt::test]
async fn test_h2_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| err::<Response<Body>, Error>(ErrorBadRequest("error")))
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

#[actix_rt::test]
async fn test_h2_on_connect() {
    let srv = test_server(move || {
        HttpService::build()
            .on_connect_ext(|_, data| {
                data.insert(20isize);
            })
            .h2(|req: Request| {
                assert!(req.extensions().contains::<isize>());
                ok::<_, ()>(Response::Ok().finish())
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
}
