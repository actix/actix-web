use std::task::Poll;

use actix_rt::pin;
use actix_utils::future::poll_fn;
use bytes::{Bytes, BytesMut};
use futures_core::ready;

use super::{BodySize, MessageBody};

/// Collects the body produced by a `MessageBody` implementation into `Bytes`.
///
/// Any errors produced by the body stream are returned immediately.
///
/// # Examples
/// ```
/// use actix_http::body::{self, to_bytes};
/// use bytes::Bytes;
///
/// # async fn test_to_bytes() {
/// let body = body::None::new();
/// let bytes = to_bytes(body).await.unwrap();
/// assert!(bytes.is_empty());
///
/// let body = Bytes::from_static(b"123");
/// let bytes = to_bytes(body).await.unwrap();
/// assert_eq!(bytes, b"123"[..]);
/// # }
/// ```
pub async fn to_bytes<B: MessageBody>(body: B) -> Result<Bytes, B::Error> {
    let cap = match body.size() {
        BodySize::None | BodySize::Sized(0) => return Ok(Bytes::new()),
        BodySize::Sized(size) => size as usize,
        // good enough first guess for chunk size
        BodySize::Stream => 32_768,
    };

    let mut buf = BytesMut::with_capacity(cap);

    pin!(body);

    poll_fn(|cx| loop {
        let body = body.as_mut();

        match ready!(body.poll_next(cx)) {
            Some(Ok(bytes)) => buf.extend_from_slice(&bytes),
            None => return Poll::Ready(Ok(())),
            Some(Err(err)) => return Poll::Ready(Err(err)),
        }
    })
    .await?;

    Ok(buf.freeze())
}

#[cfg(test)]
mod test {
    use futures_util::{stream, StreamExt as _};

    use super::*;
    use crate::{body::BodyStream, Error};

    #[actix_rt::test]
    async fn test_to_bytes() {
        let bytes = to_bytes(()).await.unwrap();
        assert!(bytes.is_empty());

        let body = Bytes::from_static(b"123");
        let bytes = to_bytes(body).await.unwrap();
        assert_eq!(bytes, b"123"[..]);

        let stream = stream::iter(vec![Bytes::from_static(b"123"), Bytes::from_static(b"abc")])
            .map(Ok::<_, Error>);
        let body = BodyStream::new(stream);
        let bytes = to_bytes(body).await.unwrap();
        assert_eq!(bytes, b"123abc"[..]);
    }
}
