#![cfg(feature = "openssl")]

extern crate tls_openssl as openssl;

use std::{convert::Infallible, io, time::Duration};

use actix_http::{
    body::{BodyStream, BoxBody, SizedStream},
    error::PayloadError,
    header::{self, HeaderValue},
    Error, HttpService, Method, Request, Response, StatusCode, TlsAcceptorConfig, Version,
};
use actix_http_test::test_server;
use actix_service::{fn_service, ServiceFactoryExt};
use actix_utils::future::{err, ok, ready};
use bytes::{Bytes, BytesMut};
use derive_more::{Display, Error};
use futures_core::Stream;
use futures_util::{stream::once, StreamExt as _};
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
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

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
async fn h2() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Error>(Response::ok()))
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn h2_1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .finish(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), Version::HTTP_2);
                ok::<_, Error>(Response::ok())
            })
            .openssl_with_config(
                tls_config(),
                TlsAcceptorConfig::default().handshake_timeout(Duration::from_secs(5)),
            )
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn h2_body() -> io::Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024); // 640 KiB
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|mut req: Request<_>| async move {
                let body = load_body(req.take_payload()).await?;
                Ok::<_, Error>(Response::ok().set_body(body))
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
async fn h2_content_length() {
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
                ok::<_, Infallible>(Response::new(statuses[idx]))
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    static VALUE: HeaderValue = HeaderValue::from_static("0");

    {
        let req = srv.request(Method::HEAD, srv.surl("/0")).send();
        req.await.expect_err("should timeout on recv 1xx frame");

        let req = srv.request(Method::GET, srv.surl("/0")).send();
        req.await.expect_err("should timeout on recv 1xx frame");

        let req = srv.request(Method::GET, srv.surl("/1")).send();
        let response = req.await.unwrap();
        assert!(response.headers().get("content-length").is_none());

        for &i in &[2, 3] {
            let req = srv
                .request(Method::GET, srv.surl(&format!("/{}", i)))
                .send();
            let response = req.await.unwrap();
            assert_eq!(response.headers().get("content-length"), Some(&VALUE));
        }
    }
}

#[actix_rt::test]
async fn h2_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test_server(move || {
        let data = data.clone();
        HttpService::build()
            .h2(move |_| {
                let mut builder = Response::build(StatusCode::OK);
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
                ok::<_, Infallible>(builder.body(data.clone()))
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

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
async fn h2_body2() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
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
async fn h2_head_empty() {
    let mut srv = test_server(move || {
        HttpService::build()
            .finish(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
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
async fn h2_head_binary() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
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
async fn h2_head_binary2() {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
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
async fn h2_body_length() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| async {
                let body = once(async { Ok::<_, Infallible>(Bytes::from_static(STR.as_ref())) });

                Ok::<_, Infallible>(
                    Response::ok().set_body(SizedStream::new(STR.len() as u64, body)),
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
async fn h2_body_chunked_explicit() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, Infallible>(
                    Response::build(StatusCode::OK)
                        .insert_header((header::TRANSFER_ENCODING, "chunked"))
                        .body(BodyStream::new(body)),
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
async fn h2_response_http_error_handling() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(fn_service(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, Infallible>(
                    Response::build(StatusCode::OK)
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
    assert_eq!(
        bytes,
        Bytes::from_static(b"error processing HTTP: failed to parse header value")
    );
}

#[derive(Debug, Display, Error)]
#[display(fmt = "error")]
struct BadRequest;

impl From<BadRequest> for Response<BoxBody> {
    fn from(err: BadRequest) -> Self {
        Response::build(StatusCode::BAD_REQUEST)
            .body(err.to_string())
            .map_into_boxed_body()
    }
}

#[actix_rt::test]
async fn h2_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| err::<Response<BoxBody>, _>(BadRequest))
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
async fn h2_on_connect() {
    let srv = test_server(move || {
        HttpService::build()
            .on_connect_ext(|_, data| {
                data.insert(20isize);
            })
            .h2(|req: Request| {
                assert!(req.conn_data::<isize>().is_some());
                ok::<_, Infallible>(Response::ok())
            })
            .openssl(tls_config())
            .map_err(|_| ())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
}
