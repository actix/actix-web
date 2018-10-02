extern crate actix;
extern crate actix_net;
extern crate actix_web;
#[cfg(feature = "brotli")]
extern crate brotli2;
extern crate bytes;
extern crate flate2;
extern crate futures;
extern crate h2;
extern crate http as modhttp;
extern crate rand;
extern crate tokio;
extern crate tokio_current_thread;
extern crate tokio_current_thread as current_thread;
extern crate tokio_reactor;
extern crate tokio_tcp;

#[cfg(feature = "ssl")]
extern crate openssl;

use std::io::{Read, Write};
use std::sync::Arc;
use std::{thread, time};

use actix_net::server::Server;
#[cfg(feature = "brotli")]
use brotli2::write::{BrotliDecoder, BrotliEncoder};
use bytes::{Bytes, BytesMut};
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, ZlibDecoder, ZlibEncoder};
use flate2::Compression;
use futures::stream::once;
use futures::{Future, Stream};
use h2::client as h2client;
use modhttp::Request;
use rand::distributions::Alphanumeric;
use rand::Rng;
use tokio::runtime::current_thread::Runtime;
use tokio_current_thread::spawn;
use tokio_tcp::TcpStream;

use actix_web::*;

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
#[cfg(unix)]
fn test_start() {
    use actix::System;
    use std::sync::mpsc;

    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(|| {
        System::run(move || {
            let srv = server::new(|| {
                vec![App::new().resource("/", |r| {
                    r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                })]
            });

            let srv = srv.bind("127.0.0.1:0").unwrap();
            let addr = srv.addrs()[0];
            let srv_addr = srv.start();
            let _ = tx.send((addr, srv_addr, System::current()));
        });
    });
    let (addr, srv_addr, sys) = rx.recv().unwrap();
    System::set_current(sys.clone());

    let mut rt = Runtime::new().unwrap();
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }

    // pause
    let _ = srv_addr.send(server::PauseServer).wait();
    thread::sleep(time::Duration::from_millis(200));
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .timeout(time::Duration::from_millis(200))
            .finish()
            .unwrap();
        assert!(rt.block_on(req.send()).is_err());
    }

    // resume
    let _ = srv_addr.send(server::ResumeServer).wait();
    thread::sleep(time::Duration::from_millis(200));
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }

    let _ = sys.stop();
}

#[test]
#[cfg(unix)]
fn test_shutdown() {
    use actix::System;
    use std::net;
    use std::sync::mpsc;

    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(|| {
        System::run(move || {
            let srv = server::new(|| {
                vec![App::new().resource("/", |r| {
                    r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                })]
            });

            let srv = srv.bind("127.0.0.1:0").unwrap();
            let addr = srv.addrs()[0];
            let srv_addr = srv.shutdown_timeout(1).start();
            let _ = tx.send((addr, srv_addr, System::current()));
        });
    });
    let (addr, srv_addr, sys) = rx.recv().unwrap();
    System::set_current(sys.clone());

    let mut rt = Runtime::new().unwrap();
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send()).unwrap();
        srv_addr.do_send(server::StopServer { graceful: true });
        assert!(response.status().is_success());
    }

    thread::sleep(time::Duration::from_millis(1000));
    assert!(net::TcpStream::connect(addr).is_err());

    let _ = sys.stop();
}

#[test]
#[cfg(unix)]
fn test_panic() {
    use actix::System;
    use std::sync::mpsc;

    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(|| {
        System::run(move || {
            let srv = server::new(|| {
                App::new()
                    .resource("/panic", |r| {
                        r.method(http::Method::GET).f(|_| -> &'static str {
                            panic!("error");
                        });
                    }).resource("/", |r| {
                        r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                    })
            }).workers(1);

            let srv = srv.bind("127.0.0.1:0").unwrap();
            let addr = srv.addrs()[0];
            srv.start();
            let _ = tx.send((addr, System::current()));
        });
    });
    let (addr, sys) = rx.recv().unwrap();
    System::set_current(sys.clone());

    let mut rt = Runtime::new().unwrap();
    {
        let req = client::ClientRequest::get(format!("http://{}/panic", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send());
        assert!(response.is_err());
    }

    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send());
        assert!(response.is_err());
    }
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = rt.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }

    let _ = sys.stop();
}

