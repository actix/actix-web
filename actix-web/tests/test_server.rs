#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;
#[cfg(feature = "rustls-0_23")]
extern crate tls_rustls as rustls;

use std::{
    future::Future,
    io::{Read, Write},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_web::{
    cookie::Cookie,
    http::{header, StatusCode},
    middleware::{Compress, NormalizePath, TrailingSlash},
    web, App, Error, HttpResponse,
};
use bytes::Bytes;
use futures_core::ready;
#[cfg(feature = "openssl")]
use openssl::{
    pkey::PKey,
    ssl::{SslAcceptor, SslMethod},
    x509::X509,
};
use rand::{distributions::Alphanumeric, Rng as _};

mod utils;

const S: &str = "Hello World ";
const STR: &str = const_str::repeat!(S, 100);

#[cfg(feature = "openssl")]
fn openssl_config() -> SslAcceptor {
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
        App::new()
            .service(web::resource("/").route(web::to(|| async { HttpResponse::Ok().body(STR) })))
    });

    let mut res = srv.get("/").send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

// enforcing an encoding per-response is removed
// #[actix_rt::test]
// async fn test_body_encoding_override() {
//     let srv = actix_test::start_with(actix_test::config().h1(), || {
//         App::new()
//             .wrap(Compress::default())
//             .service(web::resource("/").route(web::to(|| {
//                 HttpResponse::Ok()
//                     .encode_with(ContentEncoding::Deflate)
//                     .body(STR)
//             })))
//             .service(web::resource("/raw").route(web::to(|| {
//                 let mut res = HttpResponse::with_body(actix_web::http::StatusCode::OK, STR);
//                 res.encode_with(ContentEncoding::Deflate);
//                 res.map_into_boxed_body()
//             })))
//     });

//     // Builder
//     let mut res = srv
//         .get("/")
//         .no_decompress()
//         .append_header((ACCEPT_ENCODING, "deflate"))
//         .send()
//         .await
//         .unwrap();
//     assert_eq!(res.status(), StatusCode::OK);

//     let bytes = res.body().await.unwrap();
//     assert_eq!(utils::deflate::decode(bytes), STR.as_bytes());

//     // Raw Response
//     let mut res = srv
//         .request(actix_web::http::Method::GET, srv.url("/raw"))
//         .no_decompress()
//         .append_header((ACCEPT_ENCODING, "deflate"))
//         .send()
//         .await
//         .unwrap();
//     assert_eq!(res.status(), StatusCode::OK);

//     let bytes = res.body().await.unwrap();
//     assert_eq!(utils::deflate::decode(bytes), STR.as_bytes());

//     srv.stop().await;
// }

#[actix_rt::test]
async fn body_gzip_large() {
    let data = STR.repeat(10);
    let srv_data = data.clone();

    let srv = actix_test::start_with(actix_test::config().h1(), move || {
        let data = srv_data.clone();

        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(move || {
                let data = data.clone();
                async move { HttpResponse::Ok().body(data.clone()) }
            })))
    });

    let mut res = srv
        .get("/")
        .no_decompress()
        .append_header((header::ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), data.as_bytes());

    srv.stop().await;
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
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(move || {
                let data = data.clone();
                async move { HttpResponse::Ok().body(data.clone()) }
            })))
    });

    let mut res = srv
        .get("/")
        .no_decompress()
        .append_header((header::ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), data.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_chunked_implicit() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::get().to(|| async {
                HttpResponse::Ok().streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
            })))
    });

    let mut res = srv
        .get("/")
        .no_decompress()
        .append_header((header::ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::TRANSFER_ENCODING).unwrap(),
        "chunked"
    );

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_br_streaming() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(|| async {
                HttpResponse::Ok().streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
            })))
    });

    let mut res = srv
        .get("/")
        .append_header((header::ACCEPT_ENCODING, "br"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::brotli::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_head_binary() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::head().to(move || async { HttpResponse::Ok().body(STR) })),
        )
    });

    let mut res = srv.head("/").send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let len = res.headers().get(header::CONTENT_LENGTH).unwrap();
    assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());

    let bytes = res.body().await.unwrap();
    assert!(bytes.is_empty());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_no_chunking() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move || async {
            HttpResponse::Ok()
                .no_chunking(STR.len() as u64)
                .streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
        })))
    });

    let mut res = srv.get("/").send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!res.headers().contains_key(header::TRANSFER_ENCODING));

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_deflate() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().wrap(Compress::default()).service(
            web::resource("/").route(web::to(move || async { HttpResponse::Ok().body(STR) })),
        )
    });

    let mut res = srv
        .get("/")
        .append_header((header::ACCEPT_ENCODING, "deflate"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::deflate::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_brotli() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().wrap(Compress::default()).service(
            web::resource("/").route(web::to(move || async { HttpResponse::Ok().body(STR) })),
        )
    });

    let mut res = srv
        .get("/")
        .append_header((header::ACCEPT_ENCODING, "br"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::brotli::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_zstd() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().wrap(Compress::default()).service(
            web::resource("/").route(web::to(move || async { HttpResponse::Ok().body(STR) })),
        )
    });

    let mut res = srv
        .get("/")
        .append_header((header::ACCEPT_ENCODING, "zstd"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::zstd::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_body_zstd_streaming() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(move || async {
                HttpResponse::Ok().streaming(TestBody::new(Bytes::from_static(STR.as_ref()), 24))
            })))
    });

    let mut res = srv
        .get("/")
        .append_header((header::ACCEPT_ENCODING, "zstd"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::zstd::decode(bytes), STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_zstd_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "zstd"))
        .send_body(utils::zstd::encode(STR));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_zstd_encoding_large() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(320_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .app_data(web::PayloadConfig::new(320_000))
                .route(web::to(move |body: Bytes| async {
                    HttpResponse::Ok().streaming(TestBody::new(body, 10240))
                })),
        )
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "zstd"))
        .send_body(utils::zstd::encode(&data));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().limit(320_000).await.unwrap();
    assert_eq!(bytes, data.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(move |body: Bytes| async {
                HttpResponse::Ok().body(body)
            })))
    });

    let request = srv
        .post("/")
        .insert_header((header::CONTENT_ENCODING, "gzip"))
        .send_body(utils::gzip::encode(STR));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_gzip_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "gzip"))
        .send_body(utils::gzip::encode(STR));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, STR.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_gzip_encoding_large() {
    let data = STR.repeat(10);
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let req = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "gzip"))
        .send_body(utils::gzip::encode(&data));
    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, data);

    srv.stop().await;
}

