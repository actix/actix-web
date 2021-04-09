#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;
#[cfg(feature = "rustls")]
extern crate tls_rustls as rustls;

use std::{
    future::Future,
    io::{Read, Write},
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::http::header::{
    ContentEncoding, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING,
};
use brotli2::write::{BrotliDecoder, BrotliEncoder};
use bytes::Bytes;
use cookie::{Cookie, CookieBuilder};
use flate2::{
    read::GzDecoder,
    write::{GzEncoder, ZlibDecoder, ZlibEncoder},
    Compression,
};
use futures_core::ready;
#[cfg(feature = "openssl")]
use openssl::{
    pkey::PKey,
    ssl::{SslAcceptor, SslMethod},
    x509::X509,
};
use rand::{distributions::Alphanumeric, Rng};

use actix_web::dev::BodyEncoding;
use actix_web::middleware::{Compress, NormalizePath, TrailingSlash};
use actix_web::{dev, web, App, Error, HttpResponse};

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

#[cfg(feature = "openssl")]
fn openssl_config() -> SslAcceptor {
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

struct TestBody {
    data: Bytes,
    chunk_size: usize,
    delay: Pin<Box<actix_rt::time::Sleep>>,
}

impl TestBody {
    fn new(data: Bytes, chunk_size: usize) -> Self {
        TestBody {
            data,
            chunk_size,
            delay: Box::pin(actix_rt::time::sleep(std::time::Duration::from_millis(10))),
        }
    }
}

impl futures_core::stream::Stream for TestBody {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        ready!(Pin::new(&mut self.delay).poll(cx));

        self.delay = Box::pin(actix_rt::time::sleep(std::time::Duration::from_millis(10)));
        let chunk_size = std::cmp::min(self.chunk_size, self.data.len());
        let chunk = self.data.split_to(chunk_size);
        if chunk.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(chunk)))
        }
    }
}

#[actix_rt::test]
async fn test_body() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))))
    });

    let mut response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_gzip() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_gzip2() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::to(|| {
                HttpResponse::Ok().body(STR).into_body::<dev::Body>()
            })))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_encoding_override() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::to(|| {
                HttpResponse::Ok()
                    .encoding(ContentEncoding::Deflate)
                    .body(STR)
            })))
            .service(web::resource("/raw").route(web::to(|| {
                let body = actix_web::dev::Body::Bytes(STR.into());
                let mut response =
                    HttpResponse::with_body(actix_web::http::StatusCode::OK, body);

                response.encoding(ContentEncoding::Deflate);

                response
            })))
    });

    // Builder
    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "deflate"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = ZlibDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));

    // Raw Response
    let mut response = srv
        .request(actix_web::http::Method::GET, srv.url("/raw"))
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "deflate"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = ZlibDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_gzip_large() {
    let data = STR.repeat(10);
    let srv_data = data.clone();

    let srv = actix_test::start_with(actix_test::config().h1(), move || {
        let data = srv_data.clone();
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(
                web::resource("/")
                    .route(web::to(move || HttpResponse::Ok().body(data.clone()))),
            )
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from(data));
}

#[actix_rt::test]
async fn test_body_gzip_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(70_000)
        .map(char::from)
        .collect::<String>();
    let srv_data = data.clone();

    let srv = actix_test::start_with(actix_test::config().h1(), move || {
        let data = srv_data.clone();
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(
                web::resource("/")
                    .route(web::to(move || HttpResponse::Ok().body(data.clone()))),
            )
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(dec.len(), data.len());
    assert_eq!(Bytes::from(dec), Bytes::from(data));
}

#[actix_rt::test]
async fn test_body_chunked_implicit() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::get().to(move || {
                HttpResponse::Ok()
                    .streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
            })))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .append_header((ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.headers().get(TRANSFER_ENCODING).unwrap(),
        &b"chunked"[..]
    );

    // read response
    let bytes = response.body().await.unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_br_streaming() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Br))
            .service(web::resource("/").route(web::to(move || {
                HttpResponse::Ok()
                    .streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
            })))
    });

    let mut response = srv
        .get("/")
        .append_header((ACCEPT_ENCODING, "br"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    println!("TEST: {:?}", bytes.len());

    // decode br
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    println!("T: {:?}", Bytes::copy_from_slice(&dec));
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_head_binary() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::head().to(move || HttpResponse::Ok().body(STR))),
        )
    });

    let mut response = srv.head("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response.headers().get(CONTENT_LENGTH).unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = response.body().await.unwrap();
    assert!(bytes.is_empty());
}

