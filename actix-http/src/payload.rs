use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;
use pin_project_lite::pin_project;

use crate::error::PayloadError;

/// A boxed payload stream.
pub type BoxedPayloadStream = Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>>;

#[doc(hidden)]
#[deprecated(since = "3.0.0", note = "Renamed to `BoxedPayloadStream`.")]
pub type PayloadStream = BoxedPayloadStream;

#[cfg(not(feature = "http2"))]
pin_project! {
    /// A streaming payload.
    #[project = PayloadProj]
    pub enum Payload<S = BoxedPayloadStream> {
        None,
        H1 { payload: crate::h1::Payload },
        Stream { #[pin] payload: S },
    }
}

#[cfg(feature = "http2")]
pin_project! {
    /// A streaming payload.
    #[project = PayloadProj]
    pub enum Payload<S = BoxedPayloadStream> {
        None,
        H1 { payload: crate::h1::Payload },
        H2 { payload: crate::h2::Payload },
        Stream { #[pin] payload: S },
    }
}

impl<S> From<crate::h1::Payload> for Payload<S> {
    fn from(payload: crate::h1::Payload) -> Self {
        Payload::H1 { payload }
    }
}

#[cfg(feature = "http2")]
impl<S> From<crate::h2::Payload> for Payload<S> {
    fn from(payload: crate::h2::Payload) -> Self {
        Payload::H2 { payload }
    }
}

#[cfg(feature = "http2")]
impl<S> From<::h2::RecvStream> for Payload<S> {
    fn from(stream: ::h2::RecvStream) -> Self {
        Payload::H2 {
            payload: crate::h2::Payload::new(stream),
        }
    }
}

impl From<BoxedPayloadStream> for Payload {
    fn from(payload: BoxedPayloadStream) -> Self {
        Payload::Stream { payload }
    }
}

impl<S> Payload<S> {
    /// Takes current payload and replaces it with `None` value
    pub fn take(&mut self) -> Payload<S> {
        mem::replace(self, Payload::None)
    }
}

impl<S> Stream for Payload<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    type Item = Result<Bytes, PayloadError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.project() {
            PayloadProj::None => Poll::Ready(None),
            PayloadProj::H1 { payload } => Pin::new(payload).poll_next(cx),

            #[cfg(feature = "http2")]
            PayloadProj::H2 { payload } => Pin::new(payload).poll_next(cx),

            PayloadProj::Stream { payload } => payload.poll_next(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;

    assert_impl_all!(Payload: Unpin);
    assert_not_impl_any!(Payload: Send, Sync);
}
