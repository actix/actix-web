use bytes::Bytes;
use derive_more::From;
use futures::{Poll, Stream};
use h2::RecvStream;

use crate::error::PayloadError;

#[derive(From)]
pub enum Payload {
    H1(crate::h1::Payload),
    H2(crate::h2::Payload),
    Dyn(Box<Stream<Item = Bytes, Error = PayloadError>>),
}

impl From<RecvStream> for Payload {
    fn from(v: RecvStream) -> Self {
        Payload::H2(crate::h2::Payload::new(v))
    }
}

impl Stream for Payload {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self {
            Payload::H1(ref mut pl) => pl.poll(),
            Payload::H2(ref mut pl) => pl.poll(),
            Payload::Dyn(ref mut pl) => pl.poll(),
        }
    }
}