#[actix_rt::test]
async fn test_no_chunking() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move || {
            HttpResponse::Ok()
                .no_chunking(STR.len() as u64)
                .streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
        })))
    });

    let mut response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
    assert!(!response.headers().contains_key(TRANSFER_ENCODING));

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_deflate() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Deflate))
            .service(web::resource("/").route(web::to(move || HttpResponse::Ok().body(STR))))
    });

    // client request
    let mut response = srv
        .get("/")
        .append_header((ACCEPT_ENCODING, "deflate"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    let mut e = ZlibDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_brotli() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Br))
            .service(web::resource("/").route(web::to(move || HttpResponse::Ok().body(STR))))
    });

    // client request
    let mut response = srv
        .get("/")
        .append_header((ACCEPT_ENCODING, "br"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode brotli
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().wrap(Compress::default()).service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .insert_header((CONTENT_ENCODING, "gzip"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_gzip_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "gzip"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_gzip_encoding_large() {
    let data = STR.repeat(10);
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "gzip"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_reading_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(60_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "gzip"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_reading_deflate_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "deflate"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_reading_deflate_encoding_large() {
    let data = STR.repeat(10);
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "deflate"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_reading_deflate_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "deflate"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_brotli_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/").route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "br"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_brotli_encoding_large() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(320_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .app_data(web::PayloadConfig::new(320_000))
                .route(web::to(move |body: Bytes| {
                    HttpResponse::Ok().streaming(TestBody::new(body, 10240))
                })),
        )
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .append_header((CONTENT_ENCODING, "br"))
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().limit(320_000).await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(feature = "openssl")]
#[actix_rt::test]
async fn test_brotli_encoding_large_openssl() {
    let data = STR.repeat(10);
    let srv =
        actix_test::start_with(actix_test::config().openssl(openssl_config()), move || {
            App::new().service(web::resource("/").route(web::to(|bytes: Bytes| {
                HttpResponse::Ok()
                    .encoding(actix_web::http::ContentEncoding::Identity)
                    .body(bytes)
            })))
        });

    // body
    let mut e = BrotliEncoder::new(Vec::new(), 3);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let mut response = srv
        .post("/")
        .append_header((actix_web::http::header::CONTENT_ENCODING, "br"))
        .send_body(enc)
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(all(feature = "rustls", feature = "openssl"))]
mod plus_rustls {
    use std::io::BufReader;

    use rustls::{
        internal::pemfile::{certs, pkcs8_private_keys},
        NoClientAuth, ServerConfig as RustlsServerConfig,
    };

    use super::*;

    fn rustls_config() -> RustlsServerConfig {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let cert_file = cert.serialize_pem().unwrap();
        let key_file = cert.serialize_private_key_pem();

        let mut config = RustlsServerConfig::new(NoClientAuth::new());
        let cert_file = &mut BufReader::new(cert_file.as_bytes());
        let key_file = &mut BufReader::new(key_file.as_bytes());

        let cert_chain = certs(cert_file).unwrap();
        let mut keys = pkcs8_private_keys(key_file).unwrap();
        config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

        config
    }

    #[actix_rt::test]
    async fn test_reading_deflate_encoding_large_random_rustls() {
        let data = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(160_000)
            .map(char::from)
            .collect::<String>();

        let srv = actix_test::start_with(actix_test::config().rustls(rustls_config()), || {
            App::new().service(web::resource("/").route(web::to(|bytes: Bytes| {
                HttpResponse::Ok()
                    .encoding(actix_web::http::ContentEncoding::Identity)
                    .body(bytes)
            })))
        });

        // encode data
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data.as_ref()).unwrap();
        let enc = e.finish().unwrap();

        // client request
        let req = srv
            .post("/")
            .insert_header((actix_web::http::header::CONTENT_ENCODING, "deflate"))
            .send_stream(TestBody::new(Bytes::from(enc), 1024));

        let mut response = req.await.unwrap();
        assert!(response.status().is_success());

        // read response
        let bytes = response.body().await.unwrap();
        assert_eq!(bytes.len(), data.len());
        assert_eq!(bytes, Bytes::from(data));
    }
}

#[actix_rt::test]
async fn test_server_cookies() {
    use actix_web::http;

    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|| {
            HttpResponse::Ok()
                .cookie(
                    CookieBuilder::new("first", "first_value")
                        .http_only(true)
                        .finish(),
                )
                .cookie(Cookie::new("second", "first_value"))
                .cookie(Cookie::new("second", "second_value"))
                .finish()
        }))
    });

    let req = srv.get("/");
    let res = req.send().await.unwrap();
    assert!(res.status().is_success());

    let first_cookie = CookieBuilder::new("first", "first_value")
        .http_only(true)
        .finish();
    let second_cookie = Cookie::new("second", "second_value");

    let cookies = res.cookies().expect("To have cookies");
    assert_eq!(cookies.len(), 2);
    if cookies[0] == first_cookie {
        assert_eq!(cookies[1], second_cookie);
    } else {
        assert_eq!(cookies[0], second_cookie);
        assert_eq!(cookies[1], first_cookie);
    }

    let first_cookie = first_cookie.to_string();
    let second_cookie = second_cookie.to_string();
    // Check that we have exactly two instances of raw cookie headers
    let cookies = res
        .headers()
        .get_all(http::header::SET_COOKIE)
        .map(|header| header.to_str().expect("To str").to_string())
        .collect::<Vec<_>>();
    assert_eq!(cookies.len(), 2);
    if cookies[0] == first_cookie {
        assert_eq!(cookies[1], second_cookie);
    } else {
        assert_eq!(cookies[0], second_cookie);
        assert_eq!(cookies[1], first_cookie);
    }
}

