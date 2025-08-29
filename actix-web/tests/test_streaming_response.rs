use std::{
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{
    http::header::{self, HeaderValue},
    HttpResponse,
};
use bytes::Bytes;
use futures_core::Stream;

struct FixedSizeStream {
    data: Vec<u8>,
    yielded: bool,
}

impl FixedSizeStream {
    fn new(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
            yielded: false,
        }
    }
}

impl Stream for FixedSizeStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.yielded {
            Poll::Ready(None)
        } else {
            self.yielded = true;
            let data = std::mem::take(&mut self.data);
            Poll::Ready(Some(Ok(Bytes::from(data))))
        }
    }
}

#[actix_rt::test]
async fn test_streaming_response_with_content_length() {
    let stream = FixedSizeStream::new(100);

    let resp = HttpResponse::Ok()
        .append_header((header::CONTENT_LENGTH, "100"))
        .streaming(stream);

    assert_eq!(
        resp.headers().get(header::CONTENT_LENGTH),
        Some(&HeaderValue::from_static("100")),
        "Content-Length should be preserved when explicitly set"
    );

    let has_chunked = resp
        .headers()
        .get(header::TRANSFER_ENCODING)
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("")
        .contains("chunked");

    assert!(
        !has_chunked,
        "chunked should not be used when Content-Length is provided"
    );

    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/octet-stream")),
        "Content-Type should default to application/octet-stream"
    );
}

#[actix_rt::test]
async fn test_streaming_response_default_content_type() {
    let stream = FixedSizeStream::new(50);

    let resp = HttpResponse::Ok().streaming(stream);

    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/octet-stream")),
        "Content-Type should default to application/octet-stream"
    );
}

#[actix_rt::test]
async fn test_streaming_response_user_defined_content_type() {
    let stream = FixedSizeStream::new(25);

    let resp = HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/plain"))
        .streaming(stream);

    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain")),
        "User-defined Content-Type should be preserved"
    );
}

#[actix_rt::test]
async fn test_streaming_response_empty_stream() {
    let stream = FixedSizeStream::new(0);

    let resp = HttpResponse::Ok()
        .append_header((header::CONTENT_LENGTH, "0"))
        .streaming(stream);

    assert_eq!(
        resp.headers().get(header::CONTENT_LENGTH),
        Some(&HeaderValue::from_static("0")),
        "Content-Length 0 should be preserved for empty streams"
    );
}