#[test]
fn test_simple() {
    let mut srv = test::TestServer::new(|app| app.handler(|_| HttpResponse::Ok()));
    let req = srv.get().finish().unwrap();
    let response = srv.execute(req.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_headers() {
    let data = STR.repeat(10);
    let srv_data = Arc::new(data.clone());
    let mut srv = test::TestServer::new(move |app| {
        let data = srv_data.clone();
        app.handler(move |_| {
            let mut builder = HttpResponse::Ok();
            for idx in 0..90 {
                builder.header(
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
            builder.body(data.as_ref())
        })
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_body() {
    let mut srv =
        test::TestServer::new(|app| app.handler(|_| HttpResponse::Ok().body(STR)));

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_gzip() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Gzip)
                .body(STR)
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_gzip_large() {
    let data = STR.repeat(10);
    let srv_data = Arc::new(data.clone());

    let mut srv = test::TestServer::new(move |app| {
        let data = srv_data.clone();
        app.handler(move |_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Gzip)
                .body(data.as_ref())
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from(data));
}

#[test]
fn test_body_gzip_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(70_000)
        .collect::<String>();
    let srv_data = Arc::new(data.clone());

    let mut srv = test::TestServer::new(move |app| {
        let data = srv_data.clone();
        app.handler(move |_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Gzip)
                .body(data.as_ref())
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(dec.len(), data.len());
    assert_eq!(Bytes::from(dec), Bytes::from(data));
}

#[test]
fn test_body_chunked_implicit() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[cfg(feature = "brotli")]
#[test]
fn test_body_br_streaming() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Br)
                .body(Body::Streaming(Box::new(body)))
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode br
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_head_empty() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| HttpResponse::Ok().content_length(STR.len() as u64).finish())
    });

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Identity)
                .content_length(100)
                .body(STR)
        })
    });

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_head_binary2() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Identity)
                .body(STR)
        })
    });

    let request = srv.head().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
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
fn test_body_length() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            HttpResponse::Ok()
                .content_length(STR.len() as u64)
                .content_encoding(http::ContentEncoding::Identity)
                .body(Body::Streaming(Box::new(body)))
        })
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_chunked_explicit() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            HttpResponse::Ok()
                .chunked()
                .content_encoding(http::ContentEncoding::Gzip)
                .body(Body::Streaming(Box::new(body)))
        })
    });

    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_body_identity() {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();
    let enc2 = enc.clone();

    let mut srv = test::TestServer::new(move |app| {
        let enc3 = enc2.clone();
        app.handler(move |_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Identity)
                .header(http::header::CONTENT_ENCODING, "deflate")
                .body(enc3.clone())
        })
    });

    // client request
    let request = srv
        .get()
        .header("accept-encoding", "deflate")
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode deflate
    assert_eq!(bytes, Bytes::from(STR));
}

#[test]
fn test_body_deflate() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Deflate)
                .body(STR)
        })
    });

    // client request
    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode deflate
    let mut e = ZlibDecoder::new(Vec::new());
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[cfg(feature = "brotli")]
#[test]
fn test_body_brotli() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|_| {
            HttpResponse::Ok()
                .content_encoding(http::ContentEncoding::Br)
                .body(STR)
        })
    });

    // client request
    let request = srv.get().disable_decompress().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();

    // decode brotli
    let mut e = BrotliDecoder::new(Vec::with_capacity(2048));
    e.write_all(bytes.as_ref()).unwrap();
    let dec = e.finish().unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_gzip_encoding() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "gzip")
        .body(enc.clone())
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_gzip_encoding_large() {
    let data = STR.repeat(10);
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "gzip")
        .body(enc.clone())
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_reading_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(60_000)
        .collect::<String>();

    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    // client request
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "gzip")
        .body(enc.clone())
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_reading_deflate_encoding() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "deflate")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_reading_deflate_encoding_large() {
    let data = STR.repeat(10);
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "deflate")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_reading_deflate_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .collect::<String>();

    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "deflate")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(feature = "brotli")]
