use std::io::{Read, Write};

use actix_http::http::header::{
    ContentEncoding, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH,
    TRANSFER_ENCODING,
};
use brotli::{CompressorWriter, DecompressorWriter};
use bytes::Bytes;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, ZlibDecoder, ZlibEncoder};
use flate2::Compression;
use futures::{future::ok, stream::once};
use rand::{distributions::Alphanumeric, Rng};

use actix_web::dev::BodyEncoding;
use actix_web::middleware::Compress;
use actix_web::{dev, test, web, App, Error, HttpResponse};

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
async fn test_body() {
    let srv = test::start(|| {
        App::new()
            .service(web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))))
    });

    let mut response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_gzip() {
    let srv = test::start_with(test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .header(ACCEPT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::to(|| {
                HttpResponse::Ok().body(STR).into_body::<dev::Body>()
            })))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .header(ACCEPT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
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
        .header(ACCEPT_ENCODING, "deflate")
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
        .header(ACCEPT_ENCODING, "deflate")
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

    let srv = test::start_with(test::config().h1(), move || {
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
        .header(ACCEPT_ENCODING, "gzip")
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
        .collect::<String>();
    let srv_data = data.clone();

    let srv = test::start_with(test::config().h1(), move || {
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
        .header(ACCEPT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Gzip))
            .service(web::resource("/").route(web::get().to(move || {
                HttpResponse::Ok()
                    .streaming(once(ok::<_, Error>(Bytes::from_static(STR.as_ref()))))
            })))
    });

    let mut response = srv
        .get("/")
        .no_decompress()
        .header(ACCEPT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().wrap(Compress::new(ContentEncoding::Br)).service(
            web::resource("/").route(web::to(move || {
                HttpResponse::Ok()
                    .streaming(once(ok::<_, Error>(Bytes::from_static(STR.as_ref()))))
            })),
        )
    });

    let mut response = srv
        .get("/")
        .header(ACCEPT_ENCODING, "br")
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode br
    let mut e = DecompressorWriter::new(Vec::with_capacity(2048), 0);
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.into_inner().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_head_binary() {
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(web::resource("/").route(
            web::head().to(move || HttpResponse::Ok().content_length(100).body(STR)),
        ))
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(web::resource("/").route(web::to(move || {
            HttpResponse::Ok()
                .no_chunking()
                .content_length(STR.len() as u64)
                .streaming(once(ok::<_, Error>(Bytes::from_static(STR.as_ref()))))
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
    let srv = test::start_with(test::config().h1(), || {
        App::new()
            .wrap(Compress::new(ContentEncoding::Deflate))
            .service(
                web::resource("/").route(web::to(move || HttpResponse::Ok().body(STR))),
            )
    });

    // client request
    let mut response = srv
        .get("/")
        .header(ACCEPT_ENCODING, "deflate")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().wrap(Compress::new(ContentEncoding::Br)).service(
            web::resource("/").route(web::to(move || HttpResponse::Ok().body(STR))),
        )
    });

    // client request
    let mut response = srv
        .get("/")
        .header(ACCEPT_ENCODING, "br")
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();

    // decode brotli
    let mut e = DecompressorWriter::new(Vec::with_capacity(2048), 0);
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.into_inner().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_encoding() {
    let srv = test::start_with(test::config().h1(), || {
        App::new().wrap(Compress::default()).service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "gzip")
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_gzip_encoding() {
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "gzip")
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
        .collect::<String>();

    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "gzip")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "deflate")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "deflate")
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
        .collect::<String>();

    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "deflate")
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
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = CompressorWriter::new(Vec::new(), 0, 3, 0);
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.into_inner();

    // client request
    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "br")
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_brotli_encoding_large() {
    let data = STR.repeat(10);
    let srv = test::start_with(test::config().h1(), || {
        App::new().service(
            web::resource("/")
                .route(web::to(move |body: Bytes| HttpResponse::Ok().body(body))),
        )
    });

    let mut e = CompressorWriter::new(Vec::new(), 0, 3, 0);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.into_inner();

    // client request
    let request = srv
        .post("/")
        .header(CONTENT_ENCODING, "br")
        .send_body(enc.clone());
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(feature = "openssl")]
#[actix_rt::test]
async fn test_brotli_encoding_large_openssl() {
    // load ssl keys
    use open_ssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("tests/cert.pem")
        .unwrap();

    let data = STR.repeat(10);
    let srv = test::start_with(test::config().openssl(builder.build()), move || {
        App::new().service(web::resource("/").route(web::to(|bytes: Bytes| {
            HttpResponse::Ok()
                .encoding(actix_web::http::ContentEncoding::Identity)
                .body(bytes)
        })))
    });

    // body
    let mut e = CompressorWriter::new(Vec::new(), 0, 3, 0);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.into_inner();

    // client request
    let mut response = srv
        .post("/")
        .header(actix_web::http::header::CONTENT_ENCODING, "br")
        .send_body(enc)
        .await
        .unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(all(feature = "rustls", feature = "openssl"))]
#[actix_rt::test]
async fn test_reading_deflate_encoding_large_random_rustls() {
    use rust_tls::internal::pemfile::{certs, pkcs8_private_keys};
    use rust_tls::{NoClientAuth, ServerConfig};
    use std::fs::File;
    use std::io::BufReader;

    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .collect::<String>();

    // load ssl keys
    let mut config = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("tests/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("tests/key.pem").unwrap());
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    let srv = test::start_with(test::config().rustls(config), || {
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
        .header(actix_web::http::header::CONTENT_ENCODING, "deflate")
        .send_body(enc);

    let mut response = req.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

// #[cfg(all(feature = "tls", feature = "ssl"))]
// #[test]
// fn test_reading_deflate_encoding_large_random_nativetls() {
//     use native_tls::{Identity, TlsAcceptor};
//     use openssl::ssl::{
//         SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode,
//     };
//     use std::fs::File;
//     use std::sync::mpsc;

//     use actix::{Actor, System};
//     let (tx, rx) = mpsc::channel();

//     // load ssl keys
//     let mut file = File::open("tests/identity.pfx").unwrap();
//     let mut identity = vec![];
//     file.read_to_end(&mut identity).unwrap();
//     let identity = Identity::from_pkcs12(&identity, "1").unwrap();
//     let acceptor = TlsAcceptor::new(identity).unwrap();

//     // load ssl keys
//     let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
//     builder
//         .set_private_key_file("tests/key.pem", SslFiletype::PEM)
//         .unwrap();
//     builder
//         .set_certificate_chain_file("tests/cert.pem")
//         .unwrap();

//     let data = rand::thread_rng()
//         .sample_iter(&Alphanumeric)
//         .take(160_000)
//         .collect::<String>();

//     let addr = test::TestServer::unused_addr();
//     thread::spawn(move || {
//         System::run(move || {
//             server::new(|| {
//                 App::new().handler("/", |req: &HttpRequest| {
//                     req.body()
//                         .and_then(|bytes: Bytes| {
//                             Ok(HttpResponse::Ok()
//                                 .content_encoding(http::ContentEncoding::Identity)
//                                 .body(bytes))
//                         })
//                         .responder()
//                 })
//             })
//             .bind_tls(addr, acceptor)
//             .unwrap()
//             .start();
//             let _ = tx.send(System::current());
//         });
//     });
//     let sys = rx.recv().unwrap();

//     let mut rt = System::new("test");

//     // client connector
//     let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
//     builder.set_verify(SslVerifyMode::NONE);
//     let conn = client::ClientConnector::with_connector(builder.build()).start();

//     // encode data
//     let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
//     e.write_all(data.as_ref()).unwrap();
//     let enc = e.finish().unwrap();

//     // client request
//     let request = client::ClientRequest::build()
//         .uri(format!("https://{}/", addr))
//         .method(http::Method::POST)
//         .header(http::header::CONTENT_ENCODING, "deflate")
//         .with_connector(conn)
//         .body(enc)
//         .unwrap();
//     let response = rt.block_on(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = rt.block_on(response.body()).unwrap();
//     assert_eq!(bytes.len(), data.len());
//     assert_eq!(bytes, Bytes::from(data));

//     let _ = sys.stop();
// }

// #[test]
// fn test_server_cookies() {
//     use actix_web::http;

//     let srv = test::TestServer::with_factory(|| {
//         App::new().resource("/", |r| {
//             r.f(|_| {
//                 HttpResponse::Ok()
//                     .cookie(
//                         http::CookieBuilder::new("first", "first_value")
//                             .http_only(true)
//                             .finish(),
//                     )
//                     .cookie(http::Cookie::new("second", "first_value"))
//                     .cookie(http::Cookie::new("second", "second_value"))
//                     .finish()
//             })
//         })
//     });

//     let first_cookie = http::CookieBuilder::new("first", "first_value")
//         .http_only(true)
//         .finish();
//     let second_cookie = http::Cookie::new("second", "second_value");

//     let request = srv.get("/").finish().unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     let cookies = response.cookies().expect("To have cookies");
//     assert_eq!(cookies.len(), 2);
//     if cookies[0] == first_cookie {
//         assert_eq!(cookies[1], second_cookie);
//     } else {
//         assert_eq!(cookies[0], second_cookie);
//         assert_eq!(cookies[1], first_cookie);
//     }

//     let first_cookie = first_cookie.to_string();
//     let second_cookie = second_cookie.to_string();
//     //Check that we have exactly two instances of raw cookie headers
//     let cookies = response
//         .headers()
//         .get_all(http::header::SET_COOKIE)
//         .iter()
//         .map(|header| header.to_str().expect("To str").to_string())
//         .collect::<Vec<_>>();
//     assert_eq!(cookies.len(), 2);
//     if cookies[0] == first_cookie {
//         assert_eq!(cookies[1], second_cookie);
//     } else {
//         assert_eq!(cookies[0], second_cookie);
//         assert_eq!(cookies[1], first_cookie);
//     }
// }

#[actix_rt::test]
async fn test_slow_request() {
    use std::net;

    let srv = test::start_with(test::config().client_timeout(200), || {
        App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok())))
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

// #[cfg(feature = "openssl")]
// #[actix_rt::test]
// async fn test_ssl_handshake_timeout() {
//     use open_ssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
//     use std::net;

//     // load ssl keys
//     let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
//     builder
//         .set_private_key_file("tests/key.pem", SslFiletype::PEM)
//         .unwrap();
//     builder
//         .set_certificate_chain_file("tests/cert.pem")
//         .unwrap();

//     let srv = test::start_with(test::config().openssl(builder.build()), || {
//         App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok())))
//     });

//     let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
//     let mut data = String::new();
//     let _ = stream.read_to_string(&mut data);
//     assert!(data.is_empty());
// }
