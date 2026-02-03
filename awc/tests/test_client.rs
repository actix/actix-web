use std::{
    collections::HashMap,
    convert::Infallible,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use actix_http::{HttpService, StatusCode};
use actix_http_test::test_server;
use actix_service::{fn_service, map_config, ServiceFactoryExt as _};
use actix_utils::future::ok;
use actix_web::{dev::AppConfig, http::header, web, App, Error, HttpRequest, HttpResponse};
use awc::error::{JsonPayloadError, PayloadError, SendRequestError};
use base64::prelude::*;
use bytes::Bytes;
use cookie::Cookie;
use futures_util::stream;
use rand::distr::{Alphanumeric, SampleString as _};

mod utils;

const S: &str = "Hello World ";
const STR: &str = const_str::repeat!(S, 100);

#[actix_rt::test]
async fn simple() {
    let srv = actix_test::start(|| {
        App::new()
            .service(web::resource("/").route(web::to(|| async { HttpResponse::Ok().body(STR) })))
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
async fn json() {
    let srv = actix_test::start(|| {
        App::new()
            .service(web::resource("/").route(web::to(|_: web::Json<String>| HttpResponse::Ok())))
    });

    let request = srv
        .get("/")
        .insert_header(("x-test", "111"))
        .send_json(&"TEST".to_string());
    let response = request.await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn form() {
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
async fn timeout() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            actix_rt::time::sleep(Duration::from_millis(200)).await;
            HttpResponse::Ok().body(STR)
        })))
    });

    let connector = awc::Connector::new()
        .connector(actix_tls::connect::ConnectorService::default())
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
async fn timeout_override() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            actix_rt::time::sleep(Duration::from_millis(200)).await;
            HttpResponse::Ok().body(STR)
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
async fn response_timeout() {
    use futures_util::{stream::once, StreamExt as _};

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
async fn connection_reuse() {
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
async fn connection_force_close() {
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
async fn connection_server_close() {
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
                App::new().service(web::resource("/").route(web::to(|| async {
                    HttpResponse::Ok().force_close().finish()
                }))),
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
async fn connection_wait_queue() {
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
                    web::resource("/").route(web::to(|| async { HttpResponse::Ok().body(STR) })),
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
async fn connection_wait_queue_force_close() {
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
                App::new().service(web::resource("/").route(web::to(|| async {
                    HttpResponse::Ok().force_close().body(STR)
                }))),
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
async fn with_query_parameter() {
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

#[cfg(feature = "compress-gzip")]
#[actix_rt::test]
async fn no_decompress() {
    let srv = actix_test::start(|| {
        App::new()
            .wrap(actix_web::middleware::Compress::default())
            .service(web::resource("/").route(web::to(|| async { HttpResponse::Ok().body(STR) })))
    });

    let mut res = awc::Client::new()
        .get(srv.url("/"))
        .insert_header((header::ACCEPT_ENCODING, "gzip"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    // read response
    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), STR.as_bytes());

    // POST
    let mut res = awc::Client::new()
        .post(srv.url("/"))
        .insert_header((header::ACCEPT_ENCODING, "gzip"))
        .no_decompress()
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    assert_eq!(utils::gzip::decode(bytes), STR.as_bytes());
}

#[cfg(feature = "compress-gzip")]
#[actix_rt::test]
async fn client_gzip_encoding() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            HttpResponse::Ok()
                .insert_header(header::ContentEncoding::Gzip)
                .body(utils::gzip::encode(STR))
        })))
    });

    // client request
    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, STR);
}

#[cfg(feature = "compress-gzip")]
#[actix_rt::test]
async fn client_gzip_encoding_large() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|| async {
            HttpResponse::Ok()
                .insert_header(header::ContentEncoding::Gzip)
                .body(utils::gzip::encode(STR.repeat(10)))
        })))
    });

    // client request
    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, STR.repeat(10));
}

