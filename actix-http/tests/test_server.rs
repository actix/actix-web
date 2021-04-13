use std::io::{Read, Write};
use std::time::Duration;
use std::{net, thread};

use actix_http_test::test_server;
use actix_rt::time::sleep;
use actix_service::fn_service;
use actix_utils::future::{err, ok, ready};
use bytes::Bytes;
use futures_util::stream::{once, StreamExt as _};
use futures_util::FutureExt as _;
use regex::Regex;

use actix_http::HttpMessage;
use actix_http::{
    body::{Body, SizedStream},
    error, http,
    http::header,
    Error, HttpService, KeepAlive, Request, Response,
};

#[actix_rt::test]
async fn test_h1() {
    let srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .h1(|req: Request| {
                assert!(req.peer_addr().is_some());
                ok::<_, ()>(Response::Ok().finish())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_h1_2() {
    let srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .finish(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), http::Version::HTTP_11);
                ok::<_, ()>(Response::Ok().finish())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_expect_continue() {
    let srv = test_server(|| {
        HttpService::build()
            .expect(fn_service(|req: Request| {
                if req.head().uri.query() == Some("yes=") {
                    ok(req)
                } else {
                    err(error::ErrorPreconditionFailed("error"))
                }
            }))
            .finish(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 412 Precondition Failed\r\ncontent-length"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test?yes= HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n"));
}

#[actix_rt::test]
async fn test_expect_continue_h1() {
    let srv = test_server(|| {
        HttpService::build()
            .expect(fn_service(|req: Request| {
                sleep(Duration::from_millis(20)).then(move |_| {
                    if req.head().uri.query() == Some("yes=") {
                        ok(req)
                    } else {
                        err(error::ErrorPreconditionFailed("error"))
                    }
                })
            }))
            .h1(fn_service(|_| ok::<_, ()>(Response::Ok().finish())))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 412 Precondition Failed\r\ncontent-length"));

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test?yes= HTTP/1.1\r\nexpect: 100-continue\r\n\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n"));
}

#[actix_rt::test]
async fn test_chunked_payload() {
    let chunk_sizes = vec![32768, 32, 32768];
    let total_size: usize = chunk_sizes.iter().sum();

    let srv = test_server(|| {
        HttpService::build()
            .h1(fn_service(|mut request: Request| {
                request
                    .take_payload()
                    .map(|res| match res {
                        Ok(pl) => pl,
                        Err(e) => panic!("Error reading payload: {}", e),
                    })
                    .fold(0usize, |acc, chunk| ready(acc + chunk.len()))
                    .map(|req_size| {
                        Ok::<_, Error>(Response::Ok().body(format!("size={}", req_size)))
                    })
            }))
            .tcp()
    })
    .await;

    let returned_size = {
        let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
        let _ = stream
            .write_all(b"POST /test HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n");

        for chunk_size in chunk_sizes.iter() {
            let mut bytes = Vec::new();
            let random_bytes: Vec<u8> =
                (0..*chunk_size).map(|_| rand::random::<u8>()).collect();

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
}

#[actix_rt::test]
async fn test_slow_request() {
    let srv = test_server(|| {
        HttpService::build()
            .client_timeout(100)
            .finish(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[actix_rt::test]
async fn test_http1_malformed_request() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));
}

#[actix_rt::test]
async fn test_http1_keepalive() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
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
}

#[actix_rt::test]
async fn test_http1_keepalive_timeout() {
    let srv = test_server(|| {
        HttpService::build()
            .keep_alive(1)
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
    thread::sleep(Duration::from_millis(1100));

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[actix_rt::test]
async fn test_http1_keepalive_close() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ =
        stream.write_all(b"GET /test/tests/test HTTP/1.1\r\nconnection: close\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[actix_rt::test]
async fn test_http10_keepalive_default_close() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
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
}

#[actix_rt::test]
async fn test_http10_keepalive() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
            .tcp()
    })
    .await;

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream
        .write_all(b"GET /test/tests/test HTTP/1.0\r\nconnection: keep-alive\r\n\r\n");
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
}

#[actix_rt::test]
async fn test_http1_keepalive_disabled() {
    let srv = test_server(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .h1(|_| ok::<_, ()>(Response::Ok().finish()))
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
}

#[actix_rt::test]
async fn test_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let srv = test_server(|| {
        HttpService::build()
            .h1(|req: Request| {
                let indx: usize = req.uri().path()[1..].parse().unwrap();
                let statuses = [
                    StatusCode::NO_CONTENT,
                    StatusCode::CONTINUE,
                    StatusCode::SWITCHING_PROTOCOLS,
                    StatusCode::PROCESSING,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                ];
                ok::<_, ()>(Response::new(statuses[indx]))
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
}

#[actix_rt::test]
async fn test_h1_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = test_server(move || {
        let data = data.clone();
        HttpService::build().h1(move |_| {
            let mut builder = Response::Ok();
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
            ok::<_, ()>(builder.body(data.clone()))
        }).tcp()
    }).await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from(data2));
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
async fn test_h1_body() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_h1_head_empty() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
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
}

#[actix_rt::test]
async fn test_h1_head_binary() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
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
}

#[actix_rt::test]
async fn test_h1_head_binary2() {
    let srv = test_server(|| {
        HttpService::build()
            .h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
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
}

#[actix_rt::test]
async fn test_h1_body_length() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok(Bytes::from_static(STR.as_ref())));
                ok::<_, ()>(
                    Response::Ok().body(SizedStream::new(STR.len() as u64, body)),
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
}

#[actix_rt::test]
async fn test_h1_body_chunked_explicit() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, ()>(
                    Response::Ok()
                        .insert_header((header::TRANSFER_ENCODING, "chunked"))
                        .streaming(body),
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
}

#[actix_rt::test]
async fn test_h1_body_chunked_implicit() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| {
                let body = once(ok::<_, Error>(Bytes::from_static(STR.as_ref())));
                ok::<_, ()>(Response::Ok().streaming(body))
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
}

#[actix_rt::test]
async fn test_h1_response_http_error_handling() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(fn_service(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, ()>(
                    Response::Ok()
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
    assert_eq!(bytes, Bytes::from_static(b"failed to parse header value"));
}

#[actix_rt::test]
async fn test_h1_service_error() {
    let mut srv = test_server(|| {
        HttpService::build()
            .h1(|_| err::<Response<Body>, _>(error::ErrorBadRequest("error")))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).await.unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

#[actix_rt::test]
async fn test_h1_on_connect() {
    let srv = test_server(|| {
        HttpService::build()
            .on_connect_ext(|_, data| {
                data.insert(20isize);
            })
            .h1(|req: Request| {
                assert!(req.extensions().contains::<isize>());
                ok::<_, ()>(Response::Ok().finish())
            })
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());
}