#[actix_rt::test]
async fn test_reading_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(60_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "gzip"))
        .send_body(utils::gzip::encode(&data));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, data.as_bytes());

    srv.stop().await;
}

#[actix_rt::test]
async fn test_reading_deflate_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "deflate"))
        .send_body(utils::deflate::encode(STR));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_reading_deflate_encoding_large() {
    let data = STR.repeat(10);
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "deflate"))
        .send_body(utils::deflate::encode(&data));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_reading_deflate_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "deflate"))
        .send_body(utils::deflate::encode(&data));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_brotli_encoding() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move |body: Bytes| async {
            HttpResponse::Ok().body(body)
        })))
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "br"))
        .send_body(utils::brotli::encode(STR));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
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
                .route(web::to(move |body: Bytes| async {
                    HttpResponse::Ok().streaming(TestBody::new(body, 10240))
                })),
        )
    });

    let request = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "br"))
        .send_body(utils::brotli::encode(&data));
    let mut res = request.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().limit(320_000).await.unwrap();
    assert_eq!(bytes, Bytes::from(data));

    srv.stop().await;
}

#[cfg(feature = "openssl")]
#[actix_rt::test]
async fn test_brotli_encoding_large_openssl() {
    use actix_web::http::header;

    let data = STR.repeat(10);
    let srv = actix_test::start_with(actix_test::config().openssl(openssl_config()), move || {
        App::new().service(web::resource("/").route(web::to(|bytes: Bytes| async {
            // echo decompressed request body back in response
            HttpResponse::Ok()
                .insert_header(header::ContentEncoding::Identity)
                .body(bytes)
        })))
    });

    let mut res = srv
        .post("/")
        .append_header((header::CONTENT_ENCODING, "br"))
        .send_body(utils::brotli::encode(&data))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));

    srv.stop().await;
}

#[cfg(feature = "rustls-0_23")]
mod plus_rustls {
    use std::io::BufReader;

    use rustls::{pki_types::PrivateKeyDer, ServerConfig as RustlsServerConfig};
    use rustls_pemfile::{certs, pkcs8_private_keys};

    use super::*;

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

        RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, PrivateKeyDer::Pkcs8(keys.remove(0)))
            .unwrap()
    }

    #[actix_rt::test]
    async fn test_reading_deflate_encoding_large_random_rustls() {
        let data = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(160_000)
            .map(char::from)
            .collect::<String>();

        let srv = actix_test::start_with(actix_test::config().rustls_0_23(tls_config()), || {
            App::new().service(web::resource("/").route(web::to(|bytes: Bytes| async {
                // echo decompressed request body back in response
                HttpResponse::Ok()
                    .insert_header(header::ContentEncoding::Identity)
                    .body(bytes)
            })))
        });

        let req = srv
            .post("/")
            .insert_header((header::CONTENT_ENCODING, "deflate"))
            .send_stream(TestBody::new(
                Bytes::from(utils::deflate::encode(&data)),
                1024,
            ));

        let mut res = req.await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = res.body().await.unwrap();
        assert_eq!(bytes.len(), data.len());
        assert_eq!(bytes, Bytes::from(data));

        srv.stop().await;
    }
}

#[actix_rt::test]
async fn test_server_cookies() {
    use actix_web::http;

    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|| async {
            HttpResponse::Ok()
                .cookie(
                    Cookie::build("first", "first_value")
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

    {
        let first_cookie = Cookie::build("first", "first_value")
            .http_only(true)
            .finish();
        let second_cookie = Cookie::new("second", "first_value");

        let cookies = res.cookies().expect("To have cookies");
        assert_eq!(cookies.len(), 3);
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
        assert_eq!(cookies.len(), 3);
        if cookies[0] == first_cookie {
            assert_eq!(cookies[1], second_cookie);
        } else {
            assert_eq!(cookies[0], second_cookie);
            assert_eq!(cookies[1], first_cookie);
        }
    }

    srv.stop().await;
}

#[actix_rt::test]
async fn test_slow_request() {
    use std::net;

    let srv = actix_test::start_with(
        actix_test::config().client_request_timeout(Duration::from_millis(200)),
        || App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
    );

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

    srv.stop().await;
}

#[actix_rt::test]
async fn test_normalize() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .service(web::resource("/one").route(web::to(HttpResponse::Ok)))
    });

    let res = srv.get("/one/").send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    srv.stop().await
}

// allow deprecated App::data
#[allow(deprecated)]
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

#[actix_rt::test]
async fn test_accept_encoding_no_match() {
    let srv = actix_test::start_with(actix_test::config().h1(), || {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(HttpResponse::Ok)))
    });

    let mut res = srv
        .get("/")
        .insert_header((header::ACCEPT_ENCODING, "xz, identity;q=0"))
        .no_decompress()
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None);

    let bytes = res.body().await.unwrap();
    // body should contain the supported encodings
    assert!(!bytes.is_empty());

    srv.stop().await;
}