#[cfg(feature = "compress-gzip")]
#[actix_rt::test]
async fn client_gzip_encoding_large_random() {
    let data = Alphanumeric.sample_string(&mut rand::rng(), 100_000);

    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| async {
            HttpResponse::Ok()
                .insert_header(header::ContentEncoding::Gzip)
                .body(utils::gzip::encode(data))
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(data.clone()).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, data);
}

#[cfg(feature = "compress-brotli")]
#[actix_rt::test]
async fn client_brotli_encoding() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| async {
            HttpResponse::Ok()
                .insert_header(("content-encoding", "br"))
                .body(utils::brotli::encode(data))
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(STR).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[cfg(feature = "compress-brotli")]
#[actix_rt::test]
async fn client_brotli_encoding_large_random() {
    let data = Alphanumeric.sample_string(&mut rand::rng(), 70_000);

    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").route(web::to(|data: Bytes| async {
            HttpResponse::Ok()
                .insert_header(header::ContentEncoding::Brotli)
                .body(utils::brotli::encode(data))
        })))
    });

    // client request
    let mut response = srv.post("/").send_body(data.clone()).await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, data);
}

#[actix_rt::test]
async fn client_deflate_encoding() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: Bytes| async {
            HttpResponse::Ok().body(body)
        }))
    });

    let req = srv
        .post("/")
        .insert_header((header::ACCEPT_ENCODING, "gzip"))
        .send_body(STR);

    let mut res = req.await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, STR);
}

#[actix_rt::test]
async fn client_deflate_encoding_large_random() {
    let data = Alphanumeric.sample_string(&mut rand::rng(), 70_000);

    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: Bytes| async {
            HttpResponse::Ok().body(body)
        }))
    });

    let req = srv
        .post("/")
        .insert_header((header::ACCEPT_ENCODING, "br"))
        .send_body(data.clone());

    let mut res = req.await.unwrap();
    let bytes = res.body().await.unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(bytes, Bytes::from(data));
}

#[actix_rt::test]
async fn client_streaming_explicit() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|body: web::Payload| async {
            HttpResponse::Ok().streaming(body)
        }))
    });

    let body =
        stream::once(async { Ok::<_, actix_http::Error>(Bytes::from_static(STR.as_bytes())) });
    let req = srv
        .post("/")
        .insert_header((header::ACCEPT_ENCODING, "identity"))
        .send_stream(Box::pin(body));

    let mut res = req.await.unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn body_streaming_implicit() {
    let srv = actix_test::start(|| {
        App::new().default_service(web::to(|| async {
            let body =
                stream::once(async { Ok::<_, Infallible>(Bytes::from_static(STR.as_bytes())) });
            HttpResponse::Ok().streaming(body)
        }))
    });

    let req = srv
        .get("/")
        .insert_header((header::ACCEPT_ENCODING, "gzip"))
        .send();

    let mut res = req.await.unwrap();
    assert!(res.status().is_success());

    let bytes = res.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn client_cookie_handling() {
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
                    req.cookie("cookie1")
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
                        .map_err(|_| Error::from(IoError::from(ErrorKind::NotFound)))?;

                    // Send some cookies back
                    Ok::<_, Error>(HttpResponse::Ok().cookie(cookie1).cookie(cookie2).finish())
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
        let (mut stream, _) = lst.accept().unwrap();
        let mut b = [0; 1000];
        let _ = stream.read(&mut b).unwrap();
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\n\
                connection: close\r\n\
                \r\n\
                welcome!",
        );
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
                    == format!("Basic {}", BASE64_STANDARD.encode("username:password"))
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
async fn local_address() {
    let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

    let srv = actix_test::start(move || {
        App::new().service(
            web::resource("/").route(web::to(move |req: HttpRequest| async move {
                assert_eq!(req.peer_addr().unwrap().ip(), ip);
                Ok::<_, Error>(HttpResponse::Ok())
            })),
        )
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
