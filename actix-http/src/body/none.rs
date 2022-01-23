use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody};

/// Body type for responses that forbid payloads.
///
/// This is distinct from an "empty" response which _would_ contain a `Content-Length` header.
/// For an "empty" body, use `()` or `Bytes::new()`.
///
/// For example, the HTTP spec forbids a payload to be sent with a `204 No Content` response.
/// In this case, the payload (or lack thereof) is implicit from the status code, so a
/// `Content-Length` header is not required.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct None;

impl None {
    /// Constructs new "none" body.
    #[inline]
    pub fn new() -> Self {
        None
    }
}

impl MessageBody for None {
    type Error = Infallible;

    #[inline]
    fn size(&self) -> BodySize {
        BodySize::None
    }

    #[inline]
    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        Poll::Ready(Option::None)
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self> {
        Ok(Bytes::new())
    }
}
