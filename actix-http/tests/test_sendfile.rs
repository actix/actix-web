#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    fs::File,
    io::{self, Read as _, Write as _},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use actix_http::{
    body::{BodySize, FileBody, FileRegion, MessageBody},
    HttpService, Request, Response, StatusCode,
};
use actix_http_test::test_server;
use bytes::Bytes;

// Any attempt to read file bytes in userspace fails the response.
struct DirectOnly(FileBody);

impl MessageBody for DirectOnly {
    type Error = io::Error;

    fn size(&self) -> BodySize {
        self.0.size()
    }

    fn file_region(self: Pin<&mut Self>) -> Option<FileRegion<'_>> {
        Pin::new(&mut self.get_mut().0).file_region()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, io::Error>>> {
        assert_eq!(
            self.as_mut().file_region().unwrap().remaining(),
            0,
            "file read fallback used"
        );
        Poll::Ready(None)
    }
}

struct Fixture(std::path::PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[actix_rt::test]
async fn direct_transfer_handles_backpressure_ranges_and_pipeline() {
    let bytes: Vec<_> = (0..8 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let path = std::env::temp_dir().join(format!("actix-sendfile-{}", std::process::id()));
    let fixture = Arc::new(Fixture(path));
    File::options()
        .write(true)
        .create_new(true)
        .open(&fixture.0)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let server_file = Arc::clone(&fixture);
    let server = test_server(move || {
        let file = Arc::clone(&server_file);
        HttpService::build()
            .h1(move |req: Request| {
                let body = match req.path() {
                    "/range" => FileBody::region(File::open(&file.0).unwrap(), 17, 100003).unwrap(),
                    "/empty" => FileBody::region(File::open(&file.0).unwrap(), 0, 0).unwrap(),
                    _ => FileBody::new(File::open(&file.0).unwrap()).unwrap(),
                };
                let status = match req.path() {
                    "/no-content" => StatusCode::NO_CONTENT,
                    "/not-modified" => StatusCode::NOT_MODIFIED,
                    _ => StatusCode::OK,
                };
                std::future::ready(Ok::<_, std::convert::Infallible>(
                    Response::build(status)
                        .body(DirectOnly(body))
                        .map_into_boxed_body(),
                ))
            })
            .tcp()
    })
    .await;
    let address = server.addr();

    actix_rt::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        socket.write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\nGET /no-content HTTP/1.1\r\nHost: localhost\r\n\r\nGET /not-modified HTTP/1.1\r\nHost: localhost\r\n\r\nGET / HTTP/1.1\r\nHost: localhost\r\n\r\nGET /range HTTP/1.1\r\nHost: localhost\r\n\r\nGET /empty HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        // Let the sender fill its socket buffer before the receiver starts reading.
        std::thread::sleep(Duration::from_millis(100));
        let mut received = Vec::new();
        socket.read_to_end(&mut received).unwrap();
        let mut rest = received.as_slice();

        for (status, expected) in [(200, &b""[..]), (204, &b""[..]), (304, &b""[..]), (200, bytes.as_slice()), (200, &bytes[17..100020]), (200, &b""[..])] {
            let header_end = rest.windows(4).position(|bytes| bytes == b"\r\n\r\n").unwrap() + 4;
            let headers = std::str::from_utf8(&rest[..header_end]).unwrap();
            assert!(headers.starts_with(&format!("HTTP/1.1 {status}")), "{headers}");
            rest = &rest[header_end..];
            assert!(rest.starts_with(expected));
            rest = &rest[expected.len()..];
        }

        assert!(rest.is_empty());
    }).await.unwrap();
}

#[actix_rt::test]
async fn truncated_file_closes_response_without_hanging() {
    let path =
        std::env::temp_dir().join(format!("actix-sendfile-truncated-{}", std::process::id()));
    let fixture = Arc::new(Fixture(path));
    File::options()
        .write(true)
        .create_new(true)
        .open(&fixture.0)
        .unwrap()
        .write_all(b"original content")
        .unwrap();
    let server_file = Arc::clone(&fixture);
    let server = test_server(move || {
        let file = Arc::clone(&server_file);
        HttpService::build()
            .h1(move |_: Request| {
                let body = FileBody::new(File::open(&file.0).unwrap()).unwrap();
                File::options()
                    .write(true)
                    .open(&file.0)
                    .unwrap()
                    .set_len(0)
                    .unwrap();
                std::future::ready(Ok::<_, std::convert::Infallible>(
                    Response::ok().set_body(DirectOnly(body)),
                ))
            })
            .tcp()
    })
    .await;

    let address = server.addr();
    actix_rt::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut received = String::new();
        socket.read_to_string(&mut received).unwrap();
        assert!(received.contains("content-length: 16\r\n"), "{received}");
        assert!(received.ends_with("\r\n\r\n"));
    })
    .await
    .unwrap();
}
