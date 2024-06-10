use std::{
    convert::Infallible,
    io::{Read, Write},
    net, thread,
    time::{Duration, Instant},
};

use actix_http::{
    body::{self, BodyStream, BoxBody, SizedStream},
    header, Error, HttpService, KeepAlive, Request, Response, StatusCode, Version,
};
use actix_http_test::test_server;
use actix_rt::{net::TcpStream, time::sleep};
use actix_service::fn_service;
use actix_utils::future::{err, ok, ready};
use bytes::Bytes;
use derive_more::{Display, Error};
use futures_util::{stream::once, FutureExt as _, StreamExt as _};
use regex::Regex;

#[actix_rt::test]
async fn h1_basic() {
    let mut srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_request_timeout(Duration::from_secs(1))
            .client_disconnect_timeout(Duration::from_secs(1))
            .h1(|req: Request| {
                assert!(req.peer_addr().is_some());
                ok::<_, Infallible>(Response::ok())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_2() {
    let mut srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_request_timeout(Duration::from_secs(1))
            .client_disconnect_timeout(Duration::from_secs(1))
            .finish(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), http::Version::HTTP_11);
                ok::<_, Infallible>(Response::ok())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop().await;
}

#[derive(Debug, Display, Error)]
#[display(fmt = "expect failed")]
struct ExpectFailed;

impl From<ExpectFailed> for Response<BoxBody> {
    fn from(_: ExpectFailed) -> Self {
        Response::new(StatusCode::EXPECTATION_FAILED)
    }
}

#[actix_rt::test]
async fn expect_continue() {
    let mut srv = test_server(|| {
        HttpService::build()
            .expect(fn_service(|req: Request| {
                if req.head().uri.query() == Some("yes=") {
                    ok(req)
                } else {
                    err(ExpectFailed)
                }
            }))
            .finish(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 417 Expectation Failed\r\ncontent-length"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test?yes= HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n"));

    srv.stop().await;
}

#[actix_rt::test]
async fn expect_continue_h1() {
    let mut srv = test_server(|| {
        HttpService::build()
            .expect(fn_service(|req: Request| {
                sleep(Duration::from_millis(20)).then(move |_| {
                    if req.head().uri.query() == Some("yes=") {
                        ok(req)
                    } else {
                        err(ExpectFailed)
                    }
                })
            }))
            .h1(fn_service(|_| ok::<_, Infallible>(Response::ok())))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 417 Expectation Failed\r\ncontent-length"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test?yes= HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n"));

    srv.stop().await;
}

#[actix_rt::test]
async fn chunked_payload() {
    let chunk_sizes = [32768, 32, 32768];
    let total_size: usize = chunk_sizes.iter().sum();

    let mut srv = test_server(|| {
        HttpService::build()
            .h1(fn_service(|mut request: Request| {
                request
                    .take_payload()
                    .map(|res| match res {
                        Ok(pl) => pl,
                        Err(err) => panic!("Error reading payload: {err}"),
                    })
                    .fold(0usize, |acc, chunk| ready(acc + chunk.len()))
                    .map(|req_size| {
                        Ok::<_, Error>(Response::ok().set_body(format!("size={}", req_size)))
                    })
            }))
            .tcp()
    })
    .await;

    let returned_size = {
        let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
        let _ = stream.write_all(b"POST /test HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n");

        for chunk_size in chunk_sizes.iter() {
            let mut bytes = Vec::new();
            let random_bytes: Vec<u8> = (0..*chunk_size).map(|_| rand::random::<u8>()).collect();

            bytes.extend(format!("{:X}\r\n", chunk_size).as_bytes());
            bytes.extend(&random_bytes[..]);
            bytes.extend(b"\r\n");
            let _ = stream.write_all(&bytes);
        }

        let _ = stream.write_all(b"0\r\n\r\n");
        stream.shutdown(net::Shutdown::Write).unwrap();

        let mut data = String::new();
        let _ = stream.read_to_string(&mut data);

        let re = Regex::new(r"size=(\d+)").unwrap();
        let size: usize = match re.captures(&data) {
            Some(caps) => caps.get(1).unwrap().as_str().parse().unwrap(),
            None => panic!("Failed to find size in HTTP Response: {}", data),
        };

        size
    };

    assert_eq!(returned_size, total_size);

    srv.stop().await;
}

#[actix_rt::test]
async fn slow_request_408() {
    let mut srv = test_server(|| {
        HttpService::build()
            .client_request_timeout(Duration::from_millis(200))
            .keep_alive(Duration::from_secs(2))
            .finish(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let start = Instant::now();

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(
        data.starts_with("HTTP/1.1 408 Request Timeout"),
        "response was not 408: {}",
        data
    );

    let diff = start.elapsed();

    if diff < Duration::from_secs(1) {
        // test success
    } else if diff < Duration::from_secs(3) {
        panic!("request seems to have wrongly timed-out according to keep-alive");
    } else {
        panic!("request took way too long to time out");
    }

    srv.stop().await;
}

#[actix_rt::test]
async fn http1_malformed_request() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));

    srv.stop().await;
}

#[actix_rt::test]
async fn http1_keepalive() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    srv.stop().await;
}

#[actix_rt::test]
async fn http1_keepalive_timeout() {
    let mut srv = test_server(|| {
        HttpService::build()
            .keep_alive(Duration::from_secs(1))
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();

    let _ = stream.write_all(b"GET /test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 256];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    thread::sleep(Duration::from_millis(1100));

    let mut data = vec![0; 256];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);

    srv.stop().await;
}

#[actix_rt::test]
async fn http1_keepalive_close() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\nconnection: close\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);

    srv.stop().await;
}

#[actix_rt::test]
async fn http10_keepalive_default_close() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);

    srv.stop().await;
}

