#![cfg(feature = "rustls-0_23")]

extern crate tls_rustls_023 as rustls;

use std::{
    convert::Infallible,
    io::{self, BufReader, Write},
    net::{SocketAddr, TcpStream as StdTcpStream},
    sync::Arc,
    task::Poll,
    time::Duration,
};

use actix_http::{
    body::{BodyStream, BoxBody, SizedStream},
    error::PayloadError,
    header::{self, HeaderName, HeaderValue},
    Error, HttpService, Method, Request, Response, StatusCode, TlsAcceptorConfig, Version,
};
use actix_http_test::test_server;
use actix_rt::pin;
use actix_service::{fn_factory_with_config, fn_service};
use actix_tls::connect::rustls_0_23::webpki_roots_cert_store;
use actix_utils::future::{err, ok, poll_fn};
use bytes::{Bytes, BytesMut};
use derive_more::{Display, Error};
use futures_core::{ready, Stream};
use futures_util::stream::once;
use rustls::{pki_types::ServerName, ServerConfig as RustlsServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};

async fn load_body<S>(stream: S) -> Result<BytesMut, PayloadError>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    let mut buf = BytesMut::new();

    pin!(stream);

    poll_fn(|cx| loop {
        let body = stream.as_mut();

        match ready!(body.poll_next(cx)) {
            Some(Ok(bytes)) => buf.extend_from_slice(&bytes),
            None => return Poll::Ready(Ok(())),
            Some(Err(err)) => return Poll::Ready(Err(err)),
        }
    })
    .await?;

    Ok(buf)
}

fn tls_config() -> RustlsServerConfig {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

    let cert_file = &mut BufReader::new(cert_file.as_bytes());
    let key_file = &mut BufReader::new(key_file.as_bytes());

    let cert_chain = certs(cert_file).collect::<Result<Vec<_>, _>>().unwrap();
    let mut keys = pkcs8_private_keys(key_file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let mut config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            cert_chain,
            rustls::pki_types::PrivateKeyDer::Pkcs8(keys.remove(0)),
        )
        .unwrap();

    config.alpn_protocols.push(HTTP1_1_ALPN_PROTOCOL.to_vec());
    config.alpn_protocols.push(H2_ALPN_PROTOCOL.to_vec());

    config
}

pub fn get_negotiated_alpn_protocol(
    addr: SocketAddr,
    client_alpn_protocol: &[u8],
) -> Option<Vec<u8>> {
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(webpki_roots_cert_store())
        .with_no_client_auth();

    config.alpn_protocols.push(client_alpn_protocol.to_vec());

    let mut sess =
        rustls::ClientConnection::new(Arc::new(config), ServerName::try_from("localhost").unwrap())
            .unwrap();

    let mut sock = StdTcpStream::connect(addr).unwrap();
    let mut stream = rustls::Stream::new(&mut sess, &mut sock);

    // The handshake will fails because the client will not be able to verify the server
    // certificate, but it doesn't matter here as we are just interested in the negotiated ALPN
    // protocol
    let _ = stream.flush();

    sess.alpn_protocol().map(|proto| proto.to_vec())
}

