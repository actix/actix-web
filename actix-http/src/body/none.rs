use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody};

/// Body type for responses that forbid payloads.
///
/// Distinct from an empty response which would contain Content-Length header.
///
/// For an "empty" body, use `()` or `Bytes::new()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct None(());

impl None {
    /// Constructs new "none" body.
    pub fn new() -> Self {
        None(())
    }
}

impl MessageBody for None {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::None
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        Poll::Ready(Option::None)
    }
}
