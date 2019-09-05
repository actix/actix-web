use std::io::{Read, Write};
use std::time::Duration;
use std::{net, thread};

use actix_http_test::TestServer;
use actix_server_config::ServerConfig;
use actix_service::{new_service_cfg, service_fn, NewService};
use bytes::Bytes;
use futures::future::{self, ok, Future};
use futures::stream::{once, Stream};
use regex::Regex;
use tokio_timer::sleep;

use actix_http::httpmessage::HttpMessage;
use actix_http::{
    body, error, http, http::header, Error, HttpService, KeepAlive, Request, Response,
};

#[test]
fn test_h1() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .h1(|req: Request| {
                assert!(req.peer_addr().is_some());
                future::ok::<_, ()>(Response::Ok().finish())
            })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_h1_2() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .finish(|req: Request| {
                assert!(req.peer_addr().is_some());
                assert_eq!(req.version(), http::Version::HTTP_11);
                future::ok::<_, ()>(Response::Ok().finish())
            })
            .map(|_| ())
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_expect_continue() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .expect(service_fn(|req: Request| {
                if req.head().uri.query() == Some("yes=") {
                    Ok(req)
                } else {
                    Err(error::ErrorPreconditionFailed("error"))
                }
            }))
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_expect_continue_h1() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .expect(service_fn(|req: Request| {
                sleep(Duration::from_millis(20)).then(move |_| {
                    if req.head().uri.query() == Some("yes=") {
                        Ok(req)
                    } else {
                        Err(error::ErrorPreconditionFailed("error"))
                    }
                })
            }))
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_chunked_payload() {
    let chunk_sizes = vec![32768, 32, 32768];
    let total_size: usize = chunk_sizes.iter().sum();

    let srv = TestServer::new(|| {
        HttpService::build().h1(|mut request: Request| {
            request
                .take_payload()
                .map_err(|e| panic!(format!("Error reading payload: {}", e)))
                .fold(0usize, |acc, chunk| future::ok::<_, ()>(acc + chunk.len()))
                .map(|req_size| Response::Ok().body(format!("size={}", req_size)))
        })
    });

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
            None => panic!(format!("Failed to find size in HTTP Response: {}", data)),
        };
        size
    };

    assert_eq!(returned_size, total_size);
}

#[test]
fn test_slow_request() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .client_timeout(100)
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[test]
fn test_http1_malformed_request() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));
}

#[test]
fn test_http1_keepalive() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_http1_keepalive_timeout() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(1)
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_http1_keepalive_close() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_http10_keepalive_default_close() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_http10_keepalive() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

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

#[test]
fn test_http1_keepalive_disabled() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|req: Request| {
            let indx: usize = req.uri().path()[1..].parse().unwrap();
            let statuses = [
                StatusCode::NO_CONTENT,
                StatusCode::CONTINUE,
                StatusCode::SWITCHING_PROTOCOLS,
                StatusCode::PROCESSING,
                StatusCode::OK,
                StatusCode::NOT_FOUND,
            ];
            future::ok::<_, ()>(Response::new(statuses[indx]))
        })
    });

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = srv
                .request(http::Method::GET, srv.url(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = srv
                .request(http::Method::HEAD, srv.url(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = srv
                .request(http::Method::GET, srv.url(&format!("/{}", i)))
                .send();
            let response = srv.block_on(req).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_h1_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = TestServer::new(move || {
        let data = data.clone();
        HttpService::build().h1(move |_| {
            let mut builder = Response::Ok();
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
            future::ok::<_, ()>(builder.body(data.clone()))
        })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
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

#[test]
fn test_h1_body() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_head_empty() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
    });

    let response = srv.block_on(srv.head("/").send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h1_head_binary() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            ok::<_, ()>(Response::Ok().content_length(STR.len() as u64).body(STR))
        })
    });

    let response = srv.block_on(srv.head("/").send()).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h1_head_binary2() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
    });

    let response = srv.block_on(srv.head("/").send()).unwrap();
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
fn test_h1_body_length() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(
                Response::Ok().body(body::SizedStream::new(STR.len() as u64, body)),
            )
        })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_body_chunked_explicit() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(
                Response::Ok()
                    .header(header::TRANSFER_ENCODING, "chunked")
                    .streaming(body),
            )
        })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
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
    let bytes = srv.load_body(response).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_body_chunked_implicit() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(Response::Ok().streaming(body))
        })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
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
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_response_http_error_handling() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(new_service_cfg(|_: &ServerConfig| {
            Ok::<_, ()>(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, ()>(
                    Response::Ok()
                        .header(http::header::CONTENT_TYPE, broken_header)
                        .body(STR),
                )
            })
        }))
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"failed to parse header value"));
}

#[test]
fn test_h1_service_error() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .h1(|_| Err::<Response, Error>(error::ErrorBadRequest("error")))
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"error"));
}

#[test]
fn test_h1_on_connect() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .on_connect(|_| 10usize)
            .h1(|req: Request| {
                assert!(req.extensions().contains::<usize>());
                future::ok::<_, ()>(Response::Ok().finish())
            })
    });

    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());
}
