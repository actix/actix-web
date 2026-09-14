use actix_http::ContentEncoding;
use actix_web::{
    http::{header, StatusCode},
    middleware::Compress,
    web, App, HttpResponse,
};
use bytes::Bytes;

mod utils;

static LOREM: &[u8] = include_bytes!("fixtures/lorem.txt");
static LOREM_GZIP: &[u8] = include_bytes!("fixtures/lorem.txt.gz");
static LOREM_BR: &[u8] = include_bytes!("fixtures/lorem.txt.br");
static LOREM_ZSTD: &[u8] = include_bytes!("fixtures/lorem.txt.zst");
static LOREM_XZ: &[u8] = include_bytes!("fixtures/lorem.txt.xz");

macro_rules! test_server {
    () => {
        actix_test::start(|| {
            App::new()
                .wrap(Compress::default())
                .route(
                    "/static",
                    web::to(|| async { HttpResponse::Ok().body(LOREM) }),
                )
                .route(
                    "/static-gzip",
                    web::to(|| async {
                        HttpResponse::Ok()
                            // signal to compressor that content should not be altered
                            // signal to client that content is encoded
                            .insert_header(ContentEncoding::Gzip)
                            .body(LOREM_GZIP)
                    }),
                )
                .route(
                    "/static-br",
                    web::to(|| async {
                        HttpResponse::Ok()
                            // signal to compressor that content should not be altered
                            // signal to client that content is encoded
                            .insert_header(ContentEncoding::Brotli)
                            .body(LOREM_BR)
                    }),
                )
                .route(
                    "/static-zstd",
                    web::to(|| async {
                        HttpResponse::Ok()
                            // signal to compressor that content should not be altered
                            // signal to client that content is encoded
                            .insert_header(ContentEncoding::Zstd)
                            .body(LOREM_ZSTD)
                    }),
                )
                .route(
                    "/static-xz",
                    web::to(|| async {
                        HttpResponse::Ok()
                            // signal to compressor that content should not be altered
                            // signal to client that content is encoded as 7zip
                            .insert_header((header::CONTENT_ENCODING, "xz"))
                            .body(LOREM_XZ)
                    }),
                )
                .route(
                    "/echo",
                    web::to(|body: Bytes| async move { HttpResponse::Ok().body(body) }),
                )
        })
    };
}

#[actix_rt::test]
async fn negotiate_encoding_identity() {
    let srv = test_server!();

    let req = srv
        .post("/static")
        .insert_header((header::ACCEPT_ENCODING, "identity"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    srv.stop().await;
}

#[actix_rt::test]
async fn negotiate_encoding_gzip() {
    let srv = test_server!();

    let req = srv
        .post("/static")
        .insert_header((header::ACCEPT_ENCODING, "gzip, br;q=0.8, zstd;q=0.5"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    let mut res = srv
        .post("/static")
        .no_decompress()
        .insert_header((header::ACCEPT_ENCODING, "gzip, br;q=0.8, zstd;q=0.5"))
        .send()
        .await
        .unwrap();
    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), LOREM);

    srv.stop().await;
}

#[actix_rt::test]
async fn negotiate_encoding_br() {
    let srv = test_server!();

    // check that brotli content-encoding header is returned

    let req = srv
        .post("/static")
        .insert_header((header::ACCEPT_ENCODING, "br, zstd, gzip"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "br");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    // check that brotli is preferred even when later in (q-less) list

    let req = srv
        .post("/static")
        .insert_header((header::ACCEPT_ENCODING, "gzip, zstd, br"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "br");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    // check that returned content is actually brotli encoded

    let mut res = srv
        .post("/static")
        .no_decompress()
        .insert_header((header::ACCEPT_ENCODING, "br, zstd, gzip"))
        .send()
        .await
        .unwrap();
    let bytes = res.body().await.unwrap();
    assert_eq!(utils::brotli::decode(bytes), LOREM);

    srv.stop().await;
}

#[actix_rt::test]
async fn negotiate_encoding_zstd() {
    let srv = test_server!();

    let req = srv
        .post("/static")
        .insert_header((header::ACCEPT_ENCODING, "zstd, gzip, br;q=0.8"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "zstd");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    let mut res = srv
        .post("/static")
        .no_decompress()
        .insert_header((header::ACCEPT_ENCODING, "zstd, gzip, br;q=0.8"))
        .send()
        .await
        .unwrap();
    let bytes = res.body().await.unwrap();
    assert_eq!(utils::zstd::decode(bytes), LOREM);

    srv.stop().await;
}

#[cfg(all(
    feature = "compress-brotli",
    feature = "compress-gzip",
    feature = "compress-zstd",
))]
#[actix_rt::test]
async fn client_encoding_prefers_brotli() {
    let srv = test_server!();

    let req = srv.post("/static").send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "br");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    srv.stop().await;
}

#[actix_rt::test]
async fn gzip_no_decompress() {
    let srv = test_server!();

    let req = srv
        .post("/static-gzip")
        // don't decompress response body
        .no_decompress()
        // signal that we want a compressed body
        .insert_header((header::ACCEPT_ENCODING, "gzip, br;q=0.8, zstd;q=0.5"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM_GZIP));

    srv.stop().await;
}

#[actix_rt::test]
async fn manual_custom_coding() {
    let srv = test_server!();

    let req = srv
        .post("/static-xz")
        // don't decompress response body
        .no_decompress()
        // signal that we want a compressed body
        .insert_header((header::ACCEPT_ENCODING, "xz"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "xz");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM_XZ));

    srv.stop().await;
}