#[actix_rt::test]
async fn test_slow_request() {
    use std::net;

    let srv = actix_test::start_with(actix_test::config().client_timeout(200), || {
        App::new().service(web::resource("/").route(web::to(HttpResponse::Ok)))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[actix_rt::test]
async fn test_normalize() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .service(web::resource("/one").route(web::to(|| HttpResponse::Ok().finish())))
    });

    let response = srv.get("/one/").send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_data_drop() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct TestData(Arc<AtomicUsize>);

    impl TestData {
        fn new(inner: Arc<AtomicUsize>) -> Self {
            let _ = inner.fetch_add(1, Ordering::SeqCst);
            Self(inner)
        }
    }

    impl Clone for TestData {
        fn clone(&self) -> Self {
            let inner = self.0.clone();
            let _ = inner.fetch_add(1, Ordering::SeqCst);
            Self(inner)
        }
    }

    impl Drop for TestData {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    let num = Arc::new(AtomicUsize::new(0));
    let data = TestData::new(num.clone());
    assert_eq!(num.load(Ordering::SeqCst), 1);

    let srv = actix_test::start(move || {
        let data = data.clone();

        App::new()
            .data(data)
            .service(web::resource("/").to(|_data: web::Data<TestData>| async { "ok" }))
    });

    assert!(srv.get("/").send().await.unwrap().status().is_success());
    srv.stop().await;

    assert_eq!(num.load(Ordering::SeqCst), 0);
}
