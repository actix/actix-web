use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use h2::RecvStream;

use crate::error::PayloadError;

/// Type represent boxed payload
pub type PayloadStream = Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>>;

/// Type represent streaming payload
pub enum Payload<S = PayloadStream> {
    None,
    H1(crate::h1::Payload),
    H2(crate::h2::Payload),
    Stream(S),
}

impl<S> From<crate::h1::Payload> for Payload<S> {
    fn from(v: crate::h1::Payload) -> Self {
        Payload::H1(v)
    }
}

impl<S> From<crate::h2::Payload> for Payload<S> {
    fn from(v: crate::h2::Payload) -> Self {
        Payload::H2(v)
    }
}

impl<S> From<RecvStream> for Payload<S> {
    fn from(v: RecvStream) -> Self {
        Payload::H2(crate::h2::Payload::new(v))
    }
}

impl From<PayloadStream> for Payload {
    fn from(pl: PayloadStream) -> Self {
        Payload::Stream(pl)
    }
}

impl<S> Payload<S> {
    /// Takes current payload and replaces it with `None` value
    pub fn take(&mut self) -> Payload<S> {
        std::mem::replace(self, Payload::None)
    }
}

impl<S> Stream for Payload<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<Bytes, PayloadError>;

    #[inline]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Payload::None => Poll::Ready(None),
            Payload::H1(ref mut pl) => pl.readany(cx),
            Payload::H2(ref mut pl) => Pin::new(pl).poll_next(cx),
            Payload::Stream(ref mut pl) => Pin::new(pl).poll_next(cx),
        }
    }
}