#[actix_rt::test]
async fn deny_identity_coding() {
    let srv = test_server!();

    let req = srv
        .post("/static")
        // signal that we want a compressed body
        .insert_header((header::ACCEPT_ENCODING, "br, identity;q=0"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "br");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM));

    srv.stop().await;
}

#[actix_rt::test]
async fn deny_identity_coding_no_decompress() {
    let srv = test_server!();

    let req = srv
        .post("/static-br")
        // don't decompress response body
        .no_decompress()
        // signal that we want a compressed body
        .insert_header((header::ACCEPT_ENCODING, "br, identity;q=0"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "br");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM_BR));

    srv.stop().await;
}

// TODO: fix test
// currently fails because negotiation doesn't consider unknown encoding types
#[ignore]
#[actix_rt::test]
async fn deny_identity_for_manual_coding() {
    let srv = test_server!();

    let req = srv
        .post("/static-xz")
        // don't decompress response body
        .no_decompress()
        // signal that we want a compressed body
        .insert_header((header::ACCEPT_ENCODING, "xz, identity;q=0"))
        .send();

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "xz");

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(LOREM_XZ));

    srv.stop().await;
}

#[actix_rt::test]
async fn gzip_flushes_small_streaming_chunks() {
    // Regression: small chunks (SSE events, for example) produce no compressed
    // output on their own; the encoders buffer them internally. Without a
    // flush when a write yields no output, the whole body only reaches the
    // client when the stream ends.
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
    };

    use futures_util::{stream, StreamExt};

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    // The server factory must be Clone; only the (single) worker's handler
    // takes the receiver out of the shared slot.
    let rx = Arc::new(Mutex::new(Some(rx)));

    // The first two events are produced back to back; the third one is held
    // back behind the gate. The encoder's first write only emits the gzip
    // header, so streaming starts with the second write's flush.
    let srv = actix_test::start(move || {
        let rx = Arc::clone(&rx);
        App::new().wrap(Compress::default()).route(
            "/gated",
            web::to(move || {
                let rx = Arc::clone(&rx);
                async move {
                    let rx = rx.lock().unwrap().take();

                    let body = stream::unfold((rx, 0u8), |(rx, step)| async move {
                        match step {
                            0 => Some((
                                Ok::<_, Infallible>(Bytes::from_static(b"data: first\n\n")),
                                (rx, 1),
                            )),
                            1 => Some((
                                Ok::<_, Infallible>(Bytes::from_static(b"data: second\n\n")),
                                (rx, 2),
                            )),
                            2 => {
                                let _ = rx.unwrap().await;
                                Some((
                                    Ok::<_, Infallible>(Bytes::from_static(b"data: third\n\n")),
                                    (None, 3),
                                ))
                            }
                            _ => None,
                        }
                    });

                    HttpResponse::Ok()
                        .content_type("text/event-stream")
                        .streaming(Box::pin(body))
                }
            }),
        )
    });

    let mut res = srv
        .post("/gated")
        .no_decompress()
        .insert_header((header::ACCEPT_ENCODING, "gzip"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");

    // The gate has not been released yet. Keep reading with a timeout: with
    // the flush in place the first two events arrive as compressed data; a
    // buffering encoder only ever sends the bare 10-byte gzip header here.
    use std::io::Read as _;

    let mut raw = Vec::new();
    let first_events: &[u8] = b"data: first\n\ndata: second\n\n";
    let mut decompressed = Vec::new();

    loop {
        let chunk = actix_rt::time::timeout(std::time::Duration::from_secs(3), res.next())
            .await
            .expect("compressed data for the first events was not flushed before the stream ended")
            .unwrap()
            .unwrap();
        raw.extend_from_slice(&chunk);

        let mut decoder = flate2::read::GzDecoder::new(&raw[..]);
        let mut buf = [0u8; 128];
        loop {
            let n = decoder.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            decompressed.extend_from_slice(&buf[..n]);
            if decompressed.len() >= first_events.len() {
                break;
            }
        }

        if !decompressed.is_empty() {
            break;
        }
    }

    assert_eq!(
        &decompressed[..],
        &first_events[..],
        "first flushed chunk did not decompress to the first two events"
    );

    // Release the last event and collect the rest of the body.
    tx.send(()).unwrap();

    while let Some(chunk) = res.next().await {
        raw.extend_from_slice(&chunk.unwrap());
    }

    assert_eq!(
        utils::gzip::decode(raw),
        &b"data: first\n\ndata: second\n\ndata: third\n\n"[..]
    );

    srv.stop().await;
}
