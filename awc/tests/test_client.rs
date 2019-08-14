use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brotli2::write::BrotliEncoder;
use bytes::Bytes;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::Future;
use rand::Rng;

use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_service::{service_fn, NewService};
use actix_web::http::Cookie;
use actix_web::middleware::{BodyEncoding, Compress};
use actix_web::{http::header, web, App, Error, HttpMessage, HttpRequest, HttpResponse};
use awc::error::SendRequestError;

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
fn test_simple() {
    let mut srv =
        TestServer::new(|| {
            HttpService::new(App::new().service(
                web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))),
            ))
        });

    let request = srv.get("/").header("x-test", "111").send();
    let mut response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let mut response = srv.block_on(srv.post("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // camel case
    let response = srv.block_on(srv.post("/").camel_case().send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_json() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(
            web::resource("/").route(web::to(|_: web::Json<String>| HttpResponse::Ok())),
        ))
    });

    let request = srv
        .get("/")
        .header("x-test", "111")
        .send_json(&"TEST".to_string());
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_form() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to(
            |_: web::Form<HashMap<String, String>>| HttpResponse::Ok(),
        ))))
    });

    let mut data = HashMap::new();
    let _ = data.insert("key".to_string(), "TEST".to_string());

    let request = srv.get("/").header("x-test", "111").send_form(&data);
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_timeout() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to_async(
            || {
                tokio_timer::sleep(Duration::from_millis(200))
                    .then(|_| Ok::<_, Error>(HttpResponse::Ok().body(STR)))
            },
        ))))
    });

    let client = srv.execute(|| {
        awc::Client::build()
            .timeout(Duration::from_millis(50))
            .finish()
    });
    let request = client.get(srv.url("/")).send();
    match srv.block_on(request) {
        Err(SendRequestError::Timeout) => (),
        _ => panic!(),
    }
}

#[test]
fn test_timeout_override() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to_async(
            || {
                tokio_timer::sleep(Duration::from_millis(200))
                    .then(|_| Ok::<_, Error>(HttpResponse::Ok().body(STR)))
            },
        ))))
    });

    let client = awc::Client::build()
        .timeout(Duration::from_millis(50000))
        .finish();
    let request = client
        .get(srv.url("/"))
        .timeout(Duration::from_millis(50))
        .send();
    match srv.block_on(request) {
        Err(SendRequestError::Timeout) => (),
        _ => panic!(),
    }
}

#[test]
fn test_connection_reuse() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(HttpService::new(
            App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok()))),
        ))
    });

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/"));
    let response = srv.block_on_fn(move || req.send()).unwrap();
    assert!(response.status().is_success());

    // one connection
    assert_eq!(num.load(Ordering::Relaxed), 1);
}

#[test]
fn test_connection_force_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(HttpService::new(
            App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok()))),
        ))
    });

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).force_close().send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/")).force_close();
    let response = srv.block_on_fn(move || req.send()).unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[test]
fn test_connection_server_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(HttpService::new(
            App::new().service(
                web::resource("/")
                    .route(web::to(|| HttpResponse::Ok().force_close().finish())),
            ),
        ))
    });

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/"));
    let response = srv.block_on_fn(move || req.send()).unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[test]
fn test_connection_wait_queue() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(HttpService::new(App::new().service(
            web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))),
        )))
    });

    let client = awc::Client::build()
        .connector(awc::Connector::new().limit(1).finish())
        .finish();

    // req 1
    let request = client.get(srv.url("/")).send();
    let mut response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req2 = client.post(srv.url("/"));
    let req2_fut = srv.execute(move || {
        let mut fut = req2.send();
        assert!(fut.poll().unwrap().is_not_ready());
        fut
    });

    // read response 1
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // req 2
    let response = srv.block_on(req2_fut).unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 1);
}

#[test]
fn test_connection_wait_queue_force_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let mut srv = TestServer::new(move || {
        let num2 = num2.clone();
        service_fn(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            Ok(io)
        })
        .and_then(HttpService::new(
            App::new().service(
                web::resource("/")
                    .route(web::to(|| HttpResponse::Ok().force_close().body(STR))),
            ),
        ))
    });

    let client = awc::Client::build()
        .connector(awc::Connector::new().limit(1).finish())
        .finish();

    // req 1
    let request = client.get(srv.url("/")).send();
    let mut response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // req 2
    let req2 = client.post(srv.url("/"));
    let req2_fut = srv.execute(move || {
        let mut fut = req2.send();
        assert!(fut.poll().unwrap().is_not_ready());
        fut
    });

    // read response 1
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // req 2
    let response = srv.block_on(req2_fut).unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[test]
