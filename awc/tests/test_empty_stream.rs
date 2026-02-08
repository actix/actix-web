use std::{convert::Infallible, time::Duration};

use actix_rt::net::TcpListener;
use awc::Client;
use bytes::Bytes;
use futures_util::stream;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    time::timeout,
};

#[actix_rt::test]
async fn empty_body_stream_does_not_use_chunked_encoding() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Minimal HTTP/1.1 server that rejects chunked requests.
    let srv = actix_rt::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();

        let mut buf = Vec::with_capacity(1024);
        let mut tmp = [0u8; 1024];

        let header_end = loop {
            let n = timeout(Duration::from_secs(2), sock.read(&mut tmp))
                .await
                .unwrap()
                .unwrap();
            if n == 0 {
                break None;
            }

            buf.extend_from_slice(&tmp[..n]);

            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break Some(pos + 4);
            }

            if buf.len() > 16 * 1024 {
                break None;
            }
        }
        .expect("did not receive complete request headers");

        let headers_lower = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let has_chunked = headers_lower.contains("\r\ntransfer-encoding: chunked\r\n");

        if has_chunked {
            // Drain terminating chunk so client doesn't error on write before response is read.
            let terminator = b"0\r\n\r\n";
            while !buf[header_end..]
                .windows(terminator.len())
                .any(|w| w == terminator)
            {
                let n = match timeout(Duration::from_secs(2), sock.read(&mut tmp)).await {
                    Ok(Ok(n)) => n,
                    _ => break,
                };

                if n == 0 {
                    break;
                }

                buf.extend_from_slice(&tmp[..n]);

                if buf.len() > 32 * 1024 {
                    break;
                }
            }
        }

        let status = if has_chunked {
            "400 Bad Request"
        } else {
            "200 OK"
        };
        let resp = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        sock.write_all(resp.as_bytes()).await.unwrap();
    });

    let url = format!("http://{addr}/");
    let res = Client::default()
        .get(url)
        .send_stream(stream::empty::<Result<Bytes, Infallible>>())
        .await
        .unwrap();

    assert!(res.status().is_success());

    srv.await.unwrap();
}