#[test]
fn test_brotli_encoding() {
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(STR.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "br")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[cfg(feature = "brotli")]
#[test]
fn test_brotli_encoding_large() {
    let data = STR.repeat(10);
    let mut srv = test::TestServer::new(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "br")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(all(feature = "brotli", future = "ssl"))]
#[test]
fn test_ssl_brotli_encoding_large() {
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("tests/cert.pem")
        .unwrap();

    let data = STR.repeat(10);
    let mut srv = test::TestServer::build().ssl(builder).start(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = BrotliEncoder::new(Vec::new(), 5);
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "br")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[cfg(future = "rust-ssl")]
#[test]
fn test_reading_deflate_encoding_large_random_ssl() {
    use rustls::internal::pemfile::{certs, rsa_private_keys};
    use rustls::{NoClientAuth, ServerConfig};
    use std::fs::File;
    use std::io::BufReader;

    // load ssl keys
    let mut config = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("tests/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("tests/key.pem").unwrap());
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = rsa_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    let data = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(160_000)
        .collect::<String>();

    let mut srv = test::TestServer::build().rustls(config).start(|app| {
        app.handler(|req: &HttpRequest| {
            req.body()
                .and_then(|bytes: Bytes| {
                    Ok(HttpResponse::Ok()
                        .content_encoding(http::ContentEncoding::Identity)
                        .body(bytes))
                }).responder()
        })
    });

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data.as_ref()).unwrap();
    let enc = e.finish().unwrap();

    // client request
    let request = srv
        .post()
        .header(http::header::CONTENT_ENCODING, "deflate")
        .body(enc)
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_h2() {
    let srv = test::TestServer::new(|app| app.handler(|_| HttpResponse::Ok().body(STR)));
    let addr = srv.addr();
    thread::sleep(time::Duration::from_millis(500));

    let mut core = Runtime::new().unwrap();
    let tcp = TcpStream::connect(&addr);

    let tcp = tcp
        .then(|res| h2client::handshake(res.unwrap()))
        .then(move |res| {
            let (mut client, h2) = res.unwrap();

            let request = Request::builder()
                .uri(format!("https://{}/", addr).as_str())
                .body(())
                .unwrap();
            let (response, _) = client.send_request(request, false).unwrap();

            // Spawn a task to run the conn...
            spawn(h2.map_err(|e| println!("GOT ERR={:?}", e)));

            response.and_then(|response| {
                assert_eq!(response.status(), http::StatusCode::OK);

                let (_, body) = response.into_parts();

                body.fold(BytesMut::new(), |mut b, c| -> Result<_, h2::Error> {
                    b.extend(c);
                    Ok(b)
                })
            })
        });
    let _res = core.block_on(tcp);
    // assert_eq!(_res.unwrap(), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_application() {
    let mut srv = test::TestServer::with_factory(|| {
        App::new().resource("/", |r| r.f(|_| HttpResponse::Ok()))
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_default_404_handler_response() {
    let mut srv = test::TestServer::with_factory(|| {
        App::new()
            .prefix("/app")
            .resource("", |r| r.f(|_| HttpResponse::Ok()))
            .resource("/", |r| r.f(|_| HttpResponse::Ok()))
    });
    let addr = srv.addr();

    let mut buf = [0; 24];
    let request = TcpStream::connect(&addr)
        .and_then(|sock| {
            tokio::io::write_all(sock, "HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .and_then(|(sock, _)| tokio::io::read_exact(sock, &mut buf))
                .and_then(|(_, buf)| Ok(buf))
        }).map_err(|e| panic!("{:?}", e));
    let response = srv.execute(request).unwrap();
    let rep = String::from_utf8_lossy(&response[..]);
    assert!(rep.contains("HTTP/1.1 404 Not Found"));
}

#[test]
fn test_server_cookies() {
    use actix_web::http;

    let mut srv = test::TestServer::with_factory(|| {
        App::new().resource("/", |r| {
            r.f(|_| {
                HttpResponse::Ok()
                    .cookie(
                        http::CookieBuilder::new("first", "first_value")
                            .http_only(true)
                            .finish(),
                    ).cookie(http::Cookie::new("second", "first_value"))
                    .cookie(http::Cookie::new("second", "second_value"))
                    .finish()
            })
        })
    });

    let first_cookie = http::CookieBuilder::new("first", "first_value")
        .http_only(true)
        .finish();
    let second_cookie = http::Cookie::new("second", "second_value");

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    let cookies = response.cookies().expect("To have cookies");
    assert_eq!(cookies.len(), 2);
    if cookies[0] == first_cookie {
        assert_eq!(cookies[1], second_cookie);
    } else {
        assert_eq!(cookies[0], second_cookie);
        assert_eq!(cookies[1], first_cookie);
    }

    let first_cookie = first_cookie.to_string();
    let second_cookie = second_cookie.to_string();
    //Check that we have exactly two instances of raw cookie headers
    let cookies = response
        .headers()
        .get_all(http::header::SET_COOKIE)
        .iter()
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

#[test]
fn test_custom_pipeline() {
    use actix::System;
    use actix_net::service::NewServiceExt;
    use actix_web::server::{
        HttpService, KeepAlive, StreamConfiguration, WorkerSettings,
    };

    let addr = test::TestServer::unused_addr();

    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let app = App::new()
                    .route("/", http::Method::GET, |_: HttpRequest| "OK")
                    .finish();
                let settings = WorkerSettings::build(app)
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_shutdown(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish();

                StreamConfiguration::new()
                    .nodelay(true)
                    .tcp_keepalive(Some(time::Duration::from_secs(10)))
                    .and_then(HttpService::new(settings))
            }).unwrap()
            .run();
    });

    let mut sys = System::new("test");
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = sys.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }
}

#[test]
fn test_slow_request() {
    use actix::System;
    use std::net;
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();

    let addr = test::TestServer::unused_addr();

    thread::spawn(move || {
        System::run(move || {
            let srv = server::new(|| {
                vec![App::new().resource("/", |r| {
                    r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                })]
            });

            let srv = srv.bind(addr).unwrap();
            srv.client_timeout(200).start();
            let _ = tx.send(System::current());
        });
    });
    let sys = rx.recv().unwrap();

    thread::sleep(time::Duration::from_millis(200));

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));

    sys.stop();
}

#[test]
fn test_malformed_request() {
    use actix::System;
    use std::net;
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();

    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        System::run(move || {
            let srv = server::new(|| {
                App::new().resource("/", |r| {
                    r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                })
            });

            let _ = srv.bind(addr).unwrap().start();
            let _ = tx.send(System::current());
        });
    });
    let sys = rx.recv().unwrap();
    thread::sleep(time::Duration::from_millis(200));

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));

    sys.stop();
}

#[test]
fn test_app_404() {
    let mut srv = test::TestServer::with_factory(|| {
        App::new().prefix("/prefix").resource("/", |r| {
            r.method(http::Method::GET).f(|_| HttpResponse::Ok())
        })
    });

    let request = srv.client(http::Method::GET, "/prefix/").finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    let request = srv.client(http::Method::GET, "/").finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

#[test]
#[cfg(feature = "ssl")]
fn test_ssl_handshake_timeout() {
    use actix::System;
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
    use std::net;
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();
    let addr = test::TestServer::unused_addr();

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("tests/cert.pem")
        .unwrap();

    thread::spawn(move || {
        System::run(move || {
            let srv = server::new(|| {
                App::new().resource("/", |r| {
                    r.method(http::Method::GET).f(|_| HttpResponse::Ok())
                })
            });

            srv.bind_ssl(addr, builder)
                .unwrap()
                .workers(1)
                .client_timeout(200)
                .start();
            let _ = tx.send(System::current());
        });
    });
    let sys = rx.recv().unwrap();

    let mut stream = net::TcpStream::connect(addr).unwrap();
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.is_empty())
}
