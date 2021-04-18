use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use actix_utils::future::ok;
use brotli2::write::BrotliEncoder;
use bytes::Bytes;
use cookie::Cookie;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::stream;
use rand::Rng;

use actix_http::{
    http::{self, StatusCode},
    HttpService,
};
use actix_http_test::test_server;
use actix_service::{fn_service, map_config, ServiceFactoryExt as _};
use actix_web::{
    dev::{AppConfig, BodyEncoding},
    http::header,
    middleware::Compress,
    web, App, Error, HttpRequest, HttpResponse,
};
use awc::error::{JsonPayloadError, PayloadError, SendRequestError};

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
async fn test_simple() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))))
    });

    let request = srv.get("/").insert_header(("x-test", "111")).send();
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // camel case
    let response = srv.post("/").camel_case().send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_json() {
    let srv = actix_test::start(|| {
        App::new().service(
            web::resource("/").route(web::to(|_: web::Json<String>| HttpResponse::Ok())),
        )
    });

    let request = srv
        .get("/")
        .insert_header(("x-test", "111"))
        .send_json(&"TEST".to_string());
    let response = request.await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_form() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(
            |_: web::Form<HashMap<String, String>>| HttpResponse::Ok(),
        )))
    });

    let mut data = HashMap::new();
    let _ = data.insert("key".to_string(), "TEST".to_string());

    let request = srv
        .get("/")
        .append_header(("x-test", "111"))
        .send_form(&data);
    let response = request.await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_timeout() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            actix_rt::time::sleep(Duration::from_millis(200)).await;
            Ok::<_, Error>(HttpResponse::Ok().body(STR))
        })))
    });

    let connector = awc::Connector::new()
        .connector(actix_tls::connect::default_connector())
        .timeout(Duration::from_secs(15));

    let client = awc::Client::builder()
        .connector(connector)
        .timeout(Duration::from_millis(50))
        .finish();

    let request = client.get(srv.url("/")).send();
    match request.await {
        Err(SendRequestError::Timeout) => {}
        _ => panic!(),
    }
}

#[actix_rt::test]
async fn test_timeout_override() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            actix_rt::time::sleep(Duration::from_millis(200)).await;
            Ok::<_, Error>(HttpResponse::Ok().body(STR))
        })))
    });

    let client = awc::Client::builder()
        .timeout(Duration::from_millis(50000))
        .finish();
    let request = client
        .get(srv.url("/"))
        .timeout(Duration::from_millis(50))
        .send();
    match request.await {
        Err(SendRequestError::Timeout) => {}
        _ => panic!(),
    }
}

#[actix_rt::test]
async fn test_response_timeout() {
    use futures_util::stream::{once, StreamExt as _};

    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            Ok::<_, Error>(
                HttpResponse::Ok()
                    .content_type("application/json")
                    .streaming(Box::pin(once(async {
                        actix_rt::time::sleep(Duration::from_millis(200)).await;
                        Ok::<_, Error>(Bytes::from(STR))
                    }))),
            )
        })))
    });

    let client = awc::Client::new();

    let res = client
        .get(srv.url("/"))
        .send()
        .await
        .unwrap()
        .timeout(Duration::from_millis(500))
        .body()
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(res.as_ref()).unwrap(), STR);

    let res = client
        .get(srv.url("/"))
        .send()
        .await
        .unwrap()
        .timeout(Duration::from_millis(100))
        .next()
        .await
        .unwrap();
    match res {
        Err(PayloadError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
        _ => panic!("Response error type is not matched"),
    }

    let res = client
        .get(srv.url("/"))
        .send()
        .await
        .unwrap()
        .timeout(Duration::from_millis(100))
        .body()
        .await;
    match res {
        Err(PayloadError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
        _ => panic!("Response error type is not matched"),
    }

    let res = client
        .get(srv.url("/"))
        .send()
        .await
        .unwrap()
        .timeout(Duration::from_millis(100))
        .json::<HashMap<String, String>>()
        .await;
    match res {
        Err(JsonPayloadError::Payload(PayloadError::Io(e))) => {
            assert_eq!(e.kind(), std::io::ErrorKind::TimedOut)
        }
        _ => panic!("Response error type is not matched"),
    }
}

#[actix_rt::test]
async fn test_connection_reuse() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::new(map_config(
                App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                |_| AppConfig::default(),
            ))
            .tcp(),
        )
    })
    .await;

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/"));
    let response = req.send().await.unwrap();
    assert!(response.status().is_success());

    // one connection
    assert_eq!(num.load(Ordering::Relaxed), 1);
}

#[actix_rt::test]
async fn test_connection_force_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::new(map_config(
                App::new().service(web::resource("/").route(web::to(HttpResponse::Ok))),
                |_| AppConfig::default(),
            ))
            .tcp(),
        )
    })
    .await;

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).force_close().send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/")).force_close();
    let response = req.send().await.unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[actix_rt::test]