fn test_with_query_parameter() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").to(
            |req: HttpRequest| {
                if req.query_string().contains("qp") {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            },
        )))
    });

    let res = srv
        .block_on(awc::Client::new().get(srv.url("/?qp=5")).send())
        .unwrap();
    assert!(res.status().is_success());
}

#[test]
fn test_no_decompress() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().wrap(Compress::default()).service(
            web::resource("/").route(web::to(|| {
                let mut res = HttpResponse::Ok().body(STR);
                res.encoding(header::ContentEncoding::Gzip);
                res
            })),
        ))
    });

    let mut res = srv
        .block_on(awc::Client::new().get(srv.url("/")).no_decompress().send())
        .unwrap();
    assert!(res.status().is_success());

    // read response
    let bytes = srv.block_on(res.body()).unwrap();

    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));

    // POST
    let mut res = srv
        .block_on(awc::Client::new().post(srv.url("/")).no_decompress().send())
        .unwrap();
    assert!(res.status().is_success());

    let bytes = srv.block_on(res.body()).unwrap();
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_gzip_encoding() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to(|| {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(STR.as_ref()).unwrap();
            let data = e.finish().unwrap();

            HttpResponse::Ok()
                .header("content-encoding", "gzip")
                .body(data)
        }))))
    });

    // client request
    let mut response = srv.block_on(srv.post("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_client_gzip_encoding_large() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to(|| {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(STR.repeat(10).as_ref()).unwrap();
            let data = e.finish().unwrap();

            HttpResponse::Ok()
                .header("content-encoding", "gzip")
                .body(data)
        }))))
    });

    // client request
    let mut response = srv.block_on(srv.post("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(STR.repeat(10)));
}

#[test]
fn test_client_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(100_000)
        .collect::<String>();

    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to(
            |data: Bytes| {
                let mut e = GzEncoder::new(Vec::new(), Compression::default());
                e.write_all(&data).unwrap();
                let data = e.finish().unwrap();
                HttpResponse::Ok()
                    .header("content-encoding", "gzip")
                    .body(data)
            },
        ))))
    });

    // client request
    let mut response = srv.block_on(srv.post("/").send_body(data.clone())).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[test]