#[actix_rt::test]
async fn h1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h1(|_| ok::<_, Error>(Response::ok()))
            .rustls_0_23(tls_config())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn h2() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Error>(Response::ok()))
            .rustls_0_23(tls_config())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn h1_1() -> io::Result<()> {
    let srv = test_server(move || {
        HttpService::build()
            .h1(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), Version::HTTP_11);
                ok::<_, Error>(Response::ok())
            })
            .rustls_0_23(tls_config())
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
            .rustls_0_23_with_config(
                tls_config(),
                TlsAcceptorConfig::default().handshake_timeout(Duration::from_secs(5)),
            )
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[actix_rt::test]
async fn h2_body1() -> io::Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024);
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|mut req: Request<_>| async move {
                let body = load_body(req.take_payload()).await?;
                Ok::<_, Error>(Response::ok().set_body(body))
            })
            .rustls_0_23(tls_config())
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
                let indx: usize = req.uri().path()[1..].parse().unwrap();
                let statuses = [
                    StatusCode::CONTINUE,
                    StatusCode::NO_CONTENT,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                ];
                ok::<_, Infallible>(Response::new(statuses[indx]))
            })
            .rustls_0_23(tls_config())
    })
    .await;

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        #[allow(clippy::single_element_loop)]
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

        #[allow(clippy::single_element_loop)]
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
async fn h2_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test_server(move || {
        let data = data.clone();
        HttpService::build()
            .h2(move |_| {
                let mut config = Response::build(StatusCode::OK);
                for idx in 0..90 {
                    config.insert_header((
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
                ));
                }
                ok::<_, Infallible>(config.body(data.clone()))
            })
            .rustls_0_23(tls_config())
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
            .rustls_0_23(tls_config())
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
            .rustls_0_23(tls_config())
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
async fn h2_head_binary() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .rustls_0_23(tls_config())
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
async fn h2_head_binary2() {
    let srv = test_server(move || {
        HttpService::build()
            .h2(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .rustls_0_23(tls_config())
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
async fn h2_body_length() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| {
                let body = once(ok::<_, Infallible>(Bytes::from_static(STR.as_ref())));
                ok::<_, Infallible>(
                    Response::ok().set_body(SizedStream::new(STR.len() as u64, body)),
                )
            })
            .rustls_0_23(tls_config())
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
            .rustls_0_23(tls_config())
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
            .h2(fn_factory_with_config(|_: ()| {
                ok::<_, Infallible>(fn_service(|_| {
                    let broken_header = Bytes::from_static(b"\0\0\0");
                    ok::<_, Infallible>(
                        Response::build(StatusCode::OK)
                            .insert_header((http::header::CONTENT_TYPE, broken_header))
                            .body(STR),
                    )
                }))
            }))
            .rustls_0_23(tls_config())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

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
    fn from(_: BadRequest) -> Self {
        Response::bad_request().set_body(BoxBody::new("error"))
    }
}

#[actix_rt::test]
async fn h2_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h2(|_| err::<Response<BoxBody>, _>(BadRequest))
            .rustls_0_23(tls_config())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

#[actix_rt::test]
async fn h1_service_error() {
    let mut srv = test_server(move || {
        HttpService::build()
            .h1(|_| err::<Response<BoxBody>, _>(BadRequest))
            .rustls_0_23(tls_config())
    })
    .await;

    let response = srv.sget("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

const H2_ALPN_PROTOCOL: &[u8] = b"h2";
const HTTP1_1_ALPN_PROTOCOL: &[u8] = b"http/1.1";
const CUSTOM_ALPN_PROTOCOL: &[u8] = b"custom";

#[actix_rt::test]
async fn alpn_h1() -> io::Result<()> {
    let srv = test_server(move || {
        let mut config = tls_config();
        config.alpn_protocols.push(CUSTOM_ALPN_PROTOCOL.to_vec());
        HttpService::build()
            .h1(|_| ok::<_, Error>(Response::ok()))
            .rustls_0_23(config)
    })
    .await;

    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), CUSTOM_ALPN_PROTOCOL),
        Some(CUSTOM_ALPN_PROTOCOL.to_vec())
    );

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    Ok(())
}

#[actix_rt::test]
async fn alpn_h2() -> io::Result<()> {
    let srv = test_server(move || {
        let mut config = tls_config();
        config.alpn_protocols.push(CUSTOM_ALPN_PROTOCOL.to_vec());
        HttpService::build()
            .h2(|_| ok::<_, Error>(Response::ok()))
            .rustls_0_23(config)
    })
    .await;

    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), H2_ALPN_PROTOCOL),
        Some(H2_ALPN_PROTOCOL.to_vec())
    );
    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), CUSTOM_ALPN_PROTOCOL),
        Some(CUSTOM_ALPN_PROTOCOL.to_vec())
    );

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    Ok(())
}

#[actix_rt::test]
async fn alpn_h2_1() -> io::Result<()> {
    let srv = test_server(move || {
        let mut config = tls_config();
        config.alpn_protocols.push(CUSTOM_ALPN_PROTOCOL.to_vec());
        HttpService::build()
            .finish(|_| ok::<_, Error>(Response::ok()))
            .rustls_0_23(config)
    })
    .await;

    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), H2_ALPN_PROTOCOL),
        Some(H2_ALPN_PROTOCOL.to_vec())
    );
    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), HTTP1_1_ALPN_PROTOCOL),
        Some(HTTP1_1_ALPN_PROTOCOL.to_vec())
    );
    assert_eq!(
        get_negotiated_alpn_protocol(srv.addr(), CUSTOM_ALPN_PROTOCOL),
        Some(CUSTOM_ALPN_PROTOCOL.to_vec())
    );

    let response = srv.sget("/").send().await.unwrap();
    assert!(response.status().is_success());

    Ok(())
}