async fn test_connection_server_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::new(map_config(
                App::new().service(
                    web::resource("/")
                        .route(web::to(|| HttpResponse::Ok().force_close().finish())),
                ),
                |_| AppConfig::default(),
            ))
            .tcp(),
        )
    })
    .await;

    let client = awc::Client::default();

    // req 1
    let request = client.get(srv.url("/")).send();
    let response = request.await.unwrap();
    assert!(response.status().is_success());

    // req 2
    let req = client.post(srv.url("/"));
    let response = req.send().await.unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[actix_rt::test]
async fn test_connection_wait_queue() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::new(map_config(
                App::new().service(
                    web::resource("/").route(web::to(|| HttpResponse::Ok().body(STR))),
                ),
                |_| AppConfig::default(),
            ))
            .tcp(),
        )
    })
    .await;

    let client = awc::Client::builder()
        .connector(awc::Connector::new().limit(1))
        .finish();

    // req 1
    let request = client.get(srv.url("/")).send();
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // req 2
    let req2 = client.post(srv.url("/"));
    let req2_fut = req2.send();

    // read response 1
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // req 2
    let response = req2_fut.await.unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 1);
}

#[actix_rt::test]
async fn test_connection_wait_queue_force_close() {
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let srv = test_server(move || {
        let num2 = num2.clone();
        fn_service(move |io| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok(io)
        })
        .and_then(
            HttpService::new(map_config(
                App::new().service(
                    web::resource("/")
                        .route(web::to(|| HttpResponse::Ok().force_close().body(STR))),
                ),
                |_| AppConfig::default(),
            ))
            .tcp(),
        )
    })
    .await;

    let client = awc::Client::builder()
        .connector(awc::Connector::new().limit(1))
        .finish();

    // req 1
    let request = client.get(srv.url("/")).send();
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // req 2
    let req2 = client.post(srv.url("/"));
    let req2_fut = req2.send();

    // read response 1
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    // req 2
    let response = req2_fut.await.unwrap();
    assert!(response.status().is_success());

    // two connection
    assert_eq!(num.load(Ordering::Relaxed), 2);
}

#[actix_rt::test]
async fn test_with_query_parameter() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(|req: HttpRequest| {
            if req.query_string().contains("qp") {
                HttpResponse::Ok()
            } else {
                HttpResponse::BadRequest()
            }
        }))
    });

    let res = awc::Client::new()
        .get(srv.url("/?qp=5"))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
}

#[actix_rt::test]
async fn test_no_decompress() {
    let srv = actix_test::start(|| {
        App::new()
            .wrap(Compress::default())
            .service(web::resource("/").route(web::to(|| {
                let mut res = HttpResponse::Ok().body(STR);
                res.encoding(header::ContentEncoding::Gzip);
                res
            })))
    });

    let mut res = awc::Client::new()
        .get(srv.url("/"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    // read response
    let bytes = res.body().await.unwrap();

    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));

    // POST
    let mut res = awc::Client::new()
        .post(srv.url("/"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    let mut e = GzDecoder::new(&bytes[..]);
    let mut dec = Vec::new();
    e.read_to_end(&mut dec).unwrap();
    assert_eq!(Bytes::from(dec), Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_client_gzip_encoding() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(STR.as_ref()).unwrap();
            let data = e.finish().unwrap();

            HttpResponse::Ok()
                .insert_header(("content-encoding", "gzip"))
                .body(data)
        })))
    });

    // client request
    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_client_gzip_encoding_large() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(STR.repeat(10).as_ref()).unwrap();
            let data = e.finish().unwrap();

            HttpResponse::Ok()
                .insert_header(("content-encoding", "gzip"))
                .body(data)
        })))
    });

    // client request
    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(STR.repeat(10)));
}