fn test_client_brotli_encoding() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().service(web::resource("/").route(web::to(
            |data: Bytes| {
                let mut e = BrotliEncoder::new(Vec::new(), 5);
                e.write_all(&data).unwrap();
                let data = e.finish().unwrap();
                HttpResponse::Ok()
                    .header("content-encoding", "br")
                    .body(data)
            },
        ))))
    });

    // client request
    let mut response = srv.block_on(srv.post("/").send_body(STR)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

// #[test]
// fn test_client_brotli_encoding_large_random() {
//     let data = rand::thread_rng()
//         .sample_iter(&rand::distributions::Alphanumeric)
//         .take(70_000)
//         .collect::<String>();

//     let mut srv = test::TestServer::new(|app| {
//         app.handler(|req: &HttpRequest| {
//             req.body()
//                 .and_then(move |bytes: Bytes| {
//                     Ok(HttpResponse::Ok()
//                         .content_encoding(http::ContentEncoding::Gzip)
//                         .body(bytes))
//                 })
//                 .responder()
//         })
//     });

//     // client request
//     let request = srv
//         .client(http::Method::POST, "/")
//         .content_encoding(http::ContentEncoding::Br)
//         .body(data.clone())
//         .unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = srv.execute(response.body()).unwrap();
//     assert_eq!(bytes.len(), data.len());
//     assert_eq!(bytes, Bytes::from(data));
// }

// #[cfg(feature = "brotli")]
// #[test]
// fn test_client_deflate_encoding() {
//     let mut srv = test::TestServer::new(|app| {
//         app.handler(|req: &HttpRequest| {
//             req.body()
//                 .and_then(|bytes: Bytes| {
//                     Ok(HttpResponse::Ok()
//                         .content_encoding(http::ContentEncoding::Br)
//                         .body(bytes))
//                 })
//                 .responder()
//         })
//     });

//     // client request
//     let request = srv
//         .post()
//         .content_encoding(http::ContentEncoding::Deflate)
//         .body(STR)
//         .unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = srv.execute(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
// }

// #[test]
// fn test_client_deflate_encoding_large_random() {
//     let data = rand::thread_rng()
//         .sample_iter(&rand::distributions::Alphanumeric)
//         .take(70_000)
//         .collect::<String>();

//     let mut srv = test::TestServer::new(|app| {
//         app.handler(|req: &HttpRequest| {
//             req.body()
//                 .and_then(|bytes: Bytes| {
//                     Ok(HttpResponse::Ok()
//                         .content_encoding(http::ContentEncoding::Br)
//                         .body(bytes))
//                 })
//                 .responder()
//         })
//     });

//     // client request
//     let request = srv
//         .post()
//         .content_encoding(http::ContentEncoding::Deflate)
//         .body(data.clone())
//         .unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = srv.execute(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from(data));
// }

// #[test]
// fn test_client_streaming_explicit() {
//     let mut srv = test::TestServer::new(|app| {
//         app.handler(|req: &HttpRequest| {
//             req.body()
//                 .map_err(Error::from)
//                 .and_then(|body| {
//                     Ok(HttpResponse::Ok()
//                         .chunked()
//                         .content_encoding(http::ContentEncoding::Identity)
//                         .body(body))
//                 })
//                 .responder()
//         })
//     });

//     let body = once(Ok(Bytes::from_static(STR.as_ref())));

//     let request = srv.get("/").body(Body::Streaming(Box::new(body))).unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = srv.execute(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
// }

// #[test]
// fn test_body_streaming_implicit() {
//     let mut srv = test::TestServer::new(|app| {
//         app.handler(|_| {
//             let body = once(Ok(Bytes::from_static(STR.as_ref())));
//             HttpResponse::Ok()
//                 .content_encoding(http::ContentEncoding::Gzip)
//                 .body(Body::Streaming(Box::new(body)))
//         })
//     });

//     let request = srv.get("/").finish().unwrap();
//     let response = srv.execute(request.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = srv.execute(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
// }

#[test]
fn test_client_cookie_handling() {
    fn err() -> Error {
        use std::io::{Error as IoError, ErrorKind};
        // stub some generic error
        Error::from(IoError::from(ErrorKind::NotFound))
    }
    let cookie1 = Cookie::build("cookie1", "value1").finish();
    let cookie2 = Cookie::build("cookie2", "value2")
        .domain("www.example.org")
        .path("/")
        .secure(true)
        .http_only(true)
        .finish();
    // Q: are all these clones really necessary? A: Yes, possibly
    let cookie1b = cookie1.clone();
    let cookie2b = cookie2.clone();

    let mut srv = TestServer::new(move || {
        let cookie1 = cookie1b.clone();
        let cookie2 = cookie2b.clone();

        HttpService::new(App::new().route(
            "/",
            web::to(move |req: HttpRequest| {
                // Check cookies were sent correctly
                req.cookie("cookie1")
                    .ok_or_else(err)
                    .and_then(|c1| {
                        if c1.value() == "value1" {
                            Ok(())
                        } else {
                            Err(err())
                        }
                    })
                    .and_then(|()| req.cookie("cookie2").ok_or_else(err))
                    .and_then(|c2| {
                        if c2.value() == "value2" {
                            Ok(())
                        } else {
                            Err(err())
                        }
                    })
                    // Send some cookies back
                    .map(|_| {
                        HttpResponse::Ok()
                            .cookie(cookie1.clone())
                            .cookie(cookie2.clone())
                            .finish()
                    })
            }),
        ))
    });

    let request = srv.get("/").cookie(cookie1.clone()).cookie(cookie2.clone());
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
    let c1 = response.cookie("cookie1").expect("Missing cookie1");
    assert_eq!(c1, cookie1);
    let c2 = response.cookie("cookie2").expect("Missing cookie2");
    assert_eq!(c2, cookie2);
}

// #[test]
// fn client_read_until_eof() {
//     let addr = test::TestServer::unused_addr();

//     thread::spawn(move || {
//         let lst = net::TcpListener::bind(addr).unwrap();

//         for stream in lst.incoming() {
//             let mut stream = stream.unwrap();
//             let mut b = [0; 1000];
//             let _ = stream.read(&mut b).unwrap();
//             let _ = stream
//                 .write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\nwelcome!");
//         }
//     });

//     let mut sys = actix::System::new("test");

//     // client request
//     let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
//         .finish()
//         .unwrap();
//     let response = sys.block_on(req.send()).unwrap();
//     assert!(response.status().is_success());

//     // read response
//     let bytes = sys.block_on(response.body()).unwrap();
//     assert_eq!(bytes, Bytes::from_static(b"welcome!"));
// }

#[test]
fn client_basic_auth() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().route(
            "/",
            web::to(|req: HttpRequest| {
                if req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    == "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
                {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            }),
        ))
    });

    // set authorization header to Basic <base64 encoded username:password>
    let request = srv.get("/").basic_auth("username", Some("password"));
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn client_bearer_auth() {
    let mut srv = TestServer::new(|| {
        HttpService::new(App::new().route(
            "/",
            web::to(|req: HttpRequest| {
                if req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    == "Bearer someS3cr3tAutht0k3n"
                {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            }),
        ))
    });

    // set authorization header to Bearer <token>
    let request = srv.get("/").bearer_auth("someS3cr3tAutht0k3n");
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
}
