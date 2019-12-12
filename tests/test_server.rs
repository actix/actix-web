use std::io::{Read, Write};

use actix_http::http::header::{
    ContentEncoding, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH,
    TRANSFER_ENCODING,
};
use actix_http::{Error, HttpService, Response};
use actix_http_test::TestServer;
use brotli::DecompressorWriter;
use bytes::Bytes;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, ZlibDecoder, ZlibEncoder};
use flate2::Compression;
use futures::{future::ok, stream::once};
use rand::{distributions::Alphanumeric, Rng};

use actix_web::middleware::{BodyEncoding, Compress};
use actix_web::{dev, http, web, App, HttpResponse, HttpServer};

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
    let srv = TestServer::start(|| {
        HttpService::build()
            .h1(App::new()
                .service(web::resource("/").route(web::to(|| Response::Ok().body(STR)))))
            .tcp()
    });

    let mut response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_gzip() {
    let srv = TestServer::start(|| {
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(web::resource("/").route(web::to(|| Response::Ok().body(STR)))))
            .tcp()
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
    let srv = TestServer::start(|| {
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(web::resource("/").route(web::to(|| {
                    Response::Ok().body(STR).into_body::<dev::Body>()
                }))))
            .tcp()
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
    let srv = TestServer::start(|| {
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(web::resource("/").route(web::to(|| {
                    Response::Ok().encoding(ContentEncoding::Deflate).body(STR)
                })))
                .service(web::resource("/raw").route(web::to(|| {
                    let body = actix_web::dev::Body::Bytes(STR.into());
                    let mut response =
                        Response::with_body(actix_web::http::StatusCode::OK, body);

                    response.encoding(ContentEncoding::Deflate);

                    response
                }))))
            .tcp()
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

    let srv = TestServer::start(move || {
        let data = srv_data.clone();
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(
                    web::resource("/")
                        .route(web::to(move || Response::Ok().body(data.clone()))),
                ))
            .tcp()
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

    let srv = TestServer::start(move || {
        let data = srv_data.clone();
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(
                    web::resource("/")
                        .route(web::to(move || Response::Ok().body(data.clone()))),
                ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Gzip))
                .service(web::resource("/").route(web::get().to(move || {
                    Response::Ok().streaming(once(ok::<_, Error>(Bytes::from_static(
                        STR.as_ref(),
                    ))))
                }))))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().wrap(Compress::new(ContentEncoding::Br)).service(
                web::resource("/").route(web::to(move || {
                    Response::Ok().streaming(once(ok::<_, Error>(Bytes::from_static(
                        STR.as_ref(),
                    ))))
                })),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(web::resource("/").route(
                web::head().to(move || Response::Ok().content_length(100).body(STR)),
            )))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(
                App::new().service(web::resource("/").route(web::to(move || {
                    Response::Ok()
                        .no_chunking()
                        .content_length(STR.len() as u64)
                        .streaming(once(ok::<_, Error>(Bytes::from_static(
                            STR.as_ref(),
                        ))))
                }))),
            )
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new()
                .wrap(Compress::new(ContentEncoding::Deflate))
                .service(
                    web::resource("/").route(web::to(move || Response::Ok().body(STR))),
                ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().wrap(Compress::new(ContentEncoding::Br)).service(
                web::resource("/").route(web::to(move || Response::Ok().body(STR))),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().wrap(Compress::default()).service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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

    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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

    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
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
#[cfg(feature = "brotli")]
async fn test_brotli_encoding() {
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

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

#[cfg(feature = "brotli")]
#[actix_rt::test]
async fn test_brotli_encoding_large() {
    let data = STR.repeat(10);
    let srv = TestServer::start(move || {
        HttpService::build()
            .h1(App::new().service(
                web::resource("/")
                    .route(web::to(move |body: Bytes| Response::Ok().body(body))),
            ))
            .tcp()
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

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

// #[cfg(feature = "ssl")]
// #[actix_rt::test]
// async fn test_brotli_encoding_large_ssl() {
//     use actix::{Actor, System};
//     use openssl::ssl::{
//         SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode,
//     };
//     // load ssl keys
//     let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
//     builder
//         .set_private_key_file("tests/key.pem", SslFiletype::PEM)
//         .unwrap();
//     builder
//         .set_certificate_chain_file("tests/cert.pem")
//         .unwrap();

//     let data = STR.repeat(10);
//     let srv = test::TestServer::build().ssl(builder).start(|app| {
//         app.handler(|req: &HttpRequest| {
//             req.body()
//                 .and_then(|bytes: Bytes| {
//                     Ok(HttpResponse::Ok()
//                         .content_encoding(http::ContentEncoding::Identity)
//                         .body(bytes))
//                 })
//                 .responder()
//         })
//     });
//     let mut rt = System::new("test");

//     // client connector
//     let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
//     builder.set_verify(SslVerifyMode::NONE);
//     let conn = client::ClientConnector::with_connector(builder.build()).start();

//     // body
//     let mut e = BrotliEncoder::new(Vec::new(), 5);
//     e.write_all(data.as_ref()).unwrap();
//     let enc = e.finish().unwrap();

//     // client request
//     let request = client::ClientRequest::build()
//         .uri(srv.url("/"))
//         .method(http::Method::POST)
//         .header(http::header::CONTENT_ENCODING, "br")
//         .with_connector(conn)
//         .body(enc)
//         .unwrap();
//     let response = rt.block_on(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = rt.block_on(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from(data));
// }

#[cfg(all(feature = "rustls", feature = "openssl"))]
#[actix_rt::test]
async fn test_reading_deflate_encoding_large_random_ssl() {
    use open_ssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
    use rust_tls::internal::pemfile::{certs, pkcs8_private_keys};
    use rust_tls::{NoClientAuth, ServerConfig};
    use std::fs::File;
    use std::io::BufReader;
    use std::sync::mpsc;

    let addr = TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .collect::<String>();

    std::thread::spawn(move || {
        let sys = actix_rt::System::new("test");

        // load ssl keys
        let mut config = ServerConfig::new(NoClientAuth::new());
        let cert_file = &mut BufReader::new(File::open("tests/cert.pem").unwrap());
        let key_file = &mut BufReader::new(File::open("tests/key.pem").unwrap());
        let cert_chain = certs(cert_file).unwrap();
        let mut keys = pkcs8_private_keys(key_file).unwrap();
        config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

        let srv = HttpServer::new(|| {
            App::new().service(web::resource("/").route(web::to(|bytes: Bytes| {
                async move {
                    Ok::<_, Error>(
                        HttpResponse::Ok()
                            .encoding(http::ContentEncoding::Identity)
                            .body(bytes),
                    )
                }
            })))
        })
        .bind_rustls(addr, config)
        .unwrap()
        .start();

        let _ = tx.send((srv, actix_rt::System::current()));
        let _ = sys.run();
    });
    let (srv, _sys) = rx.recv().unwrap();
    let client = {
        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::NONE);
        let _ = builder.set_alpn_protos(b"\x02h2\x08http/1.1").unwrap();

        awc::Client::build()
            .connector(
                awc::Connector::new()
                    .timeout(std::time::Duration::from_millis(500))
                    .ssl(builder.build())
                    .finish(),
            )
            .finish()
    };

    // encode data
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let req = client
        .post(format!("https://localhost:{}/", addr.port()))
        .header(http::header::CONTENT_ENCODING, "deflate")
        .send_body(enc);

    let mut response = req.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));

    // stop
    let _ = srv.stop(false);
}

// #[cfg(all(feature = "tls", feature = "ssl"))]
// #[test]
// fn test_reading_deflate_encoding_large_random_tls() {
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

// #[test]
// fn test_slow_request() {
//     use actix::System;
//     use std::net;
//     use std::sync::mpsc;
//     let (tx, rx) = mpsc::channel();

//     let addr = test::TestServer::unused_addr();
//     thread::spawn(move || {
//         System::run(move || {
//             let srv = server::new(|| {
//                 vec![App::new().resource("/", |r| {
//                     r.method(http::Method::GET).f(|_| HttpResponse::Ok())
//                 })]
//             });

//             let srv = srv.bind(addr).unwrap();
//             srv.client_timeout(200).start();
//             let _ = tx.send(System::current());
//         });
//     });
//     let sys = rx.recv().unwrap();

//     thread::sleep(time::Duration::from_millis(200));

//     let mut stream = net::TcpStream::connect(addr).unwrap();
//     let mut data = String::new();
//     let _ = stream.read_to_string(&mut data);
//     assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

//     let mut stream = net::TcpStream::connect(addr).unwrap();
//     let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
//     let mut data = String::new();
//     let _ = stream.read_to_string(&mut data);
//     assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

//     sys.stop();
// }

// #[test]
// #[cfg(feature = "ssl")]
// fn test_ssl_handshake_timeout() {
//     use actix::System;
//     use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
//     use std::net;
//     use std::sync::mpsc;

//     let (tx, rx) = mpsc::channel();
//     let addr = test::TestServer::unused_addr();

//     // load ssl keys
//     let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
//     builder
//         .set_private_key_file("tests/key.pem", SslFiletype::PEM)
//         .unwrap();
//     builder
//         .set_certificate_chain_file("tests/cert.pem")
//         .unwrap();

//     thread::spawn(move || {
//         System::run(move || {
//             let srv = server::new(|| {
//                 App::new().resource("/", |r| {
//                     r.method(http::Method::GET).f(|_| HttpResponse::Ok())
//                 })
//             });

//             srv.bind_ssl(addr, builder)
//                 .unwrap()
//                 .workers(1)
//                 .client_timeout(200)
//                 .start();
//             let _ = tx.send(System::current());
//         });
//     });
//     let sys = rx.recv().unwrap();

//     let mut stream = net::TcpStream::connect(addr).unwrap();
//     let mut data = String::new();
//     let _ = stream.read_to_string(&mut data);
//     assert!(data.is_empty());

//     let _ = sys.stop();
// }