#[actix_rt::test]
async fn test_client_gzip_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(100_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(&data).unwrap();
            let data = e.finish().unwrap();
            HttpResponse::Ok()
                .insert_header(("content-encoding", "gzip"))
                .body(data)
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(data.clone()).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_client_brotli_encoding() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| {
            let mut e = BrotliEncoder::new(Vec::new(), 5);
            e.write_all(&data).unwrap();
            let data = e.finish().unwrap();
            HttpResponse::Ok()
                .insert_header(("content-encoding", "br"))
                .body(data)
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(STR).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_client_brotli_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(70_000)
        .map(char::from)
        .collect::<String>();

    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| {
            let mut e = BrotliEncoder::new(Vec::new(), 5);
            e.write_all(&data).unwrap();
            let data = e.finish().unwrap();
            HttpResponse::Ok()
                .insert_header(("content-encoding", "br"))
                .body(data)
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(data.clone()).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes.len(), data.len());
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_client_deflate_encoding() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: Bytes| {
            HttpResponse::Ok()
                .encoding(http::ContentEncoding::Br)
                .body(body)
        }))
    });

    let req = srv.post("/").send_body(STR);

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_client_deflate_encoding_large_random() {
    let data = rand::thread_rng()
        .sample_iter(rand::distributions::Alphanumeric)
        .map(char::from)
        .take(70_000)
        .collect::<String>();

    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: Bytes| {
            HttpResponse::Ok()
                .encoding(http::ContentEncoding::Br)
                .body(body)
        }))
    });

    let req = srv.post("/").send_body(data.clone());

    let mut res = req.await.unwrap();
    let bytes = res.body().await.unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn test_client_streaming_explicit() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: web::Payload| {
            HttpResponse::Ok()
                .encoding(http::ContentEncoding::Identity)
                .streaming(body)
        }))
    });

    let body =
        stream::once(async { Ok::<_, actix_http::Error>(Bytes::from_static(STR.as_bytes())) });
    let req = srv.post("/").send_stream(Box::pin(body));

    let mut res = req.await.unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_body_streaming_implicit() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|| {
            let body = stream::once(async {
                Ok::<_, actix_http::Error>(Bytes::from_static(STR.as_bytes()))
            });

            HttpResponse::Ok()
                .encoding(http::ContentEncoding::Gzip)
                .streaming(Box::pin(body))
        }))
    });

    let req = srv.get("/").send();

    let mut res = req.await.unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_client_cookie_handling() {
    use std::io::{Error as IoError, ErrorKind};

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

    let srv = actix_test::start(move || {
        let cookie1 = cookie1b.clone();
        let cookie2 = cookie2b.clone();

        App::new().route(
            "/",
            web::to(move |req: HttpRequest| {
                let cookie1 = cookie1.clone();
                let cookie2 = cookie2.clone();

                async move {
                    // Check cookies were sent correctly
                    let res: Result<(), Error> = req
                        .cookie("cookie1")
                        .ok_or(())
                        .and_then(|c1| {
                            if c1.value() == "value1" {
                                Ok(())
                            } else {
                                Err(())
                            }
                        })
                        .and_then(|()| req.cookie("cookie2").ok_or(()))
                        .and_then(|c2| {
                            if c2.value() == "value2" {
                                Ok(())
                            } else {
                                Err(())
                            }
                        })
                        .map_err(|_| Error::from(IoError::from(ErrorKind::NotFound)));

                    if let Err(e) = res {
                        Err(e)
                    } else {
                        // Send some cookies back
                        Ok::<_, Error>(
                            HttpResponse::Ok().cookie(cookie1).cookie(cookie2).finish(),
                        )
                    }
                }
            }),
        )
    });

    let request = srv.get("/").cookie(cookie1.clone()).cookie(cookie2.clone());
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
    let c1 = response.cookie("cookie1").expect("Missing cookie1");
    assert_eq!(c1, cookie1);
    let c2 = response.cookie("cookie2").expect("Missing cookie2");
    assert_eq!(c2, cookie2);
}

#[actix_rt::test]
async fn client_unread_response() {
    let addr = actix_test::unused_addr();
    let lst = std::net::TcpListener::bind(addr).unwrap();

    std::thread::spawn(move || {
        for stream in lst.incoming() {
            let mut stream = stream.unwrap();
            let mut b = [0; 1000];
            let _ = stream.read(&mut b).unwrap();
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\n\
                connection: close\r\n\
                \r\n\
                welcome!",
            );
        }
    });

    // client request
    let req = awc::Client::new().get(format!("http://{}/", addr).as_str());
    let mut res = req.send().await.unwrap();
    assert!(res.status().is_success());

    // awc does not read all bytes unless content-length is specified
    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b""));
}

#[actix_rt::test]
async fn client_basic_auth() {
    let srv = actix_test::start(|| {
        App::new().route(
            "/",
            web::to(|req: HttpRequest| {
                if req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    == format!("Basic {}", base64::encode("username:password"))
                {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            }),
        )
    });

    // set authorization header to Basic <base64 encoded username:password>
    let request = srv.get("/").basic_auth("username", "password");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn client_bearer_auth() {
    let srv = actix_test::start(|| {
        App::new().route(
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
        )
    });

    // set authorization header to Bearer <token>
    let request = srv.get("/").bearer_auth("someS3cr3tAutht0k3n");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_local_address() {
    let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

    let srv = actix_test::start(move || {
        App::new().service(web::resource("/").route(web::to(
            move |req: HttpRequest| async move {
                assert_eq!(req.peer_addr().unwrap().ip(), ip);
                Ok::<_, Error>(HttpResponse::Ok())
            },
        )))
    });
    let client = awc::Client::builder().local_address(ip).finish();

    let res = client.get(srv.url("/")).send().await.unwrap();

    assert_eq!(res.status(), 200);

    let client = awc::Client::builder()
        .connector(
            // connector local address setting should always be override by client builder.
            awc::Connector::new().local_address(IpAddr::V4(Ipv4Addr::new(128, 0, 0, 1))),
        )
        .local_address(ip)
        .finish();

    let res = client.get(srv.url("/")).send().await.unwrap();

    assert_eq!(res.status(), 200);
}