#[actix_rt::test]
async fn http10_keepalive() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\nconnection: keep-alive\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);

    srv.stop().await;
}

#[actix_rt::test]
async fn http1_keepalive_disabled() {
    let mut srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .h1(|_| ok::<_, Infallible>(Response::ok()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);

    srv.stop().await;
}

#[actix_rt::test]
async fn content_length() {
    use actix_http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|req: Request| {
                let idx: usize = req.uri().path()[1..].parse().unwrap();
                let statuses = [
                    StatusCode::NO_CONTENT,
                    StatusCode::CONTINUE,
                    StatusCode::SWITCHING_PROTOCOLS,
                    StatusCode::PROCESSING,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                ];
                ok::<_, Infallible>(Response::new(statuses[idx]))
            })
            .tcp()
    })
    .await;

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = srv.request(http::Method::GET, srv.url(&format!("/{}", i)));
            let response = req.send().await.unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = srv.request(http::Method::HEAD, srv.url(&format!("/{}", i)));
            let response = req.send().await.unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = srv.request(http::Method::GET, srv.url(&format!("/{}", i)));
            let response = req.send().await.unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test_server(move || {
        let data = data.clone();
        HttpService::build()
            .h1(move |_| {
                let mut builder = Response::build(StatusCode::OK);
                for idx in 0..90 {
                    builder.insert_header((
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
                ok::<_, Infallible>(builder.body(data.clone()))
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from(data2));

    srv.stop().await;
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
async fn h1_body() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_head_empty() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .tcp()
    })
    .await;

    let response = srv.head("/").send().await.unwrap();
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

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_head_binary() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .tcp()
    })
    .await;

    let response = srv.head("/").send().await.unwrap();
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

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_head_binary2() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, Infallible>(Response::ok().set_body(STR)))
            .tcp()
    })
    .await;

    let response = srv.head("/").send().await.unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_body_length() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok::<_, Infallible>(Bytes::from_static(STR.as_ref())));
                ok::<_, Infallible>(
                    Response::ok().set_body(SizedStream::new(STR.len() as u64, body)),
                )
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_body_chunked_explicit() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, Infallible>(
                    Response::build(StatusCode::OK)
                        .insert_header((header::TRANSFER_ENCODING, "chunked"))
                        .body(BodyStream::new(body)),
                )
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get(header::TRANSFER_ENCODING)
            .unwrap()
            .to_str()
            .unwrap(),
        "chunked"
    );

    // read response
    let bytes = srv.load_body(response).await.unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_body_chunked_implicit() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, Infallible>(Response::build(StatusCode::OK).body(BodyStream::new(body)))
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get(header::TRANSFER_ENCODING)
            .unwrap()
            .to_str()
            .unwrap(),
        "chunked"
    );

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_response_http_error_handling() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(fn_service(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, Infallible>(
                    Response::build(StatusCode::OK)
                        .insert_header((http::header::CONTENT_TYPE, broken_header))
                        .body(STR),
                )
            }))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(
        bytes,
        Bytes::from_static(b"error processing HTTP: failed to parse header value")
    );

    srv.stop().await;
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
async fn h1_service_error() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| err::<Response<()>, _>(BadRequest))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));

    srv.stop().await;
}

