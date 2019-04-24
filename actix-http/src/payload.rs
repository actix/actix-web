use bytes::Bytes;
use futures::{Async, Poll, Stream};
use h2::RecvStream;

use crate::error::PayloadError;

/// Type represent boxed payload
pub type PayloadStream = Box<dyn Stream<Item = Bytes, Error = PayloadError>>;

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
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    #[inline]
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self {
            Payload::None => Ok(Async::Ready(None)),
            Payload::H1(ref mut pl) => pl.poll(),
            Payload::H2(ref mut pl) => pl.poll(),
            Payload::Stream(ref mut pl) => pl.poll(),
        }
    }
}