#[actix_rt::test]
async fn h1_on_connect() {
    let mut srv = test_server(|| {
        HttpService::build()
            .on_connect_ext(|_, data| {
                data.insert(20isize);
            })
            .h1(|req: Request| {
                assert!(req.conn_data::<isize>().is_some());
                ok::<_, Infallible>(Response::ok())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    srv.stop().await;
}

/// Tests compliance with 304 Not Modified spec in RFC 7232 §4.1.
/// https://datatracker.ietf.org/doc/html/rfc7232#section-4.1
#[actix_rt::test]
async fn not_modified_spec_h1() {
    // TODO: this test needing a few seconds to complete reveals some weirdness with either the
    // dispatcher or the client, though similar hangs occur on other tests in this file, only
    // succeeding, it seems, because of the keepalive timer

    static CL: header::HeaderName = header::CONTENT_LENGTH;

    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|req: Request| {
                let res: Response<BoxBody> = match req.path() {
                    // with no content-length
                    "/none" => Response::with_body(StatusCode::NOT_MODIFIED, body::None::new())
                        .map_into_boxed_body(),

                    // with no content-length
                    "/body" => {
                        Response::with_body(StatusCode::NOT_MODIFIED, "1234").map_into_boxed_body()
                    }

                    // with manual content-length header and specific None body
                    "/cl-none" => {
                        let mut res =
                            Response::with_body(StatusCode::NOT_MODIFIED, body::None::new());
                        res.headers_mut()
                            .insert(CL.clone(), header::HeaderValue::from_static("24"));
                        res.map_into_boxed_body()
                    }

                    // with manual content-length header and ignore-able body
                    "/cl-body" => {
                        let mut res = Response::with_body(StatusCode::NOT_MODIFIED, "1234");
                        res.headers_mut()
                            .insert(CL.clone(), header::HeaderValue::from_static("4"));
                        res.map_into_boxed_body()
                    }

                    _ => panic!("unknown route"),
                };

                ok::<_, Infallible>(res)
            })
            .tcp()
    })
    .await;

    let res = srv.get("/none").send().await.unwrap();
    assert_eq!(res.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(res.headers().get(&CL), None);
    assert!(srv.load_body(res).await.unwrap().is_empty());

    let res = srv.get("/body").send().await.unwrap();
    assert_eq!(res.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(res.headers().get(&CL), None);
    assert!(srv.load_body(res).await.unwrap().is_empty());

    let res = srv.get("/cl-none").send().await.unwrap();
    assert_eq!(res.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(
        res.headers().get(&CL),
        Some(&header::HeaderValue::from_static("24")),
    );
    assert!(srv.load_body(res).await.unwrap().is_empty());

    let res = srv.get("/cl-body").send().await.unwrap();
    assert_eq!(res.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(
        res.headers().get(&CL),
        Some(&header::HeaderValue::from_static("4")),
    );
    // server does not prevent payload from being sent but clients may choose not to read it
    // TODO: this is probably a bug in the client, especially since CL header can differ in length
    // from the body
    assert!(!srv.load_body(res).await.unwrap().is_empty());

    // TODO: add stream response tests

    srv.stop().await;
}

#[actix_rt::test]
async fn h2c_auto() {
    let mut srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .finish(|req: Request| {
                let body = match req.version() {
                    Version::HTTP_11 => "h1",
                    Version::HTTP_2 => "h2",
                    _ => unreachable!(),
                };
                ok::<_, Infallible>(Response::ok().set_body(body))
            })
            .tcp_auto_h2c()
    })
    .await;

    let req = srv.get("/");
    assert_eq!(req.get_version(), &Version::HTTP_11);
    let mut res = req.send().await.unwrap();
    assert!(res.status().is_success());
    assert_eq!(res.body().await.unwrap(), &b"h1"[..]);

    // awc doesn't support forcing the version to http/2 so use h2 manually

    let tcp = TcpStream::connect(srv.addr()).await.unwrap();
    let (h2, connection) = h2::client::handshake(tcp).await.unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    let mut h2 = h2.ready().await.unwrap();

    let request = ::http::Request::new(());
    let (response, _) = h2.send_request(request, true).unwrap();
    let (head, mut body) = response.await.unwrap().into_parts();
    let body = body.data().await.unwrap().unwrap();

    assert!(head.status.is_success());
    assert_eq!(body, &b"h2"[..]);

    srv.stop().await;
}
